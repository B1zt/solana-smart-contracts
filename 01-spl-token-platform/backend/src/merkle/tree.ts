// js-sha3 is CommonJS and builds its exports dynamically, so Node's named-export detection
// cannot see `keccak256`. A default import always gets module.exports, which does.
import sha3 from 'js-sha3';
import {PublicKey} from '@solana/web3.js';

const {keccak256} = sha3;

export interface AirdropEntry {
  index: number;
  claimant: PublicKey;
  amount: bigint;
}

/**
 * Merkle tree for the airdrop, matching `airdrop.rs` byte for byte.
 *
 * Four details have to line up with the on-chain verifier. Getting any of them wrong produces a
 * tree that looks correct here while every single claim fails on-chain with `InvalidProof`, and
 * nothing about the error points at the cause:
 *
 *   1. **Leaf encoding** is `keccak(keccak(index_le || claimant || amount_le))`. Numbers are
 *      little-endian because that is what Rust's `to_le_bytes` produces; using big-endian is the
 *      most common way to get this wrong when porting from an EVM codebase.
 *
 *   2. **Double hashing.** A single-hashed leaf is 32 bytes and so is an internal node, so an
 *      attacker could present a node lifted from someone's published proof as a leaf.
 *
 *   3. **Sorted pairs.** Siblings are ordered before hashing, which is why proofs carry no
 *      left/right flags.
 *
 *   4. **Odd nodes are promoted**, not duplicated. Duplicating is the other common convention and
 *      produces a different root.
 *
 * The index must stay dense and stable once a root is published: it is part of the leaf and part of
 * the claim PDA's seeds, so renumbering invalidates every proof and every claim record.
 */
export class AirdropTree {
  private readonly layers: Buffer[][];
  private readonly positionByIndex: Map<number, number>;
  private readonly entries: AirdropEntry[];

  constructor(entries: AirdropEntry[]) {
    if (entries.length === 0) {
      throw new Error('AirdropTree: cannot build a tree with no entries');
    }

    this.entries = [...entries].sort((a, b) => a.index - b.index);

    const seenIndex = new Set<number>();
    const seenClaimant = new Set<string>();

    for (const entry of this.entries) {
      if (!Number.isInteger(entry.index) || entry.index < 0) {
        throw new Error(`AirdropTree: invalid index ${entry.index}`);
      }
      if (seenIndex.has(entry.index)) {
        throw new Error(`AirdropTree: duplicate index ${entry.index}`);
      }

      const claimant = entry.claimant.toBase58();
      if (seenClaimant.has(claimant)) {
        // Two leaves for one wallet means two claim PDAs, so it could claim twice.
        throw new Error(`AirdropTree: duplicate claimant ${claimant}`);
      }

      seenIndex.add(entry.index);
      seenClaimant.add(claimant);
    }

    // Leaves are sorted by hash so the tree is deterministic: the same entry set always produces
    // the same root, and a rebuild does not invalidate proofs already issued.
    const decorated = this.entries
      .map((entry) => ({entry, leaf: AirdropTree.leafFor(entry)}))
      .sort((a, b) => a.leaf.compare(b.leaf));

    this.positionByIndex = new Map(decorated.map((item, position) => [item.entry.index, position]));

    this.layers = [decorated.map((item) => item.leaf)];

    while (this.layers[this.layers.length - 1]!.length > 1) {
      this.layers.push(AirdropTree.nextLayer(this.layers[this.layers.length - 1]!));
    }
  }

  /** `keccak(keccak(index_le || claimant || amount_le))`. */
  static leafFor(entry: AirdropEntry): Buffer {
    const payload = Buffer.concat([
      AirdropTree.u64le(BigInt(entry.index)),
      entry.claimant.toBuffer(),
      AirdropTree.u64le(entry.amount),
    ]);

    const inner = Buffer.from(keccak256.arrayBuffer(payload));
    return Buffer.from(keccak256.arrayBuffer(inner));
  }

  /** Little-endian u64, matching Rust's `to_le_bytes`. */
  private static u64le(value: bigint): Buffer {
    const buffer = Buffer.alloc(8);
    buffer.writeBigUInt64LE(value);
    return buffer;
  }

  get root(): Buffer {
    return this.layers[this.layers.length - 1]![0]!;
  }

  get rootHex(): string {
    return this.root.toString('hex');
  }

  get size(): number {
    return this.entries.length;
  }

  get totalAmount(): bigint {
    return this.entries.reduce((sum, entry) => sum + entry.amount, 0n);
  }

  /** Proof for an allocation index, or null if it is not in the tree. */
  proofFor(index: number): Buffer[] | null {
    const start = this.positionByIndex.get(index);
    if (start === undefined) return null;

    const proof: Buffer[] = [];
    let position = start;

    for (let level = 0; level < this.layers.length - 1; level += 1) {
      const layer = this.layers[level]!;
      const sibling = position ^ 1;

      // A promoted odd node has no sibling at this level and contributes nothing.
      if (sibling < layer.length) {
        proof.push(layer[sibling]!);
      }

      position = Math.floor(position / 2);
    }

    return proof;
  }

  /** Local verification, used by the tests and as a self-check before publishing a root. */
  verify(entry: AirdropEntry, proof: Buffer[]): boolean {
    let computed = AirdropTree.leafFor(entry);

    for (const sibling of proof) {
      computed = AirdropTree.hashPair(computed, sibling);
    }

    return computed.equals(this.root);
  }

  private static nextLayer(layer: Buffer[]): Buffer[] {
    const next: Buffer[] = [];

    for (let i = 0; i < layer.length; i += 2) {
      const left = layer[i]!;
      const right = layer[i + 1];
      next.push(right === undefined ? left : AirdropTree.hashPair(left, right));
    }

    return next;
  }

  /** Sorted-pair keccak, matching `verify_proof` on-chain. */
  private static hashPair(a: Buffer, b: Buffer): Buffer {
    const [first, second] = a.compare(b) <= 0 ? [a, b] : [b, a];
    return Buffer.from(keccak256.arrayBuffer(Buffer.concat([first, second])));
  }
}
