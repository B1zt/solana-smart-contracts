//! # Token Platform
//!
//! Everything a Solana project needs to launch a token and distribute it: a capped mint whose
//! authority is a PDA, linear vesting with cliffs, a Merkle airdrop, and a staking pool.
//!
//! Four ideas run through the whole program, and each is the Solana-native answer to a problem that
//! is solved differently on EVM:
//!
//! **PDAs replace privileged addresses.** The mint authority, every vault and every pool are
//! program-derived addresses. A PDA has no private key and can only sign through this program, so
//! authority is bounded by code rather than by key custody. There is no equivalent of "the owner
//! key was compromised" for a PDA.
//!
//! **Account existence replaces boolean flags.** An airdrop claim is recorded by creating a PDA.
//! The runtime refuses to initialise the same address twice, so a replayed claim fails before the
//! program checks anything. There is no flag to forget to set.
//!
//! **Accumulators replace iteration.** Staking rewards use a single running `reward_per_token`
//! figure rather than per-user bookkeeping. On Solana this is not an optimisation: a transaction
//! touches a bounded set of accounts, so anything that loops over participants stops working once
//! there are enough of them.
//!
//! **Constraints replace defensive code.** Most of the security here lives in the `#[derive(Accounts)]`
//! blocks: `has_one`, `seeds`, `token::authority` and `address` are checked before any handler runs.
//! The three classic Solana vulnerabilities, a missing signer check, a substituted account and an
//! unvalidated PDA, are all closed declaratively rather than with runtime `if` statements.

pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("HxRsEhSd2zBMTDDmQsiR4jA1ZGjdQVybjXrwHAedJUDJ");

#[program]
pub mod token_platform {
    use super::*;

    /* ------------------------------------------------------------ launch --- */

    /// Take custody of a mint's authority and set an immutable supply cap.
    pub fn initialize_launch(ctx: Context<InitializeLaunch>, supply_cap: u64) -> Result<()> {
        instructions::launch::handle_initialize_launch(ctx, supply_cap)
    }

    /// Mint tokens, bounded by the cap.
    pub fn mint_tokens(ctx: Context<MintTokens>, amount: u64) -> Result<()> {
        instructions::launch::handle_mint_tokens(ctx, amount)
    }

    /// Permanently disable minting. One-way.
    pub fn finish_minting(ctx: Context<FinishMinting>) -> Result<()> {
        instructions::launch::handle_finish_minting(ctx)
    }

    /* ----------------------------------------------------------- vesting --- */

    /// Create and fund a vesting schedule.
    pub fn create_vesting(
        ctx: Context<CreateVesting>,
        seed: u64,
        amount: u64,
        start_ts: i64,
        cliff_seconds: i64,
        duration_seconds: i64,
        revocable: bool,
    ) -> Result<()> {
        instructions::vesting::handle_create_vesting(
            ctx,
            seed,
            amount,
            start_ts,
            cliff_seconds,
            duration_seconds,
            revocable,
        )
    }

    /// Release vested tokens to the beneficiary. Permissionless.
    pub fn release_vesting(ctx: Context<ReleaseVesting>) -> Result<()> {
        instructions::vesting::handle_release_vesting(ctx)
    }

    /// Cancel the unvested remainder, paying out what has already vested first.
    pub fn revoke_vesting(ctx: Context<RevokeVesting>) -> Result<()> {
        instructions::vesting::handle_revoke_vesting(ctx)
    }

    /// Close a fully released schedule and reclaim its rent.
    pub fn close_vesting(ctx: Context<CloseVesting>) -> Result<()> {
        instructions::vesting::handle_close_vesting(ctx)
    }

    /* ----------------------------------------------------------- airdrop --- */

    /// Create and fund a Merkle airdrop.
    pub fn initialize_distributor(
        ctx: Context<InitializeDistributor>,
        merkle_root: [u8; 32],
        total_amount: u64,
        claim_deadline: i64,
    ) -> Result<()> {
        instructions::airdrop::handle_initialize_distributor(
            ctx,
            merkle_root,
            total_amount,
            claim_deadline,
        )
    }

    /// Claim an allocation. Permissionless in who submits, bound to the leaf's claimant.
    pub fn claim_airdrop(
        ctx: Context<ClaimAirdrop>,
        index: u64,
        amount: u64,
        proof: Vec<[u8; 32]>,
    ) -> Result<()> {
        instructions::airdrop::handle_claim_airdrop(ctx, index, amount, proof)
    }

    /// Recover unclaimed tokens after the published deadline.
    pub fn clawback_airdrop(ctx: Context<ClawbackAirdrop>) -> Result<()> {
        instructions::airdrop::handle_clawback_airdrop(ctx)
    }

    /* ----------------------------------------------------------- staking --- */

    /// Create a staking pool with a reward schedule.
    pub fn initialize_pool(
        ctx: Context<InitializePool>,
        reward_rate: u64,
        reward_duration: i64,
        cooldown_seconds: i64,
    ) -> Result<()> {
        instructions::staking::handle_initialize_pool(
            ctx,
            reward_rate,
            reward_duration,
            cooldown_seconds,
        )
    }

    /// Stake tokens.
    pub fn stake(ctx: Context<Stake>, amount: u64) -> Result<()> {
        instructions::staking::handle_stake(ctx, amount)
    }

    /// Begin unstaking, starting the cooldown.
    pub fn unstake(ctx: Context<Unstake>, amount: u64) -> Result<()> {
        instructions::staking::handle_unstake(ctx, amount)
    }

    /// Withdraw unstaked tokens once the cooldown has elapsed.
    pub fn withdraw_unstaked(ctx: Context<WithdrawUnstaked>) -> Result<()> {
        instructions::staking::handle_withdraw_unstaked(ctx)
    }

    /// Claim accrued rewards.
    pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        instructions::staking::handle_claim_rewards(ctx)
    }

    /// Top up the reward vault and optionally extend the schedule.
    pub fn fund_rewards(ctx: Context<FundRewards>, amount: u64, extend_seconds: i64) -> Result<()> {
        instructions::staking::handle_fund_rewards(ctx, amount, extend_seconds)
    }
}
