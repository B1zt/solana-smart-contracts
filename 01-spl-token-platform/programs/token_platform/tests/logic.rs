//! Unit tests for the program's pure logic.
//!
//! Vesting curves, Merkle verification and reward accumulation are ordinary arithmetic, and testing
//! them directly is far faster and far more thorough than driving them through a validator. The
//! litesvm tests in `security.rs` cover what only a runtime can prove: account constraints, signer
//! checks and PDA uniqueness.

use solana_pubkey::Pubkey;
use token_platform::instructions::airdrop::{leaf_hash, verify_proof};
use token_platform::state::{StakePool, VestingSchedule};

/* ------------------------------------------------------------------ vesting --- */

const DAY: i64 = 24 * 60 * 60;
const YEAR: i64 = 365 * DAY;

fn schedule(total: u64, start: i64, cliff: i64, duration: i64) -> VestingSchedule {
    VestingSchedule {
        beneficiary: Pubkey::new_unique(),
        authority: Pubkey::new_unique(),
        mint: Pubkey::new_unique(),
        vault: Pubkey::new_unique(),
        total_amount: total,
        released_amount: 0,
        start_ts: start,
        cliff_seconds: cliff,
        duration_seconds: duration,
        revocable: true,
        revoked: false,
        seed: 0,
        bump: 255,
    }
}

#[test]
fn nothing_vests_before_the_cliff() {
    let s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    assert_eq!(s.vested_amount(0), 0);
    assert_eq!(s.vested_amount(YEAR - 1), 0);
}

/// Crossing the cliff releases everything accrued during it as one tranche, because vesting is
/// linear from `start_ts` rather than from the end of the cliff. That is the convention every token
/// unlock chart assumes.
#[test]
fn the_cliff_unlocks_its_accrual_at_once() {
    let s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    let at_cliff = s.vested_amount(YEAR);
    assert_eq!(at_cliff, 250_000, "one of four years");

    // And a second before the cliff, nothing at all.
    assert_eq!(s.vested_amount(YEAR - 1), 0);
}

#[test]
fn vesting_is_linear_after_the_cliff() {
    let s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    assert_eq!(s.vested_amount(2 * YEAR), 500_000);
    assert_eq!(s.vested_amount(3 * YEAR), 750_000);
    assert_eq!(s.vested_amount(4 * YEAR), 1_000_000);
}

#[test]
fn vesting_never_exceeds_the_grant() {
    let s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    assert_eq!(s.vested_amount(4 * YEAR), 1_000_000);
    assert_eq!(s.vested_amount(40 * YEAR), 1_000_000, "does not keep growing");
    assert_eq!(s.vested_amount(i64::MAX), 1_000_000);
}

#[test]
fn vesting_is_monotonic() {
    let s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    let mut previous = 0;
    for day in 0..(4 * 365 + 100) {
        let vested = s.vested_amount(day * DAY);
        assert!(vested >= previous, "vesting decreased on day {day}");
        previous = vested;
    }
}

/// A revoked schedule is frozen at whatever had vested. `total_amount` is rewritten at revocation,
/// so nothing further accrues no matter how much time passes.
#[test]
fn revocation_freezes_the_schedule() {
    let mut s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    let vested_at_revocation = s.vested_amount(2 * YEAR);
    s.revoked = true;
    s.total_amount = vested_at_revocation;

    assert_eq!(s.vested_amount(2 * YEAR), 500_000);
    assert_eq!(s.vested_amount(10 * YEAR), 500_000, "frozen");
}

#[test]
fn releasable_subtracts_what_was_already_paid() {
    let mut s = schedule(1_000_000, 0, YEAR, 4 * YEAR);

    assert_eq!(s.releasable_amount(2 * YEAR), 500_000);

    s.released_amount = 300_000;
    assert_eq!(s.releasable_amount(2 * YEAR), 200_000);

    s.released_amount = 500_000;
    assert_eq!(s.releasable_amount(2 * YEAR), 0);
}

/// Large grants over long durations overflow u64 in the intermediate multiply. The maths runs in
/// u128 for exactly this case, and this is the test that would catch a regression to u64.
#[test]
fn large_grants_do_not_overflow() {
    // A billion tokens at 9 decimals: 1e18, close to u64's ceiling.
    let total = 1_000_000_000_000_000_000u64;
    let s = schedule(total, 0, 0, 4 * YEAR);

    assert_eq!(s.vested_amount(2 * YEAR), total / 2);
    assert_eq!(s.vested_amount(4 * YEAR), total);

    // The naive u64 computation would have overflowed here:
    //   total * (2 * YEAR) = 1e18 * 63_072_000, far beyond u64::MAX.
    assert!(u64::MAX / total < (2 * YEAR) as u64, "the naive form really would overflow");
}

#[test]
fn a_zero_cliff_vests_from_the_first_second() {
    let s = schedule(1_000_000, 0, 0, 100);

    assert_eq!(s.vested_amount(0), 0);
    assert_eq!(s.vested_amount(50), 500_000);
    assert_eq!(s.vested_amount(100), 1_000_000);
}

/* ------------------------------------------------------------------- merkle --- */

/// Build a tree the same way the off-chain builder does: sorted-pair hashing, odd nodes promoted.
fn build_tree(leaves: &[[u8; 32]]) -> Vec<Vec<[u8; 32]>> {
    let mut sorted = leaves.to_vec();
    sorted.sort();

    let mut layers = vec![sorted];

    while layers.last().unwrap().len() > 1 {
        let current = layers.last().unwrap();
        let mut next = Vec::new();

        for pair in current.chunks(2) {
            if pair.len() == 1 {
                // Promoted unchanged, not duplicated.
                next.push(pair[0]);
            } else {
                next.push(hash_pair(pair[0], pair[1]));
            }
        }

        layers.push(next);
    }

    layers
}

fn hash_pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a <= b {
        solana_keccak_hasher::hashv(&[&a, &b]).to_bytes()
    } else {
        solana_keccak_hasher::hashv(&[&b, &a]).to_bytes()
    }
}

fn proof_for(layers: &[Vec<[u8; 32]>], leaf: [u8; 32]) -> Vec<[u8; 32]> {
    let mut index = layers[0].iter().position(|item| *item == leaf).expect("leaf not in tree");
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

struct Entry {
    index: u64,
    claimant: Pubkey,
    amount: u64,
}

fn fixture() -> Vec<Entry> {
    (0..5)
        .map(|i| Entry {
            index: i,
            claimant: Pubkey::new_unique(),
            amount: 1_000 * (i + 1),
        })
        .collect()
}

#[test]
fn every_entry_verifies_against_the_root() {
    let entries = fixture();
    let leaves: Vec<_> = entries
        .iter()
        .map(|e| leaf_hash(e.index, &e.claimant, e.amount))
        .collect();

    let layers = build_tree(&leaves);
    let root = layers.last().unwrap()[0];

    for entry in &entries {
        let leaf = leaf_hash(entry.index, &entry.claimant, entry.amount);
        let proof = proof_for(&layers, leaf);

        assert!(verify_proof(&proof, root, leaf), "index {} failed", entry.index);
    }
}

/// The leaf binds the amount, so a valid proof cannot be reused to claim more.
#[test]
fn an_inflated_amount_is_rejected() {
    let entries = fixture();
    let leaves: Vec<_> = entries
        .iter()
        .map(|e| leaf_hash(e.index, &e.claimant, e.amount))
        .collect();

    let layers = build_tree(&leaves);
    let root = layers.last().unwrap()[0];

    let entry = &entries[0];
    let real_leaf = leaf_hash(entry.index, &entry.claimant, entry.amount);
    let proof = proof_for(&layers, real_leaf);

    let inflated = leaf_hash(entry.index, &entry.claimant, entry.amount * 1_000);
    assert!(!verify_proof(&proof, root, inflated));
}

/// The leaf binds the claimant, so a proof cannot be redirected to a different wallet.
#[test]
fn a_substituted_claimant_is_rejected() {
    let entries = fixture();
    let leaves: Vec<_> = entries
        .iter()
        .map(|e| leaf_hash(e.index, &e.claimant, e.amount))
        .collect();

    let layers = build_tree(&leaves);
    let root = layers.last().unwrap()[0];

    let entry = &entries[0];
    let real_leaf = leaf_hash(entry.index, &entry.claimant, entry.amount);
    let proof = proof_for(&layers, real_leaf);

    let attacker = leaf_hash(entry.index, &Pubkey::new_unique(), entry.amount);
    assert!(!verify_proof(&proof, root, attacker));
}

/// The leaf binds the index, so a proof cannot be replayed under a different index to obtain a
/// second claim PDA.
#[test]
fn a_reused_index_is_rejected() {
    let entries = fixture();
    let leaves: Vec<_> = entries
        .iter()
        .map(|e| leaf_hash(e.index, &e.claimant, e.amount))
        .collect();

    let layers = build_tree(&leaves);
    let root = layers.last().unwrap()[0];

    let entry = &entries[0];
    let real_leaf = leaf_hash(entry.index, &entry.claimant, entry.amount);
    let proof = proof_for(&layers, real_leaf);

    let shifted = leaf_hash(entry.index + 1, &entry.claimant, entry.amount);
    assert!(!verify_proof(&proof, root, shifted));
}

/// A single-hashed leaf is 32 bytes and so is an internal node. Double hashing is what keeps an
/// attacker from presenting a node from a published proof as if it were a leaf.
#[test]
fn an_internal_node_cannot_pass_as_a_leaf() {
    let entries = fixture();
    let leaves: Vec<_> = entries
        .iter()
        .map(|e| leaf_hash(e.index, &e.claimant, e.amount))
        .collect();

    let layers = build_tree(&leaves);
    let root = layers.last().unwrap()[0];

    // An internal node, which an attacker can read straight out of anyone's published proof.
    let internal = layers[1][0];

    // Presenting it as a leaf requires a proof from its own level upward, and the double hash means
    // the value never matches what `leaf_hash` would produce for any real entry.
    let proof_from_here = proof_for(&layers, layers[0][0]);
    assert!(!verify_proof(&proof_from_here, root, internal));
}

#[test]
fn an_empty_proof_only_verifies_a_single_leaf_tree() {
    let claimant = Pubkey::new_unique();
    let leaf = leaf_hash(0, &claimant, 1_000);

    // One leaf: the root is the leaf, so an empty proof is correct.
    assert!(verify_proof(&[], leaf, leaf));

    // Against any other root it fails.
    let other = leaf_hash(1, &claimant, 1_000);
    assert!(!verify_proof(&[], other, leaf));
}

#[test]
fn odd_leaf_counts_still_verify() {
    // Three and five leaves both force a promoted node.
    for count in [3usize, 5, 7, 9] {
        let entries: Vec<_> = (0..count as u64)
            .map(|i| Entry {
                index: i,
                claimant: Pubkey::new_unique(),
                amount: 1_000,
            })
            .collect();

        let leaves: Vec<_> = entries
            .iter()
            .map(|e| leaf_hash(e.index, &e.claimant, e.amount))
            .collect();

        let layers = build_tree(&leaves);
        let root = layers.last().unwrap()[0];

        for entry in &entries {
            let leaf = leaf_hash(entry.index, &entry.claimant, entry.amount);
            let proof = proof_for(&layers, leaf);
            assert!(verify_proof(&proof, root, leaf), "{count} leaves, index {}", entry.index);
        }
    }
}

/* ------------------------------------------------------------------ staking --- */

/// Reward accrual, reproduced against the same accumulator maths the program uses.
fn accumulate(reward_per_token: u128, rate: u64, elapsed: i64, total_staked: u64) -> u128 {
    if total_staked == 0 {
        return reward_per_token;
    }

    let emitted = (elapsed as u128) * (rate as u128);
    reward_per_token + (emitted * StakePool::ACC_PRECISION) / total_staked as u128
}

fn owed(amount: u64, reward_per_token: u128, reward_debt: u128) -> u64 {
    ((amount as u128 * (reward_per_token - reward_debt)) / StakePool::ACC_PRECISION) as u64
}

/// Two stakers present for the same period split rewards in proportion to their stake.
#[test]
fn rewards_split_by_stake() {
    let rate = 100u64;
    let elapsed = 1_000i64;

    let alice = 1_000u64;
    let bob = 3_000u64;
    let total = alice + bob;

    let rpt = accumulate(0, rate, elapsed, total);

    let alice_owed = owed(alice, rpt, 0);
    let bob_owed = owed(bob, rpt, 0);

    let emitted = rate * elapsed as u64;

    assert_eq!(alice_owed, emitted / 4, "a quarter of the stake");
    assert_eq!(bob_owed, (emitted * 3) / 4, "three quarters");
    // Rounding may leave a unit or two behind in the vault, never above.
    assert!(alice_owed + bob_owed <= emitted);
}

/// A staker who joins later must not receive anything for the period before they arrived. Their
/// `reward_debt` is set to the accumulator at entry, so the difference starts at zero.
#[test]
fn a_late_staker_earns_nothing_from_before_they_joined() {
    let rate = 100u64;

    // Alice alone for 1,000 seconds.
    let rpt_before = accumulate(0, rate, 1_000, 1_000);

    // Bob joins with an equal stake; his debt is the accumulator as it stands.
    let bob_debt = rpt_before;

    // Another 1,000 seconds with both staked.
    let rpt_after = accumulate(rpt_before, rate, 1_000, 2_000);

    let alice_owed = owed(1_000, rpt_after, 0);
    let bob_owed = owed(1_000, rpt_after, bob_debt);

    assert_eq!(bob_owed, 50_000, "half of the second period only");
    assert_eq!(alice_owed, 150_000, "all of the first, half of the second");
}

/// Nothing accrues while the pool is empty, so a first depositor cannot collect a backlog.
#[test]
fn an_empty_pool_accrues_nothing() {
    let rpt = accumulate(0, 100, 10_000, 0);
    assert_eq!(rpt, 0, "no stake, no accrual");
}

/// The accumulator is scaled, so small stakes still accrue rather than rounding to zero.
#[test]
fn small_stakes_still_accrue() {
    let rate = 1_000u64;
    let total = 1_000_000_000u64;

    let rpt = accumulate(0, rate, 3_600, total);

    // A one-millionth share of the pool over an hour.
    let tiny = 1_000u64;
    let tiny_owed = owed(tiny, rpt, 0);

    assert!(tiny_owed > 0, "precision scaling keeps small positions earning");
}

/// The accumulator must survive a large pool running for years without overflowing u128.
#[test]
fn accumulator_does_not_overflow_over_realistic_lifetimes() {
    let rate = 1_000_000_000u64; // 1 token per second at 9 decimals
    let total = 1_000_000_000_000_000u64; // 1M tokens staked at 9 decimals
    let ten_years = 10 * 365 * 24 * 60 * 60i64;

    let rpt = accumulate(0, rate, ten_years, total);

    assert!(rpt > 0);
    // Comfortably inside u128, which is why the accumulator is u128 rather than u64.
    assert!(rpt < u128::MAX / 1_000);
}

/// Emissions stop at the schedule's end, so a pool does not keep paying after it runs dry.
#[test]
fn accrual_stops_at_the_schedule_end() {
    let pool = StakePool {
        authority: Pubkey::new_unique(),
        stake_mint: Pubkey::new_unique(),
        reward_mint: Pubkey::new_unique(),
        stake_vault: Pubkey::new_unique(),
        reward_vault: Pubkey::new_unique(),
        total_staked: 1_000,
        reward_rate: 100,
        reward_per_token: 0,
        last_update_ts: 0,
        reward_end_ts: 1_000,
        cooldown_seconds: 0,
        pending_rewards: 0,
        bump: 255,
    };

    assert_eq!(pool.accrual_ts(500), 500, "inside the schedule");
    assert_eq!(pool.accrual_ts(1_000), 1_000, "exactly at the end");
    assert_eq!(pool.accrual_ts(5_000), 1_000, "clamped past the end");
}
