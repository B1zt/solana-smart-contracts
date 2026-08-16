//! Proves the off-chain allowlist builder and the on-chain verifier agree.
//!
//! The allowlist tree is built by the backend in TypeScript and verified on-chain in Rust. Nothing
//! forces the two to agree, and when they disagree the failure is silent and total: the API serves
//! well-formed proofs and every mint fails with `InvalidProof`.
//!
//! Both sides build from the same fixed entry set and assert the same root. The TypeScript twin is
//! `backend/src/merkle/allowlist.test.ts`.

use nft_marketplace::instructions::minting::{leaf_hash, verify_proof};
use solana_pubkey::Pubkey;

/// Must match SHARED_FIXTURE_ROOT in backend/src/merkle/allowlist.test.ts.
const SHARED_FIXTURE_ROOT: &str = "2f43697b1ea109ad367ea31705f983697dd4cc11fb22a94021113fa0419c622e";

/// The same five entries the TypeScript fixture uses.
///
/// The wallets are the system program's address with the last byte varied, which gives stable,
/// readable base58 strings that both sides can hardcode without a keypair file.
fn fixture() -> Vec<(Pubkey, u16)> {
    vec![
        (Pubkey::from_str_const("11111111111111111111111111111112"), 1),
        (Pubkey::from_str_const("11111111111111111111111111111113"), 2),
        (Pubkey::from_str_const("11111111111111111111111111111114"), 3),
        (Pubkey::from_str_const("11111111111111111111111111111115"), 5),
        (Pubkey::from_str_const("11111111111111111111111111111116"), 8),
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
        .map(|(wallet, allowance)| leaf_hash(wallet, *allowance))
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
        .map(|(wallet, allowance)| leaf_hash(wallet, *allowance))
        .collect();

    let layers = build_layers(&leaves);
    let root = layers.last().unwrap()[0];

    for (wallet, allowance) in &entries {
        let leaf = leaf_hash(wallet, *allowance);
        let proof = proof_for(&layers, leaf);

        assert!(verify_proof(&proof, root, leaf), "{wallet} failed to verify");
    }
}

/// The allowance is a u16 written little-endian. That is the specific detail most likely to drift
/// between a Rust `to_le_bytes` and a JavaScript `writeUInt16LE` that somebody "fixes", so it is
/// pinned directly rather than only implied by the root.
#[test]
fn the_allowance_is_encoded_little_endian() {
    let wallet = Pubkey::from_str_const("11111111111111111111111111111112");

    let le_payload = [wallet.as_ref(), 256u16.to_le_bytes().as_slice()].concat();
    let inner = solana_keccak_hasher::hash(&le_payload);
    let expected = solana_keccak_hasher::hash(inner.as_ref()).to_bytes();

    assert_eq!(leaf_hash(&wallet, 256), expected);

    // And confirm big-endian really would differ, so this test cannot pass vacuously.
    let be_payload = [wallet.as_ref(), 256u16.to_be_bytes().as_slice()].concat();
    let be_inner = solana_keccak_hasher::hash(&be_payload);

    assert_ne!(
        solana_keccak_hasher::hash(be_inner.as_ref()).to_bytes(),
        expected,
        "endianness must actually matter here"
    );
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
