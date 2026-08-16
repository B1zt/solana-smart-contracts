//! Unit tests for the marketplace's pure logic: Merkle verification and the payment split.

use nft_marketplace::instructions::minting::{leaf_hash, verify_proof};
use nft_marketplace::state::{Collection, Marketplace};
use solana_pubkey::Pubkey;

/* ------------------------------------------------------------------- merkle --- */

fn hash_pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a <= b {
        solana_keccak_hasher::hashv(&[&a, &b]).to_bytes()
    } else {
        solana_keccak_hasher::hashv(&[&b, &a]).to_bytes()
    }
}

fn build_layers(leaves: &[[u8; 32]]) -> Vec<Vec<[u8; 32]>> {
    let mut sorted = leaves.to_vec();
    sorted.sort();

    let mut layers = vec![sorted];

    while layers.last().unwrap().len() > 1 {
        let current = layers.last().unwrap();
        let mut next = Vec::new();

        for pair in current.chunks(2) {
            // Odd nodes are promoted unchanged, not duplicated.
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
    let mut index = layers[0].iter().position(|item| *item == leaf).expect("leaf missing");
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

fn allowlist(count: usize) -> Vec<(Pubkey, u16)> {
    (0..count)
        .map(|i| (Pubkey::new_unique(), (i as u16 % 5) + 1))
        .collect()
}

#[test]
fn every_allowlist_entry_verifies() {
    let entries = allowlist(7);
    let leaves: Vec<_> = entries
        .iter()
        .map(|(wallet, allowance)| leaf_hash(wallet, *allowance))
        .collect();

    let layers = build_layers(&leaves);
    let root = layers.last().unwrap()[0];

    for (wallet, allowance) in &entries {
        let leaf = leaf_hash(wallet, *allowance);
        let proof = proof_for(&layers, leaf);

        assert!(verify_proof(&proof, root, leaf), "entry failed to verify");
    }
}

/// The leaf binds the allowance, so a valid proof cannot be reused to mint more.
#[test]
fn an_inflated_allowance_is_rejected() {
    let entries = allowlist(5);
    let leaves: Vec<_> = entries
        .iter()
        .map(|(wallet, allowance)| leaf_hash(wallet, *allowance))
        .collect();

    let layers = build_layers(&leaves);
    let root = layers.last().unwrap()[0];

    let (wallet, allowance) = &entries[0];
    let proof = proof_for(&layers, leaf_hash(wallet, *allowance));

    assert!(!verify_proof(&proof, root, leaf_hash(wallet, allowance + 100)));
}

/// The leaf binds the wallet, so an allowlist proof is not transferable.
#[test]
fn a_proof_is_not_transferable() {
    let entries = allowlist(5);
    let leaves: Vec<_> = entries
        .iter()
        .map(|(wallet, allowance)| leaf_hash(wallet, *allowance))
        .collect();

    let layers = build_layers(&leaves);
    let root = layers.last().unwrap()[0];

    let (victim, allowance) = &entries[0];
    let proof = proof_for(&layers, leaf_hash(victim, *allowance));

    let attacker = Pubkey::new_unique();
    assert!(!verify_proof(&proof, root, leaf_hash(&attacker, *allowance)));
}

/// A single-hashed leaf is 32 bytes and so is an internal node. Double hashing keeps them apart.
#[test]
fn double_hashing_separates_leaves_from_nodes() {
    let wallet = Pubkey::new_unique();

    let single = solana_keccak_hasher::hashv(&[wallet.as_ref(), &3u16.to_le_bytes()]).to_bytes();
    let double = leaf_hash(&wallet, 3);

    assert_ne!(single, double, "the leaf must not be a single hash");
    assert_eq!(
        double,
        solana_keccak_hasher::hash(&single).to_bytes(),
        "and must be exactly the hash of the single hash"
    );
}

/// Little-endian encoding, matching Rust's `to_le_bytes`. The off-chain builder must mirror it.
#[test]
fn allowance_is_encoded_little_endian() {
    let wallet = Pubkey::new_unique();

    let le_inner = solana_keccak_hasher::hashv(&[wallet.as_ref(), &256u16.to_le_bytes()]);
    let be_inner = solana_keccak_hasher::hashv(&[wallet.as_ref(), &256u16.to_be_bytes()]);

    assert_eq!(leaf_hash(&wallet, 256), solana_keccak_hasher::hash(le_inner.as_ref()).to_bytes());
    assert_ne!(
        solana_keccak_hasher::hash(be_inner.as_ref()).to_bytes(),
        leaf_hash(&wallet, 256),
        "endianness must actually matter, or this test proves nothing"
    );
}

#[test]
fn an_empty_proof_verifies_only_a_single_leaf_tree() {
    let wallet = Pubkey::new_unique();
    let leaf = leaf_hash(&wallet, 1);

    assert!(verify_proof(&[], leaf, leaf));
    assert!(!verify_proof(&[], leaf_hash(&wallet, 2), leaf));
}

#[test]
fn odd_entry_counts_still_verify() {
    for count in [3usize, 5, 9, 11] {
        let entries = allowlist(count);
        let leaves: Vec<_> = entries
            .iter()
            .map(|(wallet, allowance)| leaf_hash(wallet, *allowance))
            .collect();

        let layers = build_layers(&leaves);
        let root = layers.last().unwrap()[0];

        for (wallet, allowance) in &entries {
            let leaf = leaf_hash(wallet, *allowance);
            assert!(
                verify_proof(&proof_for(&layers, leaf), root, leaf),
                "{count} entries failed"
            );
        }
    }
}

/* -------------------------------------------------------------------- split --- */

/// The payment split as `handle_buy_nft` computes it.
fn split(price: u64, fee_bps: u16, royalty_bps: u16) -> (u64, u64, u64) {
    let fee = ((price as u128 * fee_bps as u128) / 10_000) as u64;
    let royalty = ((price as u128 * royalty_bps as u128) / 10_000) as u64;
    let proceeds = price - fee - royalty;

    (fee, royalty, proceeds)
}

#[test]
fn the_split_conserves_value() {
    let price = 5_000_000_000u64; // 5 SOL
    let (fee, royalty, proceeds) = split(price, 250, 500);

    assert_eq!(fee, 125_000_000, "2.5%");
    assert_eq!(royalty, 250_000_000, "5%");
    assert_eq!(fee + royalty + proceeds, price, "nothing created or lost");
}

/// Rounding must never create lamports out of nothing, whatever the price.
#[test]
fn the_split_never_over_pays() {
    for price in [1u64, 3, 7, 999, 1_000_001, 123_456_789, u32::MAX as u64] {
        for fee_bps in [0u16, 1, 250, Marketplace::MAX_FEE_BPS] {
            for royalty_bps in [0u16, 1, 500, Collection::MAX_ROYALTY_BPS] {
                let (fee, royalty, proceeds) = split(price, fee_bps, royalty_bps);

                assert_eq!(
                    fee + royalty + proceeds,
                    price,
                    "price {price}, fee {fee_bps}, royalty {royalty_bps}"
                );
            }
        }
    }
}

/// Both cuts are capped, so the seller always receives at least 85% of the price. That bound is
/// what makes the caps meaningful rather than decorative.
#[test]
fn the_seller_always_receives_the_majority() {
    let price = 1_000_000_000u64;

    let (fee, royalty, proceeds) = split(price, Marketplace::MAX_FEE_BPS, Collection::MAX_ROYALTY_BPS);

    assert_eq!(fee + royalty, price * 15 / 100, "5% fee plus 10% royalty");
    assert_eq!(proceeds, price * 85 / 100);
    assert!(proceeds > price / 2, "the seller keeps the majority even at the caps");
}

/// A large price must not overflow the intermediate multiply. The maths runs in u128 for this.
#[test]
fn a_large_price_does_not_overflow() {
    // Larger than the entire SOL supply, so comfortably beyond any real sale.
    let price = u64::MAX / 2;

    let (fee, royalty, proceeds) = split(price, Marketplace::MAX_FEE_BPS, Collection::MAX_ROYALTY_BPS);

    assert_eq!(fee + royalty + proceeds, price);

    // The naive u64 form really would have overflowed here.
    assert!(u64::MAX / price < Marketplace::MAX_FEE_BPS as u64);
}

#[test]
fn caps_are_what_the_documentation_claims() {
    assert_eq!(Marketplace::MAX_FEE_BPS, 500, "5% protocol fee ceiling");
    assert_eq!(Collection::MAX_ROYALTY_BPS, 1_000, "10% royalty ceiling");
}
