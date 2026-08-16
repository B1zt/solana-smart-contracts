/// Upper bound on a staking cooldown.
///
/// An authority that can set an unbounded cooldown can lock stakers in permanently, which is
/// indistinguishable from confiscation.
pub const MAX_COOLDOWN_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Maximum Merkle proof length accepted.
///
/// A 32-deep tree covers over four billion leaves. Bounding it stops a caller passing an enormous
/// proof purely to burn compute, and keeps the instruction's cost predictable.
pub const MAX_PROOF_LENGTH: usize = 32;

/// Longest vesting schedule that may be created.
///
/// A hundred year schedule is indistinguishable from a burn, and burning is the honest way to do
/// that. Bounding it keeps the intent legible.
pub const MAX_VESTING_DURATION: i64 = 10 * 365 * 24 * 60 * 60;
