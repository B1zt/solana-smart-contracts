use anchor_lang::prelude::*;

#[error_code]
pub enum TokenPlatformError {
    /* ------------------------------------------------------------ launch --- */
    #[msg("Minting would exceed the supply cap")]
    SupplyCapExceeded,
    #[msg("Minting has been permanently disabled")]
    MintingFinished,
    #[msg("Supply cap must be greater than zero")]
    InvalidSupplyCap,

    /* ----------------------------------------------------------- vesting --- */
    #[msg("Vesting duration must be greater than zero")]
    InvalidDuration,
    #[msg("Cliff cannot be longer than the total duration")]
    CliffExceedsDuration,
    #[msg("Nothing is currently releasable")]
    NothingToRelease,
    #[msg("This schedule is not revocable")]
    NotRevocable,
    #[msg("This schedule has already been revoked")]
    AlreadyRevoked,
    #[msg("Schedule still holds unreleased tokens")]
    ScheduleNotEmpty,

    /* ---------------------------------------------------------- airdrop --- */
    #[msg("Merkle proof did not verify for this claim")]
    InvalidProof,
    #[msg("The claim window has closed")]
    ClaimWindowClosed,
    #[msg("The claim window is still open")]
    ClaimWindowOpen,
    #[msg("Proof is longer than the maximum supported tree depth")]
    ProofTooLong,

    /* ---------------------------------------------------------- staking --- */
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient staked balance")]
    InsufficientStake,
    #[msg("Unstaked tokens are still cooling down")]
    StillCoolingDown,
    #[msg("Nothing is waiting to be withdrawn")]
    NothingToWithdraw,
    #[msg("Reward schedule has already ended")]
    RewardScheduleEnded,
    #[msg("Cooldown exceeds the maximum allowed")]
    CooldownTooLong,
    #[msg("Reward vault does not hold enough to cover committed rewards")]
    InsufficientRewardBalance,

    /* ----------------------------------------------------------- shared --- */
    #[msg("Arithmetic overflowed")]
    MathOverflow,
    #[msg("Timestamp is in the past")]
    TimestampInPast,
    #[msg("Caller is not authorised for this action")]
    Unauthorized,
}
