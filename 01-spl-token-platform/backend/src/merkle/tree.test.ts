import {PublicKey} from '@solana/web3.js';
import {describe, expect, it} from 'vitest';
import {AirdropTree, type AirdropEntry} from './tree.js';

/**
 * Fixed entry set shared with the Rust suite.
 *
 * `programs/token_platform/tests/cross_check.rs` builds a tree from these exact entries using the
 * on-chain leaf and pair hashing, and asserts the same root constant.
 *
 * This matters more on Solana than it did on EVM. Rust's `to_le_bytes` is little-endian while every
 * EVM codebase is big-endian, so a tree ported across without noticing produces well-formed proofs
 * that fail every claim, and the on-chain error says only `InvalidProof`.
 */
const SHARED_FIXTURE: AirdropEntry[] = [
  {index: 0, claimant: new PublicKey('11111111111111111111111111111112'), amount: 1_000_000_000n},
  {index: 1, claimant: new PublicKey('11111111111111111111111111111113'), amount: 2_000_000_000n},
  {index: 2, claimant: new PublicKey('11111111111111111111111111111114'), amount: 3_000_000_000n},
  {index: 3, claimant: new PublicKey('11111111111111111111111111111115'), amount: 5_000_000_000n},
  {index: 4, claimant: new PublicKey('11111111111111111111111111111116'), amount: 8_000_000_000n},
];

/** Regenerate with `pnpm vitest run -t 'prints the shared fixture root'` if the fixture changes. */
const SHARED_FIXTURE_ROOT = 'd05181965dd81581e12198d2ed0c84fd085d6efffc7c9c0045f8ff93c686c0b8';

describe('AirdropTree', () => {
  it('produces a verifiable proof for every entry', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);

    for (const entry of SHARED_FIXTURE) {
      const proof = tree.proofFor(entry.index);
      expect(proof, `no proof for index ${entry.index}`).not.toBeNull();
      expect(tree.verify(entry, proof!)).toBe(true);
    }
  });

  it('rejects an inflated amount', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);
    const entry = SHARED_FIXTURE[0]!;
    const proof = tree.proofFor(entry.index)!;

    expect(tree.verify({...entry, amount: entry.amount * 1_000n}, proof)).toBe(false);
  });

  it('rejects a substituted claimant', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);
    const victim = SHARED_FIXTURE[0]!;
    const attacker = SHARED_FIXTURE[1]!;
    const proof = tree.proofFor(victim.index)!;

    expect(tree.verify({...victim, claimant: attacker.claimant}, proof)).toBe(false);
  });

  /** The index is part of the leaf and part of the claim PDA's seeds. */
  it('rejects a reused index', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);
    const entry = SHARED_FIXTURE[2]!;
    const proof = tree.proofFor(entry.index)!;

    expect(tree.verify({...entry, index: 4}, proof)).toBe(false);
  });

  it('rejects duplicate indices', () => {
    expect(
      () => new AirdropTree([SHARED_FIXTURE[0]!, {...SHARED_FIXTURE[1]!, index: 0}]),
    ).toThrow(/duplicate index/);
  });

  /** Two leaves for one wallet means two claim PDAs, so it could claim twice. */
  it('rejects duplicate claimants', () => {
    expect(
      () =>
        new AirdropTree([
          SHARED_FIXTURE[0]!,
          {...SHARED_FIXTURE[1]!, claimant: SHARED_FIXTURE[0]!.claimant},
        ]),
    ).toThrow(/duplicate claimant/);
  });

  it('is independent of input ordering', () => {
    const forwards = new AirdropTree(SHARED_FIXTURE);
    const backwards = new AirdropTree([...SHARED_FIXTURE].reverse());

    expect(backwards.rootHex).toBe(forwards.rootHex);
  });

  it('handles a single entry, where the root is the leaf', () => {
    const only = SHARED_FIXTURE[0]!;
    const tree = new AirdropTree([only]);

    expect(tree.root.equals(AirdropTree.leafFor(only))).toBe(true);
    expect(tree.proofFor(only.index)).toEqual([]);
  });

  it('handles odd entry counts, where the last node is promoted', () => {
    const odd = SHARED_FIXTURE.slice(0, 3);
    const tree = new AirdropTree(odd);

    for (const entry of odd) {
      expect(tree.verify(entry, tree.proofFor(entry.index)!)).toBe(true);
    }
  });

  /**
   * The encoding trap worth pinning explicitly: Rust writes u64 little-endian, and a codebase
   * ported from EVM would naturally write big-endian.
   */
  it('encodes integers little-endian', () => {
    const entry: AirdropEntry = {
      index: 1,
      claimant: new PublicKey('11111111111111111111111111111112'),
      amount: 1n,
    };

    const leafLittleEndian = AirdropTree.leafFor(entry);

    // The same values encoded big-endian must produce a different leaf. If these ever match, the
    // encoding has silently become endian-agnostic and the cross-check is no longer meaningful.
    const bigEndianIndex = Buffer.alloc(8);
    bigEndianIndex.writeBigUInt64BE(1n);
    expect(leafLittleEndian.subarray(0, 8).equals(bigEndianIndex)).toBe(false);
  });

  it('reports the total allocation', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);
    expect(tree.totalAmount).toBe(19_000_000_000n);
    expect(tree.size).toBe(5);
  });

  it('rejects an empty entry list', () => {
    expect(() => new AirdropTree([])).toThrow(/no entries/);
  });

  it('matches the root the Rust suite asserts', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);
    expect(tree.rootHex).toBe(SHARED_FIXTURE_ROOT);
  });

  it('prints the shared fixture root', () => {
    const tree = new AirdropTree(SHARED_FIXTURE);
    console.log(`SHARED_FIXTURE_ROOT = ${tree.rootHex}`);
    expect(tree.rootHex).toMatch(/^[0-9a-f]{64}$/);
  });
});
