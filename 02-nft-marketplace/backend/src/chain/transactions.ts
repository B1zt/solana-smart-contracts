import {
  ComputeBudgetProgram,
  Connection,
  TransactionExpiredBlockheightExceededError,
  TransactionMessage,
  VersionedTransaction,
  type Blockhash,
  type Keypair,
  type PublicKey,
  type TransactionInstruction,
} from '@solana/web3.js';
import {logger} from '../lib/logger.js';

export interface SendOptions {
  /** Compute units to request. Too low and the transaction fails; too high and it overpays. */
  computeUnitLimit?: number;
  /** Micro-lamports per compute unit. Resolved from recent fees when omitted. */
  computeUnitPrice?: number;
  /** How many times to rebuild and resend before giving up. */
  maxAttempts?: number;
  /** Commitment to wait for. */
  commitment?: 'processed' | 'confirmed' | 'finalized';
}

export interface SendResult {
  signature: string;
  slot: number;
  attempts: number;
  computeUnitPrice: number;
}

/**
 * Transaction submission that survives the two things that actually go wrong on Solana.
 *
 * **Blockhash expiry.** A Solana transaction is only valid for about 150 slots, roughly a minute.
 * Under load a transaction routinely fails to land inside that window, and the fix is not to resend
 * the same bytes: the blockhash is baked into what was signed, so an expired transaction can only
 * be rebuilt and re-signed. Code that retries the identical transaction will retry it into the
 * ground.
 *
 * **Priority fees.** Solana's base fee is fixed and trivially small, so under contention it does no
 * ordering work at all. Landing a transaction when the network is busy means attaching a compute
 * unit price, and the right price is whatever recent transactions touching the same accounts paid.
 * A hardcoded value is either wasteful or useless depending on the day.
 *
 * Both are handled by rebuilding from scratch on each attempt, with the fee re-estimated as it goes.
 */
export class TransactionSender {
  constructor(private readonly connection: Connection) {}

  /**
   * Estimate a compute unit price from recent fees on the accounts being touched.
   *
   * `getRecentPrioritizationFees` reports what recent transactions writing to these accounts paid.
   * Using the 75th percentile rather than the median buys meaningful reliability for a small extra
   * cost, and the whole thing is still fractions of a cent.
   */
  async estimatePriorityFee(writableAccounts: PublicKey[]): Promise<number> {
    try {
      const fees = await this.connection.getRecentPrioritizationFees({
        lockedWritableAccounts: writableAccounts.slice(0, 128),
      });

      const nonZero = fees.map((entry) => entry.prioritizationFee).filter((fee) => fee > 0);

      if (nonZero.length === 0) {
        // Nothing recent paid a priority fee, so the accounts are uncontended.
        return 1_000;
      }

      nonZero.sort((a, b) => a - b);
      const index = Math.floor(nonZero.length * 0.75);

      // Floor and ceiling: a floor because a fee of zero does nothing under any contention, and a
      // ceiling so one outlier in the sample cannot make every transaction absurdly expensive.
      return Math.min(Math.max(nonZero[index] ?? 1_000, 1_000), 1_000_000);
    } catch (error) {
      logger.debug({error}, 'priority fee estimate failed, using default');
      return 10_000;
    }
  }

  /**
   * Simulate to find the compute units actually needed.
   *
   * The default limit is 200,000 per instruction, and requesting far more than needed raises the
   * priority fee proportionally without helping. Simulating and adding headroom is both cheaper and
   * more reliable than guessing.
   */
  async estimateComputeUnits(
    instructions: TransactionInstruction[],
    payer: PublicKey,
    blockhash: Blockhash,
  ): Promise<number> {
    try {
      const message = new TransactionMessage({
        payerKey: payer,
        recentBlockhash: blockhash,
        // A high limit during simulation only, so the simulation itself is not what runs out.
        instructions: [
          ComputeBudgetProgram.setComputeUnitLimit({units: 1_400_000}),
          ...instructions,
        ],
      }).compileToV0Message();

      const simulation = await this.connection.simulateTransaction(
        new VersionedTransaction(message),
        {sigVerify: false, replaceRecentBlockhash: true},
      );

      if (simulation.value.err || !simulation.value.unitsConsumed) {
        return 200_000;
      }

      // 20% headroom: account state moves between simulation and execution, and running out of
      // compute wastes the whole fee.
      return Math.min(Math.ceil(simulation.value.unitsConsumed * 1.2), 1_400_000);
    } catch (error) {
      logger.debug({error}, 'compute estimate failed, using default');
      return 200_000;
    }
  }

  /**
   * Build, sign, send and confirm, rebuilding on each attempt.
   *
   * The rebuild is the important part. Resending identical bytes after a blockhash expires can
   * never succeed, because the expired blockhash is inside the signed payload.
   */
  async send(
    instructions: TransactionInstruction[],
    signers: Keypair[],
    options: SendOptions = {},
  ): Promise<SendResult> {
    const maxAttempts = options.maxAttempts ?? 3;
    const commitment = options.commitment ?? 'confirmed';
    const payer = signers[0];

    if (!payer) throw new Error('at least one signer is required');

    const writable = instructions.flatMap((instruction) =>
      instruction.keys.filter((key) => key.isWritable).map((key) => key.pubkey),
    );

    let lastError: unknown;

    for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
      try {
        // A fresh blockhash every attempt. This is what makes a retry meaningful.
        const {blockhash, lastValidBlockHeight} =
          await this.connection.getLatestBlockhash(commitment);

        const computeUnitPrice =
          options.computeUnitPrice ?? (await this.estimatePriorityFee(writable));

        const computeUnitLimit =
          options.computeUnitLimit ??
          (await this.estimateComputeUnits(instructions, payer.publicKey, blockhash));

        const message = new TransactionMessage({
          payerKey: payer.publicKey,
          recentBlockhash: blockhash,
          instructions: [
            ComputeBudgetProgram.setComputeUnitLimit({units: computeUnitLimit}),
            ComputeBudgetProgram.setComputeUnitPrice({microLamports: computeUnitPrice}),
            ...instructions,
          ],
        }).compileToV0Message();

        const transaction = new VersionedTransaction(message);
        transaction.sign(signers);

        const signature = await this.connection.sendTransaction(transaction, {
          // Preflight already ran during estimation, and skipping it saves a round trip on retries.
          skipPreflight: attempt > 1,
          maxRetries: 0,
        });

        const confirmation = await this.connection.confirmTransaction(
          {signature, blockhash, lastValidBlockHeight},
          commitment,
        );

        if (confirmation.value.err) {
          throw new Error(`transaction failed: ${JSON.stringify(confirmation.value.err)}`);
        }

        const status = await this.connection.getSignatureStatus(signature);

        logger.info(
          {signature, attempt, computeUnitPrice, computeUnitLimit},
          'transaction confirmed',
        );

        return {
          signature,
          slot: status.value?.slot ?? 0,
          attempts: attempt,
          computeUnitPrice,
        };
      } catch (error) {
        lastError = error;

        // An expired blockhash is the expected failure, not an exceptional one. It means the
        // network was busy, and the next attempt with a fresh blockhash and a higher fee is the
        // correct response rather than an error to surface.
        const expired = error instanceof TransactionExpiredBlockheightExceededError;

        logger.warn(
          {attempt, maxAttempts, expired, error: (error as Error).message},
          'transaction attempt failed',
        );

        if (attempt < maxAttempts) {
          // Brief backoff, so a congested moment is not hammered.
          await new Promise((resolve) => setTimeout(resolve, 500 * attempt));
        }
      }
    }

    throw new Error(
      `transaction failed after ${maxAttempts} attempts: ${(lastError as Error)?.message}`,
    );
  }
}
