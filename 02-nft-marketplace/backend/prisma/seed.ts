/**
 * Demo data.
 *
 * The indexer fills these tables by reading program accounts, so a fresh database renders every
 * page as an empty state. This writes a plausible slice of a collection and its market so the UI
 * can be clicked through immediately, and so the README screenshots show the app working.
 *
 * Asset images and names are not stored here, deliberately: on this project they come from DAS,
 * because a compressed NFT has no account to read. Listings, offers and sale history are what this
 * database owns, and that is what gets seeded.
 *
 * Safe to re-run: every table it touches is cleared first. Local databases only.
 *
 *   pnpm db:seed
 */
import {PrismaClient} from '@prisma/client';

const prisma = new PrismaClient();

/**
 * Demo wallets: the system program's address with the last byte varied. Valid base58, stable across
 * runs, and obviously nobody's real wallet.
 */
const WALLETS = {
  alice: '11111111111111111111111111111112',
  bob: '11111111111111111111111111111113',
  carol: '11111111111111111111111111111114',
  dave: '11111111111111111111111111111115',
} as const;

const COLLECTION = 'CoLLect1on1111111111111111111111111111111111';
const TREASURY = 'TrEasuRy111111111111111111111111111111111111';

const LAMPORTS = 10n ** 9n;

function base58ish(prefix: string, index: number): string {
  // The 'z' terminates the index before the padding. Without it, padding with '1' makes index 1 and
  // index 11 produce the identical string, which collides on any unique column.
  return `${prefix}${index}z`.padEnd(44, '1').slice(0, 44);
}

function hoursAgo(hours: number): Date {
  return new Date(Date.now() - hours * 3_600_000);
}

function hoursFromNow(hours: number): Date {
  return new Date(Date.now() + hours * 3_600_000);
}

/** The all-zero root marks a phase as public. Anything else gates it behind an allowlist. */
const PUBLIC_ROOT = '0'.repeat(64);

async function main(): Promise<void> {
  console.log('clearing existing demo data');

  await prisma.sale.deleteMany();
  await prisma.offer.deleteMany();
  await prisma.listing.deleteMany();
  await prisma.allowlistEntry.deleteMany();
  await prisma.mintPhase.deleteMany();
  await prisma.collection.deleteMany();

  /* ------------------------------------------------------------- collection --- */

  const MAX_SUPPLY = 5_000;
  const MINTED = 1_842;

  console.log('seeding a collection with 2 phases');

  await prisma.collection.create({
    data: {
      address: COLLECTION,
      authority: WALLETS.alice,
      treasury: TREASURY,
      name: 'B1zt Genesis',
      baseUri: 'https://demo.b1zt.dev/metadata/',
      maxSupply: MAX_SUPPLY,
      minted: MINTED,
      royaltyBps: 500,
      royaltyRecipient: TREASURY,
      phaseCount: 2,
      floorPrice: (LAMPORTS * 32n / 10n).toString(),
      volume: (LAMPORTS * 2_180n).toString(),
    },
  });

  // A closed allowlist phase and a live public one. Two phases in different states is what makes
  // the mint timeline worth looking at.
  const phases = [
    {
      address: base58ish('PHASE', 0),
      index: 0,
      merkleRoot: '2f43697b1ea109ad367ea31705f983697dd4cc11fb22a94021113fa0419c622e',
      price: 25n,
      startTs: hoursAgo(96),
      endTs: hoursAgo(24),
      maxPerWallet: 0,
      maxSupply: 1_000,
      minted: 1_000,
    },
    {
      address: base58ish('PHASE', 1),
      index: 1,
      merkleRoot: PUBLIC_ROOT,
      price: 40n,
      startTs: hoursAgo(24),
      endTs: hoursFromNow(120),
      maxPerWallet: 5,
      maxSupply: 0,
      minted: 842,
    },
  ];

  for (const phase of phases) {
    await prisma.mintPhase.create({
      data: {
        address: phase.address,
        collectionAddress: COLLECTION,
        index: phase.index,
        merkleRoot: phase.merkleRoot,
        price: ((LAMPORTS * phase.price) / 100n).toString(),
        startTs: phase.startTs,
        endTs: phase.endTs,
        maxPerWallet: phase.maxPerWallet,
        maxSupply: phase.maxSupply,
        minted: phase.minted,
      },
    });
  }

  // The allowlist for phase 0. The on-chain root commits to these without revealing them, so this
  // table is the only place a proof can be generated from: lose it and nobody can ever mint again.
  // These five entries are the same fixture the Merkle cross-check tests use on both sides.
  await prisma.allowlistEntry.createMany({
    data: [
      {phaseAddress: phases[0]!.address, wallet: WALLETS.alice, allowance: 1},
      {phaseAddress: phases[0]!.address, wallet: WALLETS.bob, allowance: 2},
      {phaseAddress: phases[0]!.address, wallet: WALLETS.carol, allowance: 3},
      {phaseAddress: phases[0]!.address, wallet: WALLETS.dave, allowance: 5},
      {phaseAddress: phases[0]!.address, wallet: '11111111111111111111111111111116', allowance: 8},
    ],
  });

  /* --------------------------------------------------------------- listings --- */

  // Cheapest first, so the floor price above is the one the grid actually shows.
  const listingPrices = [32n, 38n, 45n, 51n, 60n, 74n, 88n, 110n, 145n, 190n];
  const owners = [WALLETS.alice, WALLETS.bob, WALLETS.carol, WALLETS.dave];

  console.log(`seeding ${listingPrices.length} listings`);

  for (const [index, tenths] of listingPrices.entries()) {
    await prisma.listing.create({
      data: {
        address: base58ish('LISTing', index),
        seller: owners[index % owners.length]!,
        mint: base58ish('MINT', index),
        // Listings are escrowed, so the token really does leave the seller's wallet.
        escrow: base58ish('ESCROW', index),
        collectionAddress: COLLECTION,
        price: ((LAMPORTS * tenths) / 10n).toString(),
        createdTs: hoursAgo(60 - index * 4),
        isActive: true,
      },
    });
  }

  /* ----------------------------------------------------------------- offers --- */

  console.log('seeding 4 offers');

  const offers = [28n, 30n, 35n, 42n];

  for (const [index, tenths] of offers.entries()) {
    await prisma.offer.create({
      data: {
        address: base58ish('OFFER', index),
        buyer: owners[(index + 1) % owners.length]!,
        mint: base58ish('MINT', index),
        amount: ((LAMPORTS * tenths) / 10n).toString(),
        expiresTs: hoursFromNow(24 * (index + 1)),
        isActive: true,
      },
    });
  }

  /* ------------------------------------------------------------------ sales --- */

  // Prices wander rather than climbing steadily, because a chart that only goes up reads as
  // fabricated. Fee and royalty are the real 2.5% and 5% the program would take.
  const salePrices = [26n, 41n, 33n, 52n, 38n, 64n, 47n, 71n, 55n, 82n, 68n, 95n];

  console.log(`seeding ${salePrices.length} sales`);

  for (const [index, tenths] of salePrices.entries()) {
    const price = (LAMPORTS * tenths) / 10n;

    await prisma.sale.create({
      data: {
        collectionAddress: COLLECTION,
        mint: base58ish('MINT', index),
        seller: owners[index % owners.length]!,
        buyer: owners[(index + 2) % owners.length]!,
        price: price.toString(),
        fee: ((price * 250n) / 10_000n).toString(),
        royalty: ((price * 500n) / 10_000n).toString(),
        signature: base58ish('SALEsig', index),
        slot: BigInt(302_000_000 + index * 14_400),
        blockTime: hoursAgo((salePrices.length - index) * 5),
      },
    });
  }

  console.log('\ndone.');
  console.log(`  collection ${COLLECTION}`);
  console.log(`  ${MINTED} of ${MAX_SUPPLY} minted across ${phases.length} phases`);
  console.log(`  ${listingPrices.length} listings, ${offers.length} offers, ${salePrices.length} sales`);
}

main()
  .catch((error) => {
    console.error(error);
    process.exit(1);
  })
  .finally(() => prisma.$disconnect());
