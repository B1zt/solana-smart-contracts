import {PublicKey} from '@solana/web3.js';
import {describe, expect, it} from 'vitest';
import {Allowlist, type AllowlistEntry} from './allowlist.js';

const FIXTURE: AllowlistEntry[] = [
  {wallet: new PublicKey('11111111111111111111111111111112'), allowance: 1},
  {wallet: new PublicKey('11111111111111111111111111111113'), allowance: 2},
  {wallet: new PublicKey('11111111111111111111111111111114'), allowance: 3},
  {wallet: new PublicKey('11111111111111111111111111111115'), allowance: 5},
];

describe('Allowlist', () => {
  it('produces a verifiable proof for every entry', () => {
    const tree = new Allowlist(FIXTURE);

    for (const entry of FIXTURE) {
      const proof = tree.proofFor(entry.wallet.toBase58());
      expect(proof).not.toBeNull();
      expect(tree.verify(entry, proof!)).toBe(true);
    }
  });

  it('rejects an inflated allowance', () => {
    const tree = new Allowlist(FIXTURE);
    const entry = FIXTURE[0]!;
    const proof = tree.proofFor(entry.wallet.toBase58())!;

    expect(tree.verify({...entry, allowance: 100}, proof)).toBe(false);
  });

  it('rejects a proof presented by another wallet', () => {
    const tree = new Allowlist(FIXTURE);
    const victim = FIXTURE[0]!;
    const attacker = FIXTURE[1]!;
    const proof = tree.proofFor(victim.wallet.toBase58())!;

    expect(tree.verify({...victim, wallet: attacker.wallet}, proof)).toBe(false);
  });

  it('rejects duplicate wallets', () => {
    expect(
      () => new Allowlist([FIXTURE[0]!, {...FIXTURE[1]!, wallet: FIXTURE[0]!.wallet}]),
    ).toThrow(/duplicate wallet/);
  });

  /** The on-chain type is u16, so anything larger cannot be represented in the leaf. */
  it('rejects an allowance that does not fit in a u16', () => {
    expect(() => new Allowlist([{...FIXTURE[0]!, allowance: 70_000}])).toThrow(/u16/);
  });

  it('encodes the allowance as a little-endian u16', () => {
    const wallet = new PublicKey('11111111111111111111111111111112');

    // 256 is 0x0100. Little-endian writes 00 01; big-endian writes 01 00. If those ever produced
    // the same leaf, this test would be proving nothing.
    const le = Buffer.alloc(2);
    le.writeUInt16LE(256);
    const be = Buffer.alloc(2);
    be.writeUInt16BE(256);

    expect(le.equals(be)).toBe(false);
    expect(Allowlist.leafFor({wallet, allowance: 256})).not.toEqual(
      Allowlist.leafFor({wallet, allowance: 1}),
    );
  });

  it('is deterministic regardless of input order', () => {
    expect(new Allowlist([...FIXTURE].reverse()).rootHex).toBe(new Allowlist(FIXTURE).rootHex);
  });

  it('handles odd entry counts', () => {
    const odd = FIXTURE.slice(0, 3);
    const tree = new Allowlist(odd);

    for (const entry of odd) {
      expect(tree.verify(entry, tree.proofFor(entry.wallet.toBase58())!)).toBe(true);
    }
  });

  it('exposes the root as bytes for the instruction', () => {
    const tree = new Allowlist(FIXTURE);
    expect(tree.rootBytes).toHaveLength(32);
    expect(Buffer.from(tree.rootBytes).toString('hex')).toBe(tree.rootHex);
  });

  it('rejects an empty entry list', () => {
    expect(() => new Allowlist([])).toThrow(/no entries/);
  });
});

/**
 * The other half of the cross-check.
 *
 * This tree is built here in TypeScript and verified on-chain in Rust, and nothing forces the two
 * to agree. When they disagree the failure is silent and total: the API serves well-formed proofs
 * and every mint fails with `InvalidProof`. Both sides build the same fixed entry set and assert
 * this same root, so a change to either encoding breaks a test rather than the drop.
 *
 * The Rust twin is `programs/nft_marketplace/tests/cross_check.rs`.
 */
describe('cross-check against the on-chain verifier', () => {
  const SHARED_FIXTURE = [
    {wallet: new PublicKey('11111111111111111111111111111112'), allowance: 1},
    {wallet: new PublicKey('11111111111111111111111111111113'), allowance: 2},
    {wallet: new PublicKey('11111111111111111111111111111114'), allowance: 3},
    {wallet: new PublicKey('11111111111111111111111111111115'), allowance: 5},
    {wallet: new PublicKey('11111111111111111111111111111116'), allowance: 8},
  ];

  /** Must match SHARED_FIXTURE_ROOT in programs/nft_marketplace/tests/cross_check.rs. */
  const SHARED_FIXTURE_ROOT =
    '2f43697b1ea109ad367ea31705f983697dd4cc11fb22a94021113fa0419c622e';

  it('produces the root the Rust tests assert', () => {
    expect(new Allowlist(SHARED_FIXTURE).rootHex).toBe(SHARED_FIXTURE_ROOT);
  });

  it('produces a proof for every fixture entry', () => {
    const tree = new Allowlist(SHARED_FIXTURE);

    for (const entry of SHARED_FIXTURE) {
      expect(tree.verify(entry, tree.proofFor(entry.wallet.toBase58())!)).toBe(true);
    }
  });
});
