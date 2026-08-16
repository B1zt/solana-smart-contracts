import {PublicKey} from '@solana/web3.js';
import type {FastifyInstance, FastifyRequest} from 'fastify';
import {z} from 'zod';
import {compressionSavings, DasClient, DasUnsupportedError} from './chain/das.js';
import {config} from './config.js';
import {prisma} from './lib/prisma.js';
import {Allowlist, type AllowlistEntry} from './merkle/allowlist.js';

const das = new DasClient(config.dasUrl);

const pubkeyString = z.string().refine((value) => {
  try {
    new PublicKey(value);
    return true;
  } catch {
    return false;
  }
}, 'must be a base58 Solana address');

function requireAdmin(request: FastifyRequest): boolean {
  if (!config.ADMIN_API_KEY) return false;
  return request.headers.authorization === `Bearer ${config.ADMIN_API_KEY}`;
}

export async function apiRoutes(app: FastifyInstance): Promise<void> {
  app.get('/config', async (_request, reply) => {
    const dasSupported = await das.isSupported().catch(() => false);

    return reply.send({
      cluster: config.CLUSTER,
      programId: config.programId.toBase58(),
      // Surfaced so the UI can explain why compressed NFTs are missing rather than silently
      // showing an incomplete wallet.
      dasSupported,
    });
  });

  /*//////////////////////////////////////////////////////////////
                              COLLECTIONS
  //////////////////////////////////////////////////////////////*/

  app.get('/collections', async (_request, reply) => {
    const collections = await prisma.collection.findMany({
      include: {phases: {orderBy: {index: 'asc'}}},
      orderBy: {updatedAt: 'desc'},
    });

    return reply.send({collections});
  });

  app.get('/collections/:address', async (request, reply) => {
    const {address} = request.params as {address: string};

    const collection = await prisma.collection.findUnique({
      where: {address},
      include: {phases: {orderBy: {index: 'asc'}}},
    });

    if (!collection) {
      return reply.status(404).send({error: 'NOT_FOUND'});
    }

    const [listedCount, salesCount] = await Promise.all([
      prisma.listing.count({where: {collectionAddress: address, isActive: true}}),
      prisma.sale.count({where: {collectionAddress: address}}),
    ]);

    return reply.send({collection, listedCount, salesCount});
  });

  /** Live listings for a collection, cheapest first. */
  app.get('/collections/:address/listings', async (request, reply) => {
    const {address} = request.params as {address: string};

    const schema = z.object({
      limit: z.coerce.number().int().min(1).max(100).default(50),
      cursor: z.string().optional(),
    });

    const parsed = schema.safeParse(request.query);
    if (!parsed.success) {
      return reply.status(400).send({error: 'INVALID_QUERY', issues: parsed.error.issues});
    }

    const {limit, cursor} = parsed.data;

    const listings = await prisma.listing.findMany({
      where: {collectionAddress: address, isActive: true},
      orderBy: [{price: 'asc'}, {address: 'asc'}],
      take: limit + 1,
      ...(cursor ? {cursor: {address: cursor}, skip: 1} : {}),
    });

    const hasMore = listings.length > limit;
    const page = hasMore ? listings.slice(0, limit) : listings;

    // Metadata comes from DAS on demand rather than being duplicated here. A compressed NFT has no
    // account to read, so DAS is the source of truth for what an asset actually is.
    const withMetadata = await Promise.all(
      page.map(async (listing) => {
        const asset = await das.getAsset(listing.mint).catch(() => null);

        return {
          ...listing,
          name: asset?.content?.metadata?.name ?? null,
          image: asset?.content?.files?.[0]?.uri ?? null,
          compressed: asset?.compression?.compressed ?? false,
        };
      }),
    );

    return reply.send({
      listings: withMetadata,
      nextCursor: hasMore ? page[page.length - 1]?.address : null,
    });
  });

  app.get('/collections/:address/activity', async (request, reply) => {
    const {address} = request.params as {address: string};

    const sales = await prisma.sale.findMany({
      where: {collectionAddress: address},
      orderBy: {blockTime: 'desc'},
      take: 50,
    });

    return reply.send({sales});
  });

  /*//////////////////////////////////////////////////////////////
                               ALLOWLIST
  //////////////////////////////////////////////////////////////*/

  /** Upload an allowlist and get the root to publish on-chain. Privileged. */
  app.post('/phases/:address/allowlist', async (request, reply) => {
    if (!requireAdmin(request)) {
      return reply.status(401).send({error: 'UNAUTHORIZED'});
    }

    const {address} = request.params as {address: string};

    const schema = z.object({
      entries: z
        .array(z.object({wallet: pubkeyString, allowance: z.number().int().min(1).max(65_535)}))
        .min(1)
        .max(50_000),
    });

    const parsed = schema.safeParse(request.body);
    if (!parsed.success) {
      return reply.status(400).send({error: 'INVALID_BODY', issues: parsed.error.issues});
    }

    const entries: AllowlistEntry[] = parsed.data.entries.map((entry) => ({
      wallet: new PublicKey(entry.wallet),
      allowance: entry.allowance,
    }));

    let tree: Allowlist;
    try {
      tree = new Allowlist(entries);
    } catch (error) {
      return reply.status(400).send({error: 'INVALID_ENTRIES', message: (error as Error).message});
    }

    await prisma.$transaction(async (tx) => {
      // Wholesale replacement. Merging would leave entries that no longer match the published root.
      await tx.allowlistEntry.deleteMany({where: {phaseAddress: address}});
      await tx.allowlistEntry.createMany({
        data: entries.map((entry) => ({
          phaseAddress: address,
          wallet: entry.wallet.toBase58(),
          allowance: entry.allowance,
        })),
      });
    });

    return reply.status(201).send({
      root: tree.rootHex,
      rootBytes: tree.rootBytes,
      entryCount: tree.size,
      nextStep: 'Pass rootBytes as the merkle_root when calling add_phase',
    });
  });

  /** Allowlist proof for a wallet. A 404 means not on the list, which is a normal UI state. */
  app.get('/phases/:address/allowlist/:wallet', async (request, reply) => {
    const params = request.params as {address: string; wallet: string};

    const rows = await prisma.allowlistEntry.findMany({
      where: {phaseAddress: params.address},
    });

    if (rows.length === 0) {
      return reply.status(404).send({error: 'NO_ALLOWLIST_CONFIGURED'});
    }

    const entry = rows.find((row) => row.wallet === params.wallet);
    if (!entry) {
      return reply.status(404).send({error: 'NOT_ELIGIBLE'});
    }

    // Rebuilt from stored entries: the tree is a pure function of them, so storing it too would be
    // a second source of truth that can drift.
    const tree = new Allowlist(
      rows.map((row) => ({wallet: new PublicKey(row.wallet), allowance: row.allowance})),
    );

    const proof = tree.proofFor(entry.wallet);
    if (!proof) {
      return reply.status(404).send({error: 'NOT_ELIGIBLE'});
    }

    return reply.send({
      wallet: entry.wallet,
      allowance: entry.allowance,
      proof: proof.map((node) => Array.from(node)),
      proofHex: proof.map((node) => node.toString('hex')),
      root: tree.rootHex,
    });
  });

  /*//////////////////////////////////////////////////////////////
                                ASSETS
  //////////////////////////////////////////////////////////////*/

  /**
   * A wallet's NFTs, from DAS.
   *
   * DAS rather than a token-account scan, because a token-account scan misses compressed NFTs
   * entirely, and compressed NFTs are why anyone chooses Solana for a large collection.
   */
  app.get('/wallet/:owner/assets', async (request, reply) => {
    const {owner} = request.params as {owner: string};

    try {
      const page = await das.getAssetsByOwner(owner);

      const listings = await prisma.listing.findMany({
        where: {seller: owner, isActive: true},
      });

      const listingByMint = new Map(listings.map((listing) => [listing.mint, listing]));

      return reply.send({
        owner,
        total: page.total,
        assets: page.items.map((asset) => ({
          id: asset.id,
          name: asset.content?.metadata?.name ?? null,
          image: asset.content?.files?.[0]?.uri ?? null,
          compressed: asset.compression?.compressed ?? false,
          collection:
            asset.grouping?.find((group) => group.group_key === 'collection')?.group_value ?? null,
          listing: listingByMint.get(asset.id) ?? null,
        })),
      });
    } catch (error) {
      if (error instanceof DasUnsupportedError) {
        // A specific, actionable message rather than a generic 500.
        return reply.status(503).send({error: 'DAS_UNSUPPORTED', message: error.message});
      }
      throw error;
    }
  });

  app.get('/assets/:mint', async (request, reply) => {
    const {mint} = request.params as {mint: string};

    const [asset, listing, offers, history] = await Promise.all([
      das.getAsset(mint).catch(() => null),
      prisma.listing.findUnique({where: {mint}}),
      prisma.offer.findMany({
        where: {mint, isActive: true, expiresTs: {gt: new Date()}},
        orderBy: {amount: 'desc'},
      }),
      prisma.sale.findMany({where: {mint}, orderBy: {blockTime: 'desc'}, take: 20}),
    ]);

    if (!asset && !listing) {
      return reply.status(404).send({error: 'NOT_FOUND'});
    }

    return reply.send({
      mint,
      asset: asset
        ? {
            name: asset.content?.metadata?.name ?? null,
            description: asset.content?.metadata?.description ?? null,
            image: asset.content?.files?.[0]?.uri ?? null,
            owner: asset.ownership.owner,
            compressed: asset.compression?.compressed ?? false,
            royaltyBps: asset.royalty?.basis_points ?? null,
          }
        : null,
      listing: listing?.isActive ? listing : null,
      offers,
      history,
    });
  });

  /**
   * Compressed NFT transfer proof.
   *
   * A compressed NFT has no account: it is a leaf in a Merkle tree with only the root on-chain, so
   * moving one requires proving membership. This is the concrete cost of compression, and the
   * reason an indexer is not optional for a compressed collection.
   */
  app.get('/assets/:mint/proof', async (request, reply) => {
    const {mint} = request.params as {mint: string};

    try {
      const proof = await das.getAssetProof(mint);
      return reply.send(proof);
    } catch (error) {
      if (error instanceof DasUnsupportedError) {
        return reply.status(503).send({error: 'DAS_UNSUPPORTED', message: error.message});
      }
      return reply.status(404).send({error: 'NOT_COMPRESSED_OR_NOT_FOUND'});
    }
  });

  /**
   * Cost comparison between regular and compressed NFTs.
   *
   * The single most persuasive argument for Solana on a large collection, and a number is more
   * convincing than a claim.
   */
  app.get('/compression/savings', async (request, reply) => {
    const schema = z.object({count: z.coerce.number().int().min(1).max(100_000_000).default(10_000)});
    const parsed = schema.safeParse(request.query);

    const count = parsed.success ? parsed.data.count : 10_000;
    const savings = compressionSavings(count);

    return reply.send({count, ...savings});
  });

  /*//////////////////////////////////////////////////////////////
                                OFFERS
  //////////////////////////////////////////////////////////////*/

  app.get('/wallet/:buyer/offers', async (request, reply) => {
    const {buyer} = request.params as {buyer: string};

    const offers = await prisma.offer.findMany({
      where: {buyer, isActive: true},
      orderBy: {expiresTs: 'asc'},
    });

    return reply.send({offers});
  });
}
