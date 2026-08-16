/**
 * Demo data.
 *
 * The indexer fills these tables by reading program accounts, so a fresh database renders every
 * page as an empty state. This writes a plausible slice of activity so the UI can be clicked
 * through immediately, and so the README screenshots show the app working.
 *
 * Safe to re-run: every table it touches is cleared first. Local databases only.
 *
 *   pnpm db:seed
 */
import {PrismaClient} from '@prisma/client';

const prisma = new PrismaClient();

/**
 * Demo wallets.
 *
 * The system program's address with the last byte varied, which gives valid base58 keys that are
 * stable across runs and obviously not anybody's real wallet.
 */
const WALLETS = {
  alice: '11111111111111111111111111111112',
  bob: '11111111111111111111111111111113',
  carol: '11111111111111111111111111111114',
  dave: '11111111111111111111111111111115',
  erin: '11111111111111111111111111111116',
} as const;

const DISTRIBUTOR = 'DiStR1butoR11111111111111111111111111111111';
const POOL = 'StAkEPoo111111111111111111111111111111111111';
const MINT = 'M1nT1111111111111111111111111111111111111111';
const AUTHORITY = 'AuTh0r1ty111111111111111111111111111111111';

/** Nine decimals, matching the mint the platform creates. */
const TOKEN = 10n ** 9n;

function base58ish(prefix: string, index: number): string {
  // The 'z' terminates the index before the padding. Without it, padding with '1' makes index 1 and
  // index 11 produce the identical string, which collides on any unique column.
  return `${prefix}${index}z`.padEnd(44, '1').slice(0, 44);
}

function hoursAgo(hours: number): Date {
  return new Date(Date.now() - hours * 3_600_000);
}

function daysFromNow(days: number): Date {
  return new Date(Date.now() + days * 86_400_000);
}

async function main(): Promise<void> {
  console.log('clearing existing demo data');

  await prisma.poolSnapshot.deleteMany();
  await prisma.stakePosition.deleteMany();
  await prisma.stakePool.deleteMany();
  await prisma.vestingRelease.deleteMany();
  await prisma.vestingSchedule.deleteMany();
  await prisma.airdropEntry.deleteMany();
  await prisma.airdropRoot.deleteMany();

  /* ---------------------------------------------------------------- airdrop --- */

  const airdrop = [
    {claimant: WALLETS.alice, amount: 12_500n, claimed: true},
    {claimant: WALLETS.bob, amount: 8_000n, claimed: true},
    {claimant: WALLETS.carol, amount: 25_000n, claimed: false},
    {claimant: WALLETS.dave, amount: 4_200n, claimed: false},
    {claimant: WALLETS.erin, amount: 60_000n, claimed: true},
  ];

  console.log(`seeding ${airdrop.length} airdrop entries`);

  await prisma.airdropEntry.createMany({
    data: airdrop.map((entry, index) => ({
      distributor: DISTRIBUTOR,
      index,
      claimant: entry.claimant,
      amount: (entry.amount * TOKEN).toString(),
      claimedAt: entry.claimed ? hoursAgo(30 - index * 4) : null,
      claimTx: entry.claimed ? base58ish('CLAIMtx', index) : null,
    })),
  });

  await prisma.airdropRoot.create({
    data: {
      distributor: DISTRIBUTOR,
      root: 'd05181965dd81581e12198d2ed0c84fd085d6efffc7c9c0045f8ff93c686c0b8',
      entryCount: airdrop.length,
      totalAmount: (airdrop.reduce((sum, entry) => sum + entry.amount, 0n) * TOKEN).toString(),
      deadline: daysFromNow(74),
    },
  });

  /* ---------------------------------------------------------------- vesting --- */

  const schedules = [
    {
      address: base58ish('VESTing', 1),
      beneficiary: WALLETS.alice,
      total: 250_000n,
      // 120 of 730 days vests about 41,095, so a claim of 25,000 leaves a real balance behind.
      released: 25_000n,
      startedDaysAgo: 120,
      cliffDays: 90,
      durationDays: 730,
      revocable: true,
    },
    {
      address: base58ish('VESTing', 2),
      beneficiary: WALLETS.bob,
      total: 400_000n,
      released: 0n,
      // Still inside its cliff: allocated, not yet claimable. The case worth showing.
      startedDaysAgo: 30,
      cliffDays: 180,
      durationDays: 1_095,
      revocable: true,
    },
    {
      address: base58ish('VESTing', 3),
      beneficiary: WALLETS.carol,
      total: 100_000n,
      released: 100_000n,
      startedDaysAgo: 800,
      cliffDays: 0,
      durationDays: 365,
      revocable: false,
    },
  ];

  console.log(`seeding ${schedules.length} vesting schedules`);

  for (const [index, schedule] of schedules.entries()) {
    await prisma.vestingSchedule.create({
      data: {
        address: schedule.address,
        beneficiary: schedule.beneficiary,
        authority: AUTHORITY,
        mint: MINT,
        vault: base58ish('VAULT', index),
        totalAmount: (schedule.total * TOKEN).toString(),
        releasedAmount: (schedule.released * TOKEN).toString(),
        startTs: hoursAgo(schedule.startedDaysAgo * 24),
        cliffSeconds: schedule.cliffDays * 86_400,
        durationSeconds: schedule.durationDays * 86_400,
        revocable: schedule.revocable,
        revoked: false,
        seed: String(index),
      },
    });
  }

  await prisma.vestingRelease.createMany({
    data: [
      {scheduleAddress: schedules[0]!.address, amount: (15_000n * TOKEN).toString(), signature: base58ish('RELEASE', 1), slot: 301_000_000n, blockTime: hoursAgo(600)},
      {scheduleAddress: schedules[0]!.address, amount: (10_000n * TOKEN).toString(), signature: base58ish('RELEASE', 2), slot: 302_400_000n, blockTime: hoursAgo(180)},
      {scheduleAddress: schedules[2]!.address, amount: (100_000n * TOKEN).toString(), signature: base58ish('RELEASE', 3), slot: 290_100_000n, blockTime: hoursAgo(2_400)},
    ],
  });

  /* ---------------------------------------------------------------- staking --- */

  console.log('seeding a stake pool with 4 stakers');

  const stakers = [
    {owner: WALLETS.alice, amount: 180_000n, pending: 1_240n},
    {owner: WALLETS.bob, amount: 95_000n, pending: 655n},
    {owner: WALLETS.carol, amount: 41_000n, pending: 283n},
    {owner: WALLETS.erin, amount: 12_500n, pending: 86n},
  ];

  const totalStaked = stakers.reduce((sum, staker) => sum + staker.amount, 0n);

  await prisma.stakePool.create({
    data: {
      address: POOL,
      authority: AUTHORITY,
      stakeMint: MINT,
      rewardMint: MINT,
      stakeVault: base58ish('STAKEvault', 1),
      rewardVault: base58ish('REWARDvault', 1),
      totalStaked: (totalStaked * TOKEN).toString(),
      // Roughly 20% a year on the staked total, expressed per second.
      rewardRate: ((totalStaked * TOKEN * 20n) / 100n / 31_536_000n).toString(),
      rewardEndTs: daysFromNow(45),
      cooldownSeconds: 3 * 86_400,
      aprBps: 2_000,
    },
  });

  for (const [index, staker] of stakers.entries()) {
    await prisma.stakePosition.create({
      data: {
        poolAddress: POOL,
        address: base58ish('POSition', index),
        owner: staker.owner,
        amount: (staker.amount * TOKEN).toString(),
        pendingReward: (staker.pending * TOKEN).toString(),
        // One staker mid-cooldown, so the unstaking path is visible rather than implied.
        unstakingAmount: index === 1 ? (10_000n * TOKEN).toString() : '0',
        unstakeReadyTs: index === 1 ? daysFromNow(2) : null,
      },
    });
  }

  // A fortnight of daily snapshots, so the APR chart has a shape.
  await prisma.poolSnapshot.createMany({
    data: Array.from({length: 14}, (_unused, day) => ({
      poolAddress: POOL,
      totalStaked: ((totalStaked * TOKEN * BigInt(9_400 + day * 45)) / 10_000n).toString(),
      // APR wanders rather than trending one way, because a line that only rises reads as invented.
      aprBps: 2_000 + Math.round(Math.sin(day) * 220),
      slot: BigInt(302_000_000 + day * 216_000),
      capturedAt: hoursAgo((13 - day) * 24),
    })),
  });

  console.log('\ndone.');
  console.log(`  ${airdrop.length} airdrop entries, ${schedules.length} vesting schedules`);
  console.log(`  1 pool with ${stakers.length} stakers`);
  console.log(`\n  distributor ${DISTRIBUTOR}`);
  console.log(`  pool        ${POOL}`);
}

main()
  .catch((error) => {
    console.error(error);
    process.exit(1);
  })
  .finally(() => prisma.$disconnect());
