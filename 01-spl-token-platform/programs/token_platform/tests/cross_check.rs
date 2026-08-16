//! Proves the off-chain Merkle builder and the on-chain verifier agree.
//!
//! The tree is built by the backend in TypeScript and verified on-chain in Rust. Nothing forces the
//! two to agree, and when they disagree the failure is silent and total: the API serves well-formed
//! proofs, and every claim reverts with `InvalidProof`.
//!
//! This matters more here than it did on EVM. Rust's `to_le_bytes` is little-endian while every EVM
//! codebase encodes big-endian, so a tree builder ported across without noticing produces a
//! plausible-looking root that matches nothing.
//!
//! Both sides build from the same fixed entry set and assert the same root. The TypeScript twin is
//! `backend/src/merkle/tree.test.ts`.

use solana_pubkey::Pubkey;
use token_platform::instructions::airdrop::{leaf_hash, verify_proof};

/// Must match SHARED_FIXTURE_ROOT in backend/src/merkle/tree.test.ts.
const SHARED_FIXTURE_ROOT: &str = "d05181965dd81581e12198d2ed0c84fd085d6efffc7c9c0045f8ff93c686c0b8";

/// The same five entries the TypeScript fixture uses.
///
/// The claimants are the system program's address with the last byte varied, which gives stable,
/// readable base58 strings that both sides can hardcode without a keypair file.
fn fixture() -> Vec<(u64, Pubkey, u64)> {
    vec![
        (0, Pubkey::from_str_const("11111111111111111111111111111112"), 1_000_000_000),
        (1, Pubkey::from_str_const("11111111111111111111111111111113"), 2_000_000_000),
        (2, Pubkey::from_str_const("11111111111111111111111111111114"), 3_000_000_000),
        (3, Pubkey::from_str_const("11111111111111111111111111111115"), 5_000_000_000),
        (4, Pubkey::from_str_const("11111111111111111111111111111116"), 8_000_000_000),
    ]
}

fn hash_pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a <= b {
        solana_keccak_hasher::hashv(&[&a, &b]).to_bytes()
    } else {
        solana_keccak_hasher::hashv(&[&b, &a]).to_bytes()
    }
}

/// Build the tree exactly as the backend does: leaves sorted by hash, pairs ordered before hashing,
/// odd nodes promoted unchanged rather than duplicated.
fn build_layers(leaves: &[[u8; 32]]) -> Vec<Vec<[u8; 32]>> {
    let mut sorted = leaves.to_vec();
    sorted.sort();

    let mut layers = vec![sorted];

    while layers.last().unwrap().len() > 1 {
        let current = layers.last().unwrap();
        let mut next = Vec::new();

        for pair in current.chunks(2) {
            next.push(if pair.len() == 1 {
                pair[0]
            } else {
                hash_pair(pair[0], pair[1])
            });
        }

        layers.push(next);
    }

    layers
}

fn proof_for(layers: &[Vec<[u8; 32]>], leaf: [u8; 32]) -> Vec<[u8; 32]> {
    let mut index = layers[0]
        .iter()
        .position(|item| *item == leaf)
        .expect("leaf not in tree");
    let mut proof = Vec::new();

    for layer in &layers[..layers.len() - 1] {
        let sibling = index ^ 1;
        if sibling < layer.len() {
            proof.push(layer[sibling]);
        }
        index /= 2;
    }

    proof
}

/// The core cross-check: same entries, same root, on both sides of the stack.
#[test]
fn root_matches_the_typescript_implementation() {
    let leaves: Vec<_> = fixture()
        .iter()
        .map(|(index, claimant, amount)| leaf_hash(*index, claimant, *amount))
        .collect();

    let layers = build_layers(&leaves);
    let root = layers.last().unwrap()[0];

    assert_eq!(hex(&root), SHARED_FIXTURE_ROOT, "roots diverged");
}

/// Every fixture entry verifies against that root using the on-chain verifier.
#[test]
fn every_fixture_entry_verifies_on_chain() {
    let entries = fixture();
    let leaves: Vec<_> = entries
        .iter()
        .map(|(index, claimant, amount)| leaf_hash(*index, claimant, *amount))
        .collect();

    let layers = build_layers(&leaves);
    let root = layers.last().unwrap()[0];

    for (index, claimant, amount) in &entries {
        let leaf = leaf_hash(*index, claimant, *amount);
        let proof = proof_for(&layers, leaf);

        assert!(
            verify_proof(&proof, root, leaf),
            "index {index} failed to verify"
        );
    }
}

/// The little-endian encoding is the specific thing most likely to drift, so it is pinned directly
/// rather than only implied by the root.
#[test]
fn integers_are_encoded_little_endian() {
    let claimant = Pubkey::from_str_const("11111111111111111111111111111112");

    // Reproduce the leaf by hand with explicit little-endian encoding.
    let payload = [
        1u64.to_le_bytes().as_slice(),
        claimant.as_ref(),
        1_000u64.to_le_bytes().as_slice(),
    ]
    .concat();

    let inner = solana_keccak_hasher::hash(&payload);
    let expected = solana_keccak_hasher::hash(inner.as_ref()).to_bytes();

    assert_eq!(leaf_hash(1, &claimant, 1_000), expected);

    // And confirm big-endian really would differ, so this test cannot pass vacuously.
    let be_payload = [
        1u64.to_be_bytes().as_slice(),
        claimant.as_ref(),
        1_000u64.to_be_bytes().as_slice(),
    ]
    .concat();

    let be_inner = solana_keccak_hasher::hash(&be_payload);
    let be_leaf = solana_keccak_hasher::hash(be_inner.as_ref()).to_bytes();

    assert_ne!(be_leaf, expected, "endianness must actually matter here");
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
