import {PublicKey} from '@solana/web3.js';
import {createHash} from 'node:crypto';
import {connection} from '../chain/connection.js';
import {config} from '../config.js';
import {logger} from '../lib/logger.js';
import {prisma} from '../lib/prisma.js';

/**
 * Marketplace indexer.
 *
 * Reads current state through `getProgramAccounts`, filtered server-side on Anchor's account
 * discriminator so the RPC returns only the type being asked for. On a program with thousands of
 * accounts that filter is the difference between a usable query and a timeout.
 *
 * Asset metadata is deliberately not indexed here. It comes from DAS, because a compressed NFT has
 * no account to read, and duplicating what DAS already serves would create a second copy that
 * drifts. What this indexer owns is what DAS does not answer: listings, offers and sale history.
 */
export class Indexer {
  private running = false;
  private timer: NodeJS.Timeout | null = null;
  private readonly discriminators = new Map<string, Buffer>();

  constructor() {
    for (const name of ['Collection', 'MintPhase', 'Listing', 'Offer', 'Marketplace']) {
      // Anchor tags every account it owns with sha256("account:<Name>")[..8].
      const hash = createHash('sha256').update(`account:${name}`).digest();
      this.discriminators.set(name, hash.subarray(0, 8));
    }
  }

  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;

    logger.info(
      {programId: config.programId.toBase58(), cluster: config.CLUSTER},
      'indexer starting',
    );

    await this.tick();
    this.scheduleNext();
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    logger.info('indexer stopped');
  }

  private scheduleNext(): void {
    if (!this.running) return;

    this.timer = setTimeout(() => {
      void this.tick()
        .catch((error) => logger.error({error}, 'indexer pass failed'))
        .finally(() => this.scheduleNext());
    }, config.INDEXER_POLL_INTERVAL * 1_000);
  }

  private async tick(): Promise<void> {
    await this.syncCollections();
    await this.syncPhases();
    await this.syncListings();
    await this.syncOffers();
    await this.refreshFloorPrices();
  }

  private async fetchAccounts(typeName: string): Promise<{pubkey: PublicKey; data: Buffer}[]> {
    const discriminator = this.discriminators.get(typeName);
    if (!discriminator) return [];

    const accounts = await connection.getProgramAccounts(config.programId, {
      filters: [{memcmp: {offset: 0, bytes: discriminator.toString('base64'), encoding: 'base64'}}],
    });

    return accounts.map((entry) => ({pubkey: entry.pubkey, data: entry.account.data as Buffer}));
  }

  /** Borsh reader over a raw account buffer, starting after the 8-byte discriminator. */
  private reader(data: Buffer) {
    let offset = 8;

    return {
      pubkey: (): string => {
        const key = new PublicKey(data.subarray(offset, offset + 32)).toBase58();
        offset += 32;
        return key;
      },
      u64: (): bigint => {
        const value = data.readBigUInt64LE(offset);
        offset += 8;
        return value;
      },
      i64: (): bigint => {
        const value = data.readBigInt64LE(offset);
        offset += 8;
        return value;
      },
      u32: (): number => {
        const value = data.readUInt32LE(offset);
        offset += 4;
        return value;
      },
      u16: (): number => {
        const value = data.readUInt16LE(offset);
        offset += 2;
        return value;
      },
      u8: (): number => {
        const value = data.readUInt8(offset);
        offset += 1;
        return value;
      },
      bytes32: (): string => {
        const value = data.subarray(offset, offset + 32).toString('hex');
        offset += 32;
        return value;
      },
      /** Borsh strings are a u32 length followed by UTF-8 bytes. */
      string: (): string => {
        const length = data.readUInt32LE(offset);
        offset += 4;
        const value = data.subarray(offset, offset + length).toString('utf8');
        offset += length;
        return value;
      },
    };
  }

  private async syncCollections(): Promise<void> {
    const accounts = await this.fetchAccounts('Collection');

    for (const {pubkey, data} of accounts) {
      try {
        const read = this.reader(data);

        const authority = read.pubkey();
        const treasury = read.pubkey();
        const maxSupply = read.u32();
        const minted = read.u32();
        const royaltyBps = read.u16();
        const royaltyRecipient = read.pubkey();
        const name = read.string();
        const baseUri = read.string();
        const phaseCount = read.u8();

        await prisma.collection.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            authority,
            treasury,
            name,
            baseUri,
            maxSupply,
            minted,
            royaltyBps,
            royaltyRecipient,
            phaseCount,
          },
          update: {minted, phaseCount, royaltyBps, royaltyRecipient},
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode collection');
      }
    }
  }

  private async syncPhases(): Promise<void> {
    const accounts = await this.fetchAccounts('MintPhase');

    for (const {pubkey, data} of accounts) {
      try {
        const read = this.reader(data);

        const collection = read.pubkey();
        const index = read.u8();
        const merkleRoot = read.bytes32();
        const price = read.u64();
        const startTs = read.i64();
        const endTs = read.i64();
        const maxPerWallet = read.u16();
        const maxSupply = read.u32();
        const minted = read.u32();

        // The collection row must exist for the foreign key. An indexer started mid-life can see a
        // phase before it has seen its collection.
        const exists = await prisma.collection.findUnique({where: {address: collection}});
        if (!exists) continue;

        await prisma.mintPhase.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            collectionAddress: collection,
            index,
            merkleRoot,
            price: price.toString(),
            startTs: new Date(Number(startTs) * 1000),
            endTs: new Date(Number(endTs) * 1000),
            maxPerWallet,
            maxSupply,
            minted,
          },
          update: {minted},
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode phase');
      }
    }
  }

  private async syncListings(): Promise<void> {
    const accounts = await this.fetchAccounts('Listing');
    const seen = new Set<string>();

    for (const {pubkey, data} of accounts) {
      try {
        const read = this.reader(data);

        const seller = read.pubkey();
        const mint = read.pubkey();
        const escrow = read.pubkey();
        const collection = read.pubkey();
        const price = read.u64();
        const createdTs = read.i64();

        const exists = await prisma.collection.findUnique({where: {address: collection}});
        if (!exists) continue;

        seen.add(pubkey.toBase58());

        await prisma.listing.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            seller,
            mint,
            escrow,
            collectionAddress: collection,
            price: price.toString(),
            createdTs: new Date(Number(createdTs) * 1000),
            isActive: true,
          },
          update: {price: price.toString(), isActive: true},
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode listing');
      }
    }

    // A listing account that no longer exists was sold or delisted. Marking it inactive rather
    // than deleting keeps the history readable on a profile page.
    await prisma.listing.updateMany({
      where: {isActive: true, address: {notIn: [...seen]}},
      data: {isActive: false},
    });
  }

  private async syncOffers(): Promise<void> {
    const accounts = await this.fetchAccounts('Offer');
    const seen = new Set<string>();

    for (const {pubkey, data} of accounts) {
      try {
        const read = this.reader(data);

        const buyer = read.pubkey();
        const mint = read.pubkey();
        const amount = read.u64();
        const expiresTs = read.i64();

        seen.add(pubkey.toBase58());

        await prisma.offer.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            buyer,
            mint,
            amount: amount.toString(),
            expiresTs: new Date(Number(expiresTs) * 1000),
            isActive: true,
          },
          update: {amount: amount.toString(), isActive: true},
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode offer');
      }
    }

    await prisma.offer.updateMany({
      where: {isActive: true, address: {notIn: [...seen]}},
      data: {isActive: false},
    });
  }

  /**
   * Recompute floor prices from active listings.
   *
   * Derived rather than maintained incrementally. Incremental is faster but drifts the moment a
   * single update is missed, and a wrong floor price is the most visible possible bug on a
   * marketplace front page.
   */
  private async refreshFloorPrices(): Promise<void> {
    const collections = await prisma.collection.findMany({select: {address: true}});

    for (const {address} of collections) {
      const listings = await prisma.listing.findMany({
        where: {collectionAddress: address, isActive: true},
        select: {price: true},
      });

      // Sorted in JS rather than SQL because prices are stored as strings, and lexicographic
      // ordering would put "9" above "10".
      const floor = listings
        .map((listing) => BigInt(listing.price))
        .reduce<bigint | null>(
          (lowest, price) => (lowest === null || price < lowest ? price : lowest),
          null,
        );

      await prisma.collection.update({
        where: {address},
        data: {floorPrice: floor?.toString() ?? null},
      });
    }
  }
}
