import 'dotenv/config';
import {PublicKey} from '@solana/web3.js';
import {z} from 'zod';

/**
 * Treat an empty variable as absent.
 *
 * An unset variable in a .env file arrives as an empty string, not as undefined, so `.optional()`
 * on its own rejects the shipped .env.example and refuses to start over settings the operator
 * deliberately left blank. Every optional value below goes through this.
 */
function optional<T extends z.ZodTypeAny>(schema: T) {
  return z.preprocess((value) => (value === '' ? undefined : value), schema.optional());
}


/** A base58 Solana address, validated by actually constructing a PublicKey. */
const pubkeySchema = z.string().refine(
  (value) => {
    try {
      new PublicKey(value);
      return true;
    } catch {
      return false;
    }
  },
  {message: 'must be a base58 Solana address'},
);

const envSchema = z.object({
  NODE_ENV: z.enum(['development', 'test', 'production']).default('development'),
  LOG_LEVEL: z.enum(['fatal', 'error', 'warn', 'info', 'debug', 'trace']).default('info'),

  PORT: z.coerce.number().int().positive().default(4004),
  HOST: z.string().default('0.0.0.0'),
  CORS_ORIGINS: z
    .string()
    .default('http://localhost:3000')
    .transform((value) => value.split(',').map((origin) => origin.trim())),

  DATABASE_URL: z.string().url(),

  /** mainnet-beta, devnet, testnet or localnet. */
  CLUSTER: z.enum(['mainnet-beta', 'devnet', 'testnet', 'localnet']).default('devnet'),
  RPC_URL: z.string().url(),

  /**
   * Separate endpoint for websocket subscriptions.
   *
   * Many providers serve HTTP and websockets on different hosts, and web3.js does not infer one
   * from the other reliably.
   */
  RPC_WS_URL: optional(z.string()),

  PROGRAM_ID: pubkeySchema,

  /** How often the indexer polls, in seconds. Solana slots are ~400ms, so this is generous. */
  INDEXER_POLL_INTERVAL: z.coerce.number().int().positive().default(10),

  /** Signatures to fetch per backfill page. The RPC caps this at 1,000. */
  SIGNATURE_PAGE_SIZE: z.coerce.number().int().positive().max(1_000).default(500),

  /**
   * Shared secret for the airdrop admin endpoints.
   *
   * Publishing a Merkle root decides who receives tokens, so those routes are not open.
   */
  /**
   * An unset variable in a .env file arrives as an empty string, not as undefined, so `.optional()`
   * alone would reject the shipped .env.example and fail startup with a length complaint about a
   * key the operator never set. Empty is normalised to absent first.
   */
  ADMIN_API_KEY: z.preprocess(
    (value) => (value === '' ? undefined : value),
    z.string().min(16).optional(),
  ),
  /**
   * Whether this process runs the indexer.
   *
   * Off lets the API serve seeded or already-indexed data without a chain connection, and lets the
   * indexer run as its own process (`pnpm indexer`) without the API double-indexing behind it.
   */
  INDEXER_ENABLED: z
    .enum(['true', 'false'])
    .default('true')
    .transform((value) => value === 'true'),

});

const parsed = envSchema.safeParse(process.env);

if (!parsed.success) {
  const issues = parsed.error.issues
    .map((issue) => `  ${issue.path.join('.') || '(root)'}: ${issue.message}`)
    .join('\n');
  throw new Error(`Invalid environment configuration:\n${issues}`);
}

export const config = Object.freeze({
  ...parsed.data,
  programId: new PublicKey(parsed.data.PROGRAM_ID),
});

export type Config = typeof config;
