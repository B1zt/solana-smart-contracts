const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:4005/api/v1';

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

async function request<T>(path: string): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    headers: {'Content-Type': 'application/json'},
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

export interface MintPhase {
  address: string;
  index: number;
  merkleRoot: string;
  price: string;
  startTs: string;
  endTs: string;
  maxPerWallet: number;
  maxSupply: number;
  minted: number;
}

export interface Collection {
  address: string;
  authority: string;
  name: string;
  baseUri: string;
  maxSupply: number;
  minted: number;
  royaltyBps: number;
  floorPrice: string | null;
  volume: string;
  phases: MintPhase[];
}

export interface Listing {
  address: string;
  seller: string;
  mint: string;
  price: string;
  createdTs: string;
  name?: string | null;
  image?: string | null;
  compressed?: boolean;
}

export interface WalletAsset {
  id: string;
  name: string | null;
  image: string | null;
  compressed: boolean;
  collection: string | null;
  listing: Listing | null;
}

export interface AllowlistProof {
  wallet: string;
  allowance: number;
  proof: number[][];
  proofHex: string[];
  root: string;
}

export const api = {
  config: () =>
    request<{cluster: string; programId: string; dasSupported: boolean}>('/config'),

  collections: () => request<{collections: Collection[]}>('/collections'),

  collection: (address: string) =>
    request<{collection: Collection; listedCount: number; salesCount: number}>(
      `/collections/${address}`,
    ),

  listings: (address: string) =>
    request<{listings: Listing[]; nextCursor: string | null}>(`/collections/${address}/listings`),

  activity: (address: string) =>
    request<{
      sales: {
        id: string;
        mint: string;
        seller: string;
        buyer: string;
        price: string;
        blockTime: string;
      }[];
    }>(`/collections/${address}/activity`),

  /** Null when the phase is public rather than gated, which is a normal state. */
  allowlistProof: async (phase: string, wallet: string): Promise<AllowlistProof | null> => {
    try {
      return await request<AllowlistProof>(`/phases/${phase}/allowlist/${wallet}`);
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) return null;
      throw error;
    }
  },

  walletAssets: (owner: string) =>
    request<{owner: string; total: number; assets: WalletAsset[]}>(`/wallet/${owner}/assets`),

  asset: (mint: string) =>
    request<{
      mint: string;
      asset: {
        name: string | null;
        description: string | null;
        image: string | null;
        owner: string;
        compressed: boolean;
        royaltyBps: number | null;
      } | null;
      listing: Listing | null;
      offers: {address: string; buyer: string; amount: string; expiresTs: string}[];
      history: {id: string; seller: string; buyer: string; price: string; blockTime: string}[];
    }>(`/assets/${mint}`),

  compressionSavings: (count: number) =>
    request<{count: number; regularSol: number; compressedSol: number; savingsMultiple: number}>(
      `/compression/savings?count=${count}`,
    ),
};

/** Lamports to SOL, trimmed. */
export function formatSol(lamports: string | bigint | null | undefined): string {
  if (lamports === null || lamports === undefined) return '-';

  const value = typeof lamports === 'string' ? BigInt(lamports) : lamports;
  const sol = Number(value) / 1e9;

  if (sol === 0) return '0 SOL';
  if (sol < 0.001) return '<0.001 SOL';

  return `${sol.toFixed(sol >= 1 ? 2 : 3)} SOL`;
}
