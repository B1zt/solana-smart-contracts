import 'dotenv/config';
import {PublicKey} from '@solana/web3.js';
import {z} from 'zod';

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

  PORT: z.coerce.number().int().positive().default(4005),
  HOST: z.string().default('0.0.0.0'),
  CORS_ORIGINS: z
    .string()
    .default('http://localhost:3000')
    .transform((value) => value.split(',').map((origin) => origin.trim())),

  DATABASE_URL: z.string().url(),

  CLUSTER: z.enum(['mainnet-beta', 'devnet', 'testnet', 'localnet']).default('devnet'),
  RPC_URL: z.string().url(),
  RPC_WS_URL: z.string().optional(),

  /**
   * DAS-capable endpoint, for reading compressed NFTs.
   *
   * The public Solana endpoints do not implement DAS. Falls back to RPC_URL, and the service
   * reports clearly when the method is missing rather than failing opaquely.
   */
  DAS_RPC_URL: z.string().url().optional(),

  PROGRAM_ID: pubkeySchema,

  INDEXER_POLL_INTERVAL: z.coerce.number().int().positive().default(10),
  SIGNATURE_PAGE_SIZE: z.coerce.number().int().positive().max(1_000).default(500),

  /** Required for the allowlist upload route: publishing a root decides who can mint. */
  ADMIN_API_KEY: z.string().min(16).optional(),
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
  dasUrl: parsed.data.DAS_RPC_URL ?? parsed.data.RPC_URL,
});

export type Config = typeof config;
