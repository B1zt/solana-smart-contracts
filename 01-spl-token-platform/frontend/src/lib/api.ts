const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:4004/api/v1';

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly body: unknown,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: {'Content-Type': 'application/json', ...init?.headers},
  });

  const body: unknown = await response.json().catch(() => null);

  if (!response.ok) {
    const message =
      body && typeof body === 'object' && 'error' in body
        ? String((body as {error: unknown}).error)
        : `Request failed with ${response.status}`;
    throw new ApiError(message, response.status, body);
  }

  return body as T;
}

export interface ClaimData {
  index: number;
  claimant: string;
  amount: string;
  /** 32-byte arrays, the shape the instruction takes. */
  proof: number[][];
  proofHex: string[];
  root: string;
  claimStatusPda: string;
  claimed: boolean;
  claimedAt: string | null;
}

export interface VestingSchedule {
  address: string;
  beneficiary: string;
  mint: string;
  vault: string;
  totalAmount: string;
  releasedAmount: string;
  vestedAmount: string;
  releasableAmount: string;
  startTs: string;
  cliffSeconds: number;
  durationSeconds: number;
  revocable: boolean;
  revoked: boolean;
  seed: string;
  releases: {id: string; amount: string; blockTime: string; signature: string}[];
}

export interface StakePool {
  address: string;
  stakeMint: string;
  rewardMint: string;
  stakeVault: string;
  rewardVault: string;
  totalStaked: string;
  rewardRate: string;
  rewardEndTs: string;
  cooldownSeconds: number;
  aprBps: number;
}

export interface StakePosition {
  address: string;
  poolAddress: string;
  owner: string;
  amount: string;
  pendingReward: string;
  unstakingAmount: string;
  unstakeReadyTs: string | null;
}

export const api = {
  config: () =>
    request<{cluster: string; programId: string; rpcUrl?: string}>('/config'),

  chainStatus: () =>
    request<{cluster: string; slot: number; blockTime: number | null; programId: string}>(
      '/chain/status',
    ),

  /** Priority fee guidance. Solana's base fee does no ordering work under contention. */
  priorityFee: (accounts?: string[]) =>
    request<{microLamportsPerComputeUnit: number; estimatedCostLamports: number}>(
      `/chain/priority-fee${accounts?.length ? `?accounts=${accounts.join(',')}` : ''}`,
    ),

  /** Returns null when the wallet has no allocation, which the UI shows as ineligible. */
  claim: async (distributor: string, claimant: string): Promise<ClaimData | null> => {
    try {
      return await request<ClaimData>(`/airdrop/${distributor}/claim/${claimant}`);
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) return null;
      throw error;
    }
  },

  airdropStats: (distributor: string) =>
    request<{
      distributor: string;
      entryCount: number;
      claimedCount: number;
      totalAmount: string;
      root: string | null;
      deadline: string | null;
    }>(`/airdrop/${distributor}/stats`),

  vesting: (beneficiary: string) =>
    request<{beneficiary: string; schedules: VestingSchedule[]}>(`/vesting/${beneficiary}`),

  vestingCurve: (address: string) =>
    request<{address: string; total: string; points: {timestamp: number; vested: string}[]}>(
      `/vesting/schedule/${address}/curve`,
    ),

  pools: () => request<{pools: StakePool[]}>('/staking/pools'),

  pool: (address: string) =>
    request<{
      pool: StakePool & {snapshots: {aprBps: number; totalStaked: string; capturedAt: string}[]};
      stakerCount: number;
    }>(`/staking/pools/${address}`),

  position: async (pool: string, owner: string): Promise<StakePosition | null> => {
    try {
      const {position} = await request<{position: StakePosition}>(
        `/staking/position/${pool}/${owner}`,
      );
      return position;
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) return null;
      throw error;
    }
  },
};

/** Format a raw token amount for display, given its decimals. */
export function formatAmount(raw: string | bigint, decimals = 9): string {
  const value = typeof raw === 'string' ? BigInt(raw) : raw;
  if (value === 0n) return '0';

  const divisor = 10n ** BigInt(decimals);
  const whole = value / divisor;
  const fraction = value % divisor;

  if (fraction === 0n) return whole.toLocaleString();

  const fractionText = fraction.toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${whole.toLocaleString()}.${fractionText.slice(0, 4)}`;
}

/** Parse a user-typed amount into base units. Returns null rather than throwing on partial input. */
export function parseAmount(input: string, decimals = 9): bigint | null {
  const trimmed = input.trim();
  if (!trimmed || !/^\d*\.?\d*$/.test(trimmed)) return null;

  const [whole = '0', fraction = ''] = trimmed.split('.');
  if (fraction.length > decimals) return null;

  try {
    return BigInt(whole || '0') * 10n ** BigInt(decimals) + BigInt(fraction.padEnd(decimals, '0') || '0');
  } catch {
    return null;
  }
}

/** `abcd…wxyz`, the standard truncation for Solana addresses. */
export function shortAddress(address: string): string {
  if (address.length < 12) return address;
  return `${address.slice(0, 4)}…${address.slice(-4)}`;
}

/** `2d 4h`, `3h 12m`, `45s`. Null once the deadline has passed. */
export function formatCountdown(target: Date | string | number): string | null {
  const remaining = new Date(target).getTime() - Date.now();
  if (remaining <= 0) return null;

  const seconds = Math.floor(remaining / 1000);
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;

  return `${seconds}s`;
}
