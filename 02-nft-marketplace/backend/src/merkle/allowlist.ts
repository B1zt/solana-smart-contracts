// js-sha3 is CommonJS and builds its exports dynamically, so Node's named-export detection
// cannot see `keccak256`. A default import always gets module.exports, which does.
import sha3 from 'js-sha3';
import {PublicKey} from '@solana/web3.js';

const {keccak256} = sha3;

export interface AllowlistEntry {
  wallet: PublicKey;
  allowance: number;
}

/**
 * Merkle allowlist for a mint phase, matching `minting.rs` exactly.
 *
 * Leaves are `keccak(keccak(wallet || allowance_le))`, where `allowance` is a **u16 little-endian**.
 * Both details are load-bearing: the width because a u32 would produce different bytes, and the
 * endianness because Rust writes little-endian while EVM codebases write big-endian.
 *
 * Pairs are sorted before hashing and odd nodes are promoted rather than duplicated, both matching
 * `verify_proof` on-chain.
 */
export class Allowlist {
  private readonly layers: Buffer[][];
  private readonly positionByWallet: Map<string, number>;
  private readonly entries: AllowlistEntry[];

  constructor(entries: AllowlistEntry[]) {
    if (entries.length === 0) {
      throw new Error('Allowlist: cannot build a tree with no entries');
    }

    const seen = new Set<string>();
    for (const entry of entries) {
      const wallet = entry.wallet.toBase58();
      if (seen.has(wallet)) {
        throw new Error(`Allowlist: duplicate wallet ${wallet}`);
      }
      if (!Number.isInteger(entry.allowance) || entry.allowance <= 0 || entry.allowance > 65_535) {
        throw new Error(`Allowlist: allowance must fit in a u16, got ${entry.allowance}`);
      }
      seen.add(wallet);
    }

    this.entries = entries;

    // Sorted by leaf so the tree is deterministic and a rebuild does not invalidate issued proofs.
    const decorated = entries
      .map((entry) => ({entry, leaf: Allowlist.leafFor(entry)}))
      .sort((a, b) => a.leaf.compare(b.leaf));

    this.positionByWallet = new Map(
      decorated.map((item, index) => [item.entry.wallet.toBase58(), index]),
    );

    this.layers = [decorated.map((item) => item.leaf)];

    while (this.layers[this.layers.length - 1]!.length > 1) {
      this.layers.push(Allowlist.nextLayer(this.layers[this.layers.length - 1]!));
    }
  }

  /** `keccak(keccak(wallet || allowance_u16_le))`. */
  static leafFor(entry: AllowlistEntry): Buffer {
    const allowance = Buffer.alloc(2);
    allowance.writeUInt16LE(entry.allowance);

    const inner = Buffer.from(
      keccak256.arrayBuffer(Buffer.concat([entry.wallet.toBuffer(), allowance])),
    );

    return Buffer.from(keccak256.arrayBuffer(inner));
  }

  get root(): Buffer {
    return this.layers[this.layers.length - 1]![0]!;
  }

  get rootHex(): string {
    return this.root.toString('hex');
  }

  /** The instruction takes the root as a 32-byte array. */
  get rootBytes(): number[] {
    return Array.from(this.root);
  }

  get size(): number {
    return this.entries.length;
  }

  proofFor(wallet: string): Buffer[] | null {
    const start = this.positionByWallet.get(wallet);
    if (start === undefined) return null;

    const proof: Buffer[] = [];
    let position = start;

    for (let level = 0; level < this.layers.length - 1; level += 1) {
      const layer = this.layers[level]!;
      const sibling = position ^ 1;

      if (sibling < layer.length) proof.push(layer[sibling]!);
      position = Math.floor(position / 2);
    }

    return proof;
  }

  verify(entry: AllowlistEntry, proof: Buffer[]): boolean {
    let computed = Allowlist.leafFor(entry);

    for (const sibling of proof) {
      computed = Allowlist.hashPair(computed, sibling);
    }

    return computed.equals(this.root);
  }

  private static nextLayer(layer: Buffer[]): Buffer[] {
    const next: Buffer[] = [];

    for (let i = 0; i < layer.length; i += 2) {
      const left = layer[i]!;
      const right = layer[i + 1];
      next.push(right === undefined ? left : Allowlist.hashPair(left, right));
    }

    return next;
  }

  private static hashPair(a: Buffer, b: Buffer): Buffer {
    const [first, second] = a.compare(b) <= 0 ? [a, b] : [b, a];
    return Buffer.from(keccak256.arrayBuffer(Buffer.concat([first, second])));
  }
}
