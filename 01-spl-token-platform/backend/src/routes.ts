import {PublicKey} from '@solana/web3.js';
import type {FastifyInstance, FastifyRequest} from 'fastify';
import {z} from 'zod';
import {connection, currentSlot} from './chain/connection.js';
import {TransactionSender} from './chain/transactions.js';
import {config} from './config.js';
import {prisma} from './lib/prisma.js';
import {AirdropTree, type AirdropEntry} from './merkle/tree.js';

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

/** Rebuild the tree from stored entries. A tree is a pure function of them, so it is never stored. */
async function loadTree(distributor: string): Promise<AirdropTree | null> {
  const rows = await prisma.airdropEntry.findMany({
    where: {distributor},
    orderBy: {index: 'asc'},
  });

  if (rows.length === 0) return null;

  const entries: AirdropEntry[] = rows.map((row) => ({
    index: row.index,
    claimant: new PublicKey(row.claimant),
    amount: BigInt(row.amount),
  }));

  return new AirdropTree(entries);
}

export async function apiRoutes(app: FastifyInstance): Promise<void> {
  const sender = new TransactionSender(connection);

  app.get('/config', async (_request, reply) =>
    reply.send({
      cluster: config.CLUSTER,
      programId: config.programId.toBase58(),
      rpcUrl: config.CLUSTER === 'localnet' ? config.RPC_URL : undefined,
    }),
  );

  /*//////////////////////////////////////////////////////////////
                                AIRDROP
  //////////////////////////////////////////////////////////////*/

  /**
   * Build an allocation list and return the root to publish on-chain.
   *
   * Privileged: this decides who receives tokens.
   */
  app.post('/airdrop/build', async (request, reply) => {
    if (!requireAdmin(request)) {
      return reply.status(401).send({error: 'UNAUTHORIZED'});
    }

    const schema = z.object({
      distributor: pubkeyString,
      deadline: z.coerce.number().int().positive(),
      entries: z
        .array(z.object({claimant: pubkeyString, amount: z.string().regex(/^\d+$/)}))
        .min(1)
        .max(100_000),
    });

    const parsed = schema.safeParse(request.body);
    if (!parsed.success) {
      return reply.status(400).send({error: 'INVALID_BODY', issues: parsed.error.issues});
    }

    // Indices are assigned here, densely from zero. They seed the on-chain claim PDA, so they must
    // never be reassigned once a root is published.
    const entries: AirdropEntry[] = parsed.data.entries.map((entry, index) => ({
      index,
      claimant: new PublicKey(entry.claimant),
      amount: BigInt(entry.amount),
    }));

    let tree: AirdropTree;
    try {
      tree = new AirdropTree(entries);
    } catch (error) {
      return reply.status(400).send({error: 'INVALID_ENTRIES', message: (error as Error).message});
    }

    const distributor = parsed.data.distributor;

    await prisma.$transaction(async (tx) => {
      // Rebuilding replaces the list wholesale. Merging would leave stale entries that no longer
      // match the published root.
      await tx.airdropEntry.deleteMany({where: {distributor}});
      await tx.airdropEntry.createMany({
        data: entries.map((entry) => ({
          distributor,
          index: entry.index,
          claimant: entry.claimant.toBase58(),
          amount: entry.amount.toString(),
        })),
      });

      await tx.airdropRoot.upsert({
        where: {distributor},
        create: {
          distributor,
          root: tree.rootHex,
          entryCount: tree.size,
          totalAmount: tree.totalAmount.toString(),
          deadline: new Date(parsed.data.deadline * 1000),
        },
        update: {
          root: tree.rootHex,
          entryCount: tree.size,
          totalAmount: tree.totalAmount.toString(),
        },
      });
    });

    return reply.status(201).send({
      // The instruction takes the root as a 32-byte array, so both encodings are returned.
      root: tree.rootHex,
      rootBytes: Array.from(tree.root),
      entryCount: tree.size,
      totalAmount: tree.totalAmount.toString(),
      nextStep: 'Pass rootBytes to initialize_distributor, then fund the vault with totalAmount',
    });
  });

  /**
   * Claim data for one wallet.
   *
   * A 404 means "no allocation", which the UI renders as ineligible rather than an error. The
   * on-chain claim-status PDA is authoritative over the indexed `claimedAt`, since the indexer can
   * lag by a few slots.
   */
  app.get('/airdrop/:distributor/claim/:claimant', async (request, reply) => {
    const params = request.params as {distributor: string; claimant: string};

    const entry = await prisma.airdropEntry.findUnique({
      where: {
        distributor_claimant: {distributor: params.distributor, claimant: params.claimant},
      },
    });

    if (!entry) {
      return reply.status(404).send({error: 'NOT_ELIGIBLE'});
    }

    const tree = await loadTree(params.distributor);
    if (!tree) {
      return reply.status(404).send({error: 'NO_AIRDROP_CONFIGURED'});
    }

    const proof = tree.proofFor(entry.index);
    if (!proof) {
      return reply.status(404).send({error: 'NOT_ELIGIBLE'});
    }

    // The claim PDA's existence is the on-chain claimed flag.
    const [claimStatus] = PublicKey.findProgramAddressSync(
      [
        Buffer.from('claim'),
        new PublicKey(params.distributor).toBuffer(),
        (() => {
          const buffer = Buffer.alloc(8);
          buffer.writeBigUInt64LE(BigInt(entry.index));
          return buffer;
        })(),
      ],
      config.programId,
    );

    const claimAccount = await connection.getAccountInfo(claimStatus);

    return reply.send({
      index: entry.index,
      claimant: entry.claimant,
      amount: entry.amount,
      // The instruction takes proof nodes as 32-byte arrays.
      proof: proof.map((node) => Array.from(node)),
      proofHex: proof.map((node) => node.toString('hex')),
      root: tree.rootHex,
      claimStatusPda: claimStatus.toBase58(),
      claimed: claimAccount !== null,
      claimedAt: entry.claimedAt,
    });
  });

  app.get('/airdrop/:distributor/stats', async (request, reply) => {
    const {distributor} = request.params as {distributor: string};

    const [entryCount, claimedCount, root] = await Promise.all([
      prisma.airdropEntry.count({where: {distributor}}),
      prisma.airdropEntry.count({where: {distributor, claimedAt: {not: null}}}),
      prisma.airdropRoot.findUnique({where: {distributor}}),
    ]);

    return reply.send({
      distributor,
      entryCount,
      claimedCount,
      totalAmount: root?.totalAmount ?? '0',
      root: root?.root ?? null,
      deadline: root?.deadline ?? null,
    });
  });

  /*//////////////////////////////////////////////////////////////
                                VESTING
  //////////////////////////////////////////////////////////////*/

  app.get('/vesting/:beneficiary', async (request, reply) => {
    const {beneficiary} = request.params as {beneficiary: string};

    const schedules = await prisma.vestingSchedule.findMany({
      where: {beneficiary},
      include: {releases: {orderBy: {blockTime: 'desc'}}},
      orderBy: {startTs: 'asc'},
    });

    const now = Math.floor(Date.now() / 1000);

    return reply.send({
      beneficiary,
      schedules: schedules.map((schedule) => {
        const start = Math.floor(schedule.startTs.getTime() / 1000);
        const total = BigInt(schedule.totalAmount);
        const released = BigInt(schedule.releasedAmount);

        // Mirrors `VestingSchedule::vested_amount`. Recomputed here rather than read per schedule
        // because a wallet may hold many, and one RPC call each would be slow for no benefit; the
        // claim transaction itself is still validated on-chain.
        let vested: bigint;
        if (schedule.revoked) {
          vested = total;
        } else if (now < start + schedule.cliffSeconds) {
          vested = 0n;
        } else if (now >= start + schedule.durationSeconds) {
          vested = total;
        } else {
          vested = (total * BigInt(now - start)) / BigInt(schedule.durationSeconds);
        }

        return {
          ...schedule,
          vestedAmount: vested.toString(),
          releasableAmount: (vested > released ? vested - released : 0n).toString(),
        };
      }),
    });
  });

  /** Unlock curve for a schedule, sampled for charting. */
  app.get('/vesting/schedule/:address/curve', async (request, reply) => {
    const {address} = request.params as {address: string};

    const schedule = await prisma.vestingSchedule.findUnique({where: {address}});
    if (!schedule) {
      return reply.status(404).send({error: 'NOT_FOUND'});
    }

    const start = Math.floor(schedule.startTs.getTime() / 1000);
    const end = start + schedule.durationSeconds;
    const total = BigInt(schedule.totalAmount);
    const POINTS = 60;

    const points = Array.from({length: POINTS + 1}, (_unused, i) => {
      const timestamp = start + Math.floor(((end - start) * i) / POINTS);

      let vested: bigint;
      if (timestamp < start + schedule.cliffSeconds) {
        vested = 0n;
      } else if (timestamp >= end) {
        vested = total;
      } else {
        vested = (total * BigInt(timestamp - start)) / BigInt(schedule.durationSeconds);
      }

      return {timestamp, vested: vested.toString()};
    });

    return reply.send({address, total: schedule.totalAmount, points});
  });

  /*//////////////////////////////////////////////////////////////
                                STAKING
  //////////////////////////////////////////////////////////////*/

  app.get('/staking/pools', async (_request, reply) => {
    const pools = await prisma.stakePool.findMany({orderBy: {aprBps: 'desc'}});
    return reply.send({pools});
  });

  app.get('/staking/pools/:address', async (request, reply) => {
    const {address} = request.params as {address: string};

    const pool = await prisma.stakePool.findUnique({
      where: {address},
      include: {snapshots: {orderBy: {capturedAt: 'asc'}, take: 500}},
    });

    if (!pool) {
      return reply.status(404).send({error: 'NOT_FOUND'});
    }

    const stakerCount = await prisma.stakePosition.count({
      where: {poolAddress: address, amount: {not: '0'}},
    });

    return reply.send({pool, stakerCount});
  });

  app.get('/staking/position/:pool/:owner', async (request, reply) => {
    const params = request.params as {pool: string; owner: string};

    const position = await prisma.stakePosition.findUnique({
      where: {poolAddress_owner: {poolAddress: params.pool, owner: params.owner}},
    });

    if (!position) {
      return reply.status(404).send({error: 'NOT_FOUND'});
    }

    return reply.send({position});
  });

  /*//////////////////////////////////////////////////////////////
                              CHAIN HEALTH
  //////////////////////////////////////////////////////////////*/

  /**
   * Current priority fee guidance.
   *
   * Solana's base fee does no ordering work under contention, so a wallet that does not attach a
   * compute unit price simply will not land when the network is busy. Exposing the estimate lets
   * the frontend attach a sensible one instead of guessing.
   */
  app.get('/chain/priority-fee', async (request, reply) => {
    const schema = z.object({accounts: z.string().optional()});
    const parsed = schema.safeParse(request.query);

    const accounts = (parsed.success && parsed.data.accounts ? parsed.data.accounts.split(',') : [])
      .filter((value) => {
        try {
          new PublicKey(value.trim());
          return true;
        } catch {
          return false;
        }
      })
      .map((value) => new PublicKey(value.trim()));

    const microLamports = await sender.estimatePriorityFee(
      accounts.length > 0 ? accounts : [config.programId],
    );

    return reply.send({
      microLamportsPerComputeUnit: microLamports,
      // What a typical 200k CU transaction would pay on top of the base fee.
      estimatedCostLamports: Math.ceil((microLamports * 200_000) / 1_000_000),
    });
  });

  app.get('/chain/status', async (_request, reply) => {
    const slot = await currentSlot();
    const blockTime = await connection.getBlockTime(slot).catch(() => null);

    return reply.send({
      cluster: config.CLUSTER,
      slot,
      blockTime,
      programId: config.programId.toBase58(),
    });
  });
}
