import {PublicKey, type ConfirmedSignatureInfo} from '@solana/web3.js';
import {connection} from '../chain/connection.js';
import {config} from '../config.js';
import {logger} from '../lib/logger.js';
import {prisma} from '../lib/prisma.js';

/**
 * Solana indexer.
 *
 * Nothing here resembles an EVM log scan, because Solana has no equivalent primitive. There is no
 * `eth_getLogs`, no block range filter, and no topic index. Two mechanisms exist instead, and this
 * uses both for the things each is good at:
 *
 * **`getProgramAccounts` for current state.** One call returns every account the program owns,
 * already decoded to its current values. There is no need to replay history to learn what a vesting
 * schedule currently holds: the account simply says. This is a genuine advantage over EVM, where
 * the same question requires either a view call per item or an event replay.
 *
 * **`getSignaturesForAddress` for history.** Walking an account's transaction history backwards
 * from its newest signature. This is the only way to reconstruct *when* things happened, and it is
 * paginated by signature rather than by block, so the cursor stores a signature.
 *
 * Reorgs are handled differently too. Solana forks are resolved within a couple of slots and
 * confirmed transactions are effectively final, so there is no delete-and-rescan window like the
 * EVM indexers here need. What replaces it is the RPC's limited history: most providers prune
 * signatures beyond a few days, which is exactly why current state comes from `getProgramAccounts`
 * rather than from replaying events since genesis.
 */
export class Indexer {
  private running = false;
  private timer: NodeJS.Timeout | null = null;

  /** Anchor's 8-byte account discriminators, resolved once from the IDL-derived names. */
  private readonly discriminators = new Map<string, Buffer>();

  async start(): Promise<void> {
    if (this.running) return;
    this.running = true;

    await this.loadDiscriminators();

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
    await this.syncVestingSchedules();
    await this.syncStakePools();
    await this.syncClaims();
    await this.snapshotPools();
  }

  /**
   * Anchor prefixes every account it owns with `sha256("account:<Name>")[..8]`.
   *
   * Filtering on it server-side means the RPC returns only the account type wanted, rather than
   * every account the program owns. On a program with thousands of accounts that is the difference
   * between a usable query and a timeout.
   */
  private async loadDiscriminators(): Promise<void> {
    const {createHash} = await import('node:crypto');

    for (const name of ['VestingSchedule', 'StakePool', 'StakeAccount', 'Distributor', 'ClaimStatus']) {
      const hash = createHash('sha256').update(`account:${name}`).digest();
      this.discriminators.set(name, hash.subarray(0, 8));
    }
  }

  /**
   * Fetch every account of one type, decoded from its raw bytes.
   *
   * Layouts are decoded by hand rather than through Anchor's client. The borsh layout is fixed and
   * documented in `state.rs`, and hand-decoding avoids shipping the IDL and the whole Anchor client
   * into a service that only ever reads.
   */
  private async fetchAccounts(typeName: string): Promise<{pubkey: PublicKey; data: Buffer}[]> {
    const discriminator = this.discriminators.get(typeName);
    if (!discriminator) return [];

    const accounts = await connection.getProgramAccounts(config.programId, {
      filters: [{memcmp: {offset: 0, bytes: discriminator.toString('base64'), encoding: 'base64'}}],
    });

    return accounts.map((entry) => ({
      pubkey: entry.pubkey,
      data: entry.account.data as Buffer,
    }));
  }

  /*//////////////////////////////////////////////////////////////
                                VESTING
  //////////////////////////////////////////////////////////////*/

  private async syncVestingSchedules(): Promise<void> {
    const accounts = await this.fetchAccounts('VestingSchedule');

    for (const {pubkey, data} of accounts) {
      try {
        // Layout after the 8-byte discriminator, matching `state.rs`:
        //   beneficiary(32) authority(32) mint(32) vault(32)
        //   total(8) released(8) start(8) cliff(8) duration(8)
        //   revocable(1) revoked(1) seed(8) bump(1)
        let offset = 8;

        const readPubkey = (): string => {
          const key = new PublicKey(data.subarray(offset, offset + 32)).toBase58();
          offset += 32;
          return key;
        };
        const readU64 = (): bigint => {
          const value = data.readBigUInt64LE(offset);
          offset += 8;
          return value;
        };
        const readI64 = (): bigint => {
          const value = data.readBigInt64LE(offset);
          offset += 8;
          return value;
        };
        const readBool = (): boolean => {
          const value = data.readUInt8(offset) === 1;
          offset += 1;
          return value;
        };

        const beneficiary = readPubkey();
        const authority = readPubkey();
        const mint = readPubkey();
        const vault = readPubkey();
        const totalAmount = readU64();
        const releasedAmount = readU64();
        const startTs = readI64();
        const cliffSeconds = readI64();
        const durationSeconds = readI64();
        const revocable = readBool();
        const revoked = readBool();
        const seed = readU64();

        await prisma.vestingSchedule.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            beneficiary,
            authority,
            mint,
            vault,
            totalAmount: totalAmount.toString(),
            releasedAmount: releasedAmount.toString(),
            startTs: new Date(Number(startTs) * 1000),
            cliffSeconds: Number(cliffSeconds),
            durationSeconds: Number(durationSeconds),
            revocable,
            revoked,
            seed: seed.toString(),
          },
          update: {
            releasedAmount: releasedAmount.toString(),
            totalAmount: totalAmount.toString(),
            revoked,
          },
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode vesting schedule');
      }
    }

    if (accounts.length > 0) {
      logger.debug({count: accounts.length}, 'synced vesting schedules');
    }
  }

  /*//////////////////////////////////////////////////////////////
                                STAKING
  //////////////////////////////////////////////////////////////*/

  private async syncStakePools(): Promise<void> {
    const pools = await this.fetchAccounts('StakePool');

    for (const {pubkey, data} of pools) {
      try {
        let offset = 8;

        const readPubkey = (): string => {
          const key = new PublicKey(data.subarray(offset, offset + 32)).toBase58();
          offset += 32;
          return key;
        };
        const readU64 = (): bigint => {
          const value = data.readBigUInt64LE(offset);
          offset += 8;
          return value;
        };
        const readI64 = (): bigint => {
          const value = data.readBigInt64LE(offset);
          offset += 8;
          return value;
        };

        const authority = readPubkey();
        const stakeMint = readPubkey();
        const rewardMint = readPubkey();
        const stakeVault = readPubkey();
        const rewardVault = readPubkey();
        const totalStaked = readU64();
        const rewardRate = readU64();
        // reward_per_token is a u128, so it is skipped rather than read: nothing here needs it.
        offset += 16;
        readI64(); // last_update_ts
        const rewardEndTs = readI64();
        const cooldownSeconds = readI64();

        // APR from the emission rate against what is staked. Simple rather than compounding,
        // because compounding assumes a reinvestment schedule the pool does not perform.
        const SECONDS_PER_YEAR = 31_536_000n;
        const aprBps =
          totalStaked > 0n
            ? Number((rewardRate * SECONDS_PER_YEAR * 10_000n) / totalStaked)
            : 0;

        await prisma.stakePool.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            authority,
            stakeMint,
            rewardMint,
            stakeVault,
            rewardVault,
            totalStaked: totalStaked.toString(),
            rewardRate: rewardRate.toString(),
            rewardEndTs: new Date(Number(rewardEndTs) * 1000),
            cooldownSeconds: Number(cooldownSeconds),
            aprBps: Math.min(aprBps, 10_000_000),
          },
          update: {
            totalStaked: totalStaked.toString(),
            rewardRate: rewardRate.toString(),
            rewardEndTs: new Date(Number(rewardEndTs) * 1000),
            aprBps: Math.min(aprBps, 10_000_000),
          },
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode stake pool');
      }
    }

    const positions = await this.fetchAccounts('StakeAccount');

    for (const {pubkey, data} of positions) {
      try {
        let offset = 8;

        const owner = new PublicKey(data.subarray(offset, offset + 32)).toBase58();
        offset += 32;
        const pool = new PublicKey(data.subarray(offset, offset + 32)).toBase58();
        offset += 32;

        const amount = data.readBigUInt64LE(offset);
        offset += 8;
        offset += 16; // reward_debt, a u128
        const pendingReward = data.readBigUInt64LE(offset);
        offset += 8;
        const unstakingAmount = data.readBigUInt64LE(offset);
        offset += 8;
        const unstakeReadyTs = data.readBigInt64LE(offset);

        // The pool row must exist for the foreign key; a position for an unknown pool is skipped
        // rather than failing the pass.
        const poolExists = await prisma.stakePool.findUnique({where: {address: pool}});
        if (!poolExists) continue;

        await prisma.stakePosition.upsert({
          where: {address: pubkey.toBase58()},
          create: {
            address: pubkey.toBase58(),
            poolAddress: pool,
            owner,
            amount: amount.toString(),
            pendingReward: pendingReward.toString(),
            unstakingAmount: unstakingAmount.toString(),
            unstakeReadyTs:
              unstakeReadyTs > 0n ? new Date(Number(unstakeReadyTs) * 1000) : null,
          },
          update: {
            amount: amount.toString(),
            pendingReward: pendingReward.toString(),
            unstakingAmount: unstakingAmount.toString(),
            unstakeReadyTs:
              unstakeReadyTs > 0n ? new Date(Number(unstakeReadyTs) * 1000) : null,
          },
        });
      } catch (error) {
        logger.warn({error, account: pubkey.toBase58()}, 'failed to decode stake position');
      }
    }
  }

  /*//////////////////////////////////////////////////////////////
                                CLAIMS
  //////////////////////////////////////////////////////////////*/

  /**
   * Mark airdrop allocations as claimed.
   *
   * The claim PDA's existence *is* the claimed flag, so this needs no event decoding at all: fetch
   * every ClaimStatus account the program owns and mark the matching allocations. That is a cleaner
   * mechanism than the EVM equivalent, where the same fact lives in a bitmap that must be read
   * word by word.
   */
  private async syncClaims(): Promise<void> {
    const accounts = await this.fetchAccounts('ClaimStatus');

    for (const {data} of accounts) {
      try {
        let offset = 8;

        const claimant = new PublicKey(data.subarray(offset, offset + 32)).toBase58();
        offset += 32;
        offset += 8; // amount
        const claimedAt = data.readBigInt64LE(offset);

        await prisma.airdropEntry.updateMany({
          where: {claimant, claimedAt: null},
          data: {claimedAt: new Date(Number(claimedAt) * 1000)},
        });
      } catch (error) {
        logger.warn({error}, 'failed to decode claim status');
      }
    }
  }

  /*//////////////////////////////////////////////////////////////
                               SNAPSHOTS
  //////////////////////////////////////////////////////////////*/

  /** Sample pool APR and TVL. Neither emits anything, so sampling is the only way to chart them. */
  private async snapshotPools(): Promise<void> {
    const pools = await prisma.stakePool.findMany();
    if (pools.length === 0) return;

    const slot = await connection.getSlot('confirmed');

    for (const pool of pools) {
      const latest = await prisma.poolSnapshot.findFirst({
        where: {poolAddress: pool.address},
        orderBy: {capturedAt: 'desc'},
      });

      // At most one sample an hour, so a fast poll interval does not fill the table.
      if (latest && Date.now() - latest.capturedAt.getTime() < 60 * 60 * 1000) continue;

      await prisma.poolSnapshot.create({
        data: {
          poolAddress: pool.address,
          totalStaked: pool.totalStaked,
          aprBps: pool.aprBps,
          slot: BigInt(slot),
          capturedAt: new Date(),
        },
      });
    }
  }

  /**
   * Walk an account's transaction history.
   *
   * Kept for the history-dependent parts of the UI. Solana paginates by signature rather than by
   * block, so the cursor is the newest signature already seen and each pass fetches everything
   * after it.
   */
  async fetchNewSignatures(account: PublicKey): Promise<ConfirmedSignatureInfo[]> {
    const address = account.toBase58();

    const cursor = await prisma.indexerCursor.findUnique({where: {account: address}});

    const signatures = await connection.getSignaturesForAddress(account, {
      until: cursor?.lastSignature ?? undefined,
      limit: config.SIGNATURE_PAGE_SIZE,
    });

    if (signatures.length > 0) {
      await prisma.indexerCursor.upsert({
        where: {account: address},
        create: {
          id: address,
          account: address,
          lastSignature: signatures[0]!.signature,
          lastSlot: BigInt(signatures[0]!.slot),
        },
        update: {
          lastSignature: signatures[0]!.signature,
          lastSlot: BigInt(signatures[0]!.slot),
        },
      });
    }

    // Oldest first, so history applies in the order it happened.
    return signatures.reverse();
  }
}
