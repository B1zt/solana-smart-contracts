import {logger} from '../lib/logger.js';

/**
 * Digital Asset Standard (DAS) client.
 *
 * **Why this exists rather than a token-account scan.** The obvious way to answer "what NFTs does
 * this wallet hold" is to enumerate its token accounts and keep the ones with supply one. That
 * works, but it misses compressed NFTs entirely, and compressed NFTs are the reason anyone chooses
 * Solana for a large collection in the first place.
 *
 * A compressed NFT does not have an account. It is a leaf in a Merkle tree stored in a single
 * concurrent-merkle-tree account, with only the root on-chain. Minting a million of them costs a few
 * SOL instead of roughly ten thousand, which is the entire pitch. The trade-off is that the asset
 * data lives off-chain in an indexer, and DAS is the standard RPC extension for querying it.
 *
 * DAS is served by Helius, Triton and QuickNode among others. A plain `api.mainnet-beta.solana.com`
 * endpoint does **not** implement it, which is the most common surprise when wiring this up, so a
 * missing-method error is reported clearly rather than as a generic RPC failure.
 */
export interface DasAsset {
  id: string;
  interface: string;
  ownership: {owner: string; delegate?: string; frozen: boolean};
  content?: {
    json_uri?: string;
    metadata?: {name?: string; symbol?: string; description?: string};
    files?: {uri?: string; mime?: string}[];
  };
  grouping?: {group_key: string; group_value: string}[];
  royalty?: {basis_points?: number; primary_sale_happened?: boolean};
  compression?: {
    compressed: boolean;
    tree?: string;
    leaf_id?: number;
    data_hash?: string;
    creator_hash?: string;
  };
  burnt?: boolean;
}

export interface DasPage<T> {
  total: number;
  limit: number;
  page: number;
  items: T[];
}

export class DasUnsupportedError extends Error {
  constructor(endpoint: string) {
    super(
      `The RPC at ${endpoint} does not implement the Digital Asset Standard. ` +
        'Use a DAS-capable provider (Helius, Triton, QuickNode); the public Solana endpoints do not support it.',
    );
    this.name = 'DasUnsupportedError';
  }
}

export class DasClient {
  private requestId = 0;

  constructor(private readonly endpoint: string) {}

  private async call<T>(method: string, params: unknown): Promise<T> {
    const response = await fetch(this.endpoint, {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: `das-${++this.requestId}`,
        method,
        params,
      }),
    });

    if (!response.ok) {
      throw new Error(`DAS request failed with ${response.status}`);
    }

    const body = (await response.json()) as {
      result?: T;
      error?: {code: number; message: string};
    };

    if (body.error) {
      // -32601 is JSON-RPC "method not found", which here means the endpoint is not DAS-capable.
      // Saying so plainly saves a long debugging session.
      if (body.error.code === -32601) {
        throw new DasUnsupportedError(this.endpoint);
      }
      throw new Error(`DAS error ${body.error.code}: ${body.error.message}`);
    }

    if (body.result === undefined) {
      throw new Error('DAS returned no result');
    }

    return body.result;
  }

  /** Every asset a wallet owns, compressed or not. */
  async getAssetsByOwner(owner: string, page = 1, limit = 100): Promise<DasPage<DasAsset>> {
    return this.call<DasPage<DasAsset>>('getAssetsByOwner', {
      ownerAddress: owner,
      page,
      limit,
      displayOptions: {showUnverifiedCollections: false, showCollectionMetadata: true},
    });
  }

  /** Every asset in a collection. */
  async getAssetsByGroup(collection: string, page = 1, limit = 100): Promise<DasPage<DasAsset>> {
    return this.call<DasPage<DasAsset>>('getAssetsByGroup', {
      groupKey: 'collection',
      groupValue: collection,
      page,
      limit,
    });
  }

  async getAsset(id: string): Promise<DasAsset> {
    return this.call<DasAsset>('getAsset', {id});
  }

  /**
   * Merkle proof for a compressed NFT.
   *
   * Transferring a compressed NFT requires proving it is in the tree, so every transfer instruction
   * carries a proof fetched from an indexer. This is the concrete cost of compression: the asset is
   * cheap to mint and slightly more involved to move.
   */
  async getAssetProof(id: string): Promise<{root: string; proof: string[]; leaf: string; tree_id: string}> {
    return this.call('getAssetProof', {id});
  }

  /** Whether the configured endpoint supports DAS at all. */
  async isSupported(): Promise<boolean> {
    try {
      await this.call('getAsset', {id: 'So11111111111111111111111111111111111111112'});
      return true;
    } catch (error) {
      if (error instanceof DasUnsupportedError) return false;
      // Any other error means the method exists but the argument was rejected, which still tells
      // us DAS is implemented.
      return true;
    }
  }
}

/**
 * Cost comparison between regular and compressed NFTs.
 *
 * Exposed through the API because it is the single most persuasive argument for choosing Solana for
 * a large collection, and a number is more convincing than a claim.
 */
export function compressionSavings(count: number): {
  regularSol: number;
  compressedSol: number;
  savingsMultiple: number;
} {
  // A regular NFT needs a mint account, a token account and a metadata account, which together come
  // to roughly 0.012 SOL of rent, plus transaction fees.
  const regularSol = count * 0.012;

  // A compressed NFT pays only for a share of one concurrent Merkle tree account. A depth-20 tree
  // holds about a million assets for roughly 8 SOL.
  const treeCost = 8;
  const treeCapacity = 1_048_576;
  const compressedSol = Math.max((count / treeCapacity) * treeCost, 0.000_001);

  logger.debug({count, regularSol, compressedSol}, 'compression cost comparison');

  return {
    regularSol,
    compressedSol,
    savingsMultiple: regularSol / compressedSol,
  };
}
