use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface};

use crate::constants::MAX_VESTING_DURATION;
use crate::error::TokenPlatformError;
use crate::state::VestingSchedule;

/// Create and fund a vesting schedule.
///
/// The grant is transferred into a program-owned vault in the same instruction. A schedule that
/// exists but is not funded is worse than no schedule: it reads as a commitment while being
/// unenforceable, and the beneficiary has no way to tell the difference from the outside.
#[derive(Accounts)]
#[instruction(seed: u64)]
pub struct CreateVesting<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: only used as a key in the PDA seeds and stored for later comparison. It never signs
    /// and is never written to, so no further validation is meaningful here.
    pub beneficiary: UncheckedAccount<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + VestingSchedule::INIT_SPACE,
        seeds = [
            VestingSchedule::SEED,
            beneficiary.key().as_ref(),
            mint.key().as_ref(),
            &seed.to_le_bytes(),
        ],
        bump,
    )]
    pub schedule: Account<'info, VestingSchedule>,

    /// Vault owned by the schedule PDA.
    ///
    /// One vault per grant. A shared pool would mean one beneficiary's release path touches
    /// balances belonging to everyone else, so any accounting slip becomes everyone's problem.
    #[account(
        init,
        payer = authority,
        seeds = [VestingSchedule::VAULT_SEED, schedule.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = schedule,
        token::token_program = token_program,
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = mint, token::authority = authority)]
    pub funding_account: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_create_vesting(
    ctx: Context<CreateVesting>,
    seed: u64,
    amount: u64,
    start_ts: i64,
    cliff_seconds: i64,
    duration_seconds: i64,
    revocable: bool,
) -> Result<()> {
    require!(amount > 0, TokenPlatformError::ZeroAmount);
    require!(duration_seconds > 0, TokenPlatformError::InvalidDuration);
    require!(
        duration_seconds <= MAX_VESTING_DURATION,
        TokenPlatformError::InvalidDuration
    );
    require!(cliff_seconds >= 0, TokenPlatformError::InvalidDuration);
    require!(
        cliff_seconds <= duration_seconds,
        TokenPlatformError::CliffExceedsDuration
    );

    let schedule = &mut ctx.accounts.schedule;
    schedule.beneficiary = ctx.accounts.beneficiary.key();
    schedule.authority = ctx.accounts.authority.key();
    schedule.mint = ctx.accounts.mint.key();
    schedule.vault = ctx.accounts.vault.key();
    schedule.total_amount = amount;
    schedule.released_amount = 0;
    schedule.start_ts = start_ts;
    schedule.cliff_seconds = cliff_seconds;
    schedule.duration_seconds = duration_seconds;
    schedule.revocable = revocable;
    schedule.revoked = false;
    schedule.seed = seed;
    schedule.bump = ctx.bumps.schedule;

    // Fund it now, not later.
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.funding_account.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    Ok(())
}

/// Release everything vested and unclaimed.
///
/// Permissionless: anyone may trigger it, but the tokens always go to the beneficiary's account.
/// That lets a project pay the transaction fee for users who never claim, without being able to
/// redirect a single token.
#[derive(Accounts)]
pub struct ReleaseVesting<'info> {
    /// Whoever pays for the transaction. Not necessarily the beneficiary.
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [
            VestingSchedule::SEED,
            schedule.beneficiary.as_ref(),
            schedule.mint.as_ref(),
            &schedule.seed.to_le_bytes(),
        ],
        bump = schedule.bump,
        has_one = vault,
        has_one = mint,
    )]
    pub schedule: Account<'info, VestingSchedule>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    /// Destination, constrained to the beneficiary's own account.
    ///
    /// `token::authority = schedule.beneficiary` is the check that makes this instruction safe to
    /// leave permissionless. Without it, anyone could trigger a release into their own account.
    #[account(
        mut,
        token::mint = mint,
        token::authority = schedule.beneficiary,
    )]
    pub beneficiary_token_account: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_release_vesting(ctx: Context<ReleaseVesting>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let releasable = ctx.accounts.schedule.releasable_amount(now);
    require!(releasable > 0, TokenPlatformError::NothingToRelease);

    // State first, transfer second.
    ctx.accounts.schedule.released_amount = ctx
        .accounts
        .schedule
        .released_amount
        .checked_add(releasable)
        .ok_or(TokenPlatformError::MathOverflow)?;

    transfer_from_vault(
        &ctx.accounts.schedule,
        &ctx.accounts.vault,
        &ctx.accounts.beneficiary_token_account,
        &ctx.accounts.mint,
        &ctx.accounts.token_program,
        releasable,
    )?;

    msg!("released {} to {}", releasable, ctx.accounts.schedule.beneficiary);

    Ok(())
}

/// Cancel the unvested remainder of a revocable schedule.
///
/// Everything already vested is paid to the beneficiary first, in the same instruction, and only
/// the remainder returns to the authority. Skipping that first step would let an authority time a
/// revocation to confiscate tokens the beneficiary had already earned, which turns vesting from a
/// commitment back into a promise.
#[derive(Accounts)]
pub struct RevokeVesting<'info> {
    #[account(address = schedule.authority @ TokenPlatformError::Unauthorized)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [
            VestingSchedule::SEED,
            schedule.beneficiary.as_ref(),
            schedule.mint.as_ref(),
            &schedule.seed.to_le_bytes(),
        ],
        bump = schedule.bump,
        has_one = vault,
        has_one = mint,
    )]
    pub schedule: Account<'info, VestingSchedule>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = schedule.beneficiary,
    )]
    pub beneficiary_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = mint, token::authority = authority)]
    pub authority_token_account: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_revoke_vesting(ctx: Context<RevokeVesting>) -> Result<()> {
    require!(ctx.accounts.schedule.revocable, TokenPlatformError::NotRevocable);
    require!(!ctx.accounts.schedule.revoked, TokenPlatformError::AlreadyRevoked);

    let now = Clock::get()?.unix_timestamp;

    let vested = ctx.accounts.schedule.vested_amount(now);
    let releasable = vested.saturating_sub(ctx.accounts.schedule.released_amount);
    let refund = ctx.accounts.schedule.total_amount.saturating_sub(vested);

    // Pay out what the beneficiary already earned before anything returns to the authority.
    if releasable > 0 {
        ctx.accounts.schedule.released_amount = vested;

        transfer_from_vault(
            &ctx.accounts.schedule,
            &ctx.accounts.vault,
            &ctx.accounts.beneficiary_token_account,
            &ctx.accounts.mint,
            &ctx.accounts.token_program,
            releasable,
        )?;
    }

    // Freeze the schedule at whatever had vested, so nothing further accrues.
    ctx.accounts.schedule.revoked = true;
    ctx.accounts.schedule.total_amount = vested;

    if refund > 0 {
        transfer_from_vault(
            &ctx.accounts.schedule,
            &ctx.accounts.vault,
            &ctx.accounts.authority_token_account,
            &ctx.accounts.mint,
            &ctx.accounts.token_program,
            refund,
        )?;
    }

    msg!("revoked: {} paid out, {} returned", releasable, refund);

    Ok(())
}

/// Close a fully released schedule and reclaim its rent.
///
/// Solana charges rent-exempt deposits for every account, and a finished vesting schedule holds two
/// of them. Reclaiming that is real money returned to whoever funded the grant, and leaving dead
/// accounts behind is one of the more common wastes in Solana programs.
#[derive(Accounts)]
pub struct CloseVesting<'info> {
    #[account(mut, address = schedule.authority @ TokenPlatformError::Unauthorized)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [
            VestingSchedule::SEED,
            schedule.beneficiary.as_ref(),
            schedule.mint.as_ref(),
            &schedule.seed.to_le_bytes(),
        ],
        bump = schedule.bump,
        has_one = vault,
        close = authority,
    )]
    pub schedule: Account<'info, VestingSchedule>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_close_vesting(ctx: Context<CloseVesting>) -> Result<()> {
    // Only closeable once nothing is owed. Closing early would strand the beneficiary's tokens in
    // an account nobody can reach.
    require!(
        ctx.accounts.schedule.released_amount >= ctx.accounts.schedule.total_amount,
        TokenPlatformError::ScheduleNotEmpty
    );
    require!(ctx.accounts.vault.amount == 0, TokenPlatformError::ScheduleNotEmpty);

    let schedule_key = ctx.accounts.schedule.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        VestingSchedule::SEED,
        ctx.accounts.schedule.beneficiary.as_ref(),
        ctx.accounts.schedule.mint.as_ref(),
        &ctx.accounts.schedule.seed.to_le_bytes(),
        &[ctx.accounts.schedule.bump],
    ]];

    // Close the vault too, returning its rent. `close = authority` on the schedule handles the
    // other account.
    token_interface::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        token_interface::CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.authority.to_account_info(),
            authority: ctx.accounts.schedule.to_account_info(),
        },
        signer_seeds,
    ))?;

    msg!("closed vesting schedule {}", schedule_key);

    Ok(())
}

/// Shared vault transfer, signed by the schedule PDA.
fn transfer_from_vault<'info>(
    schedule: &Account<'info, VestingSchedule>,
    vault: &InterfaceAccount<'info, TokenAccount>,
    destination: &InterfaceAccount<'info, TokenAccount>,
    mint: &InterfaceAccount<'info, Mint>,
    token_program: &Interface<'info, TokenInterface>,
    amount: u64,
) -> Result<()> {
    let beneficiary = schedule.beneficiary;
    let mint_key = schedule.mint;
    let seed_bytes = schedule.seed.to_le_bytes();

    let signer_seeds: &[&[&[u8]]] = &[&[
        VestingSchedule::SEED,
        beneficiary.as_ref(),
        mint_key.as_ref(),
        &seed_bytes,
        &[schedule.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            token_program.key(),
            token_interface::TransferChecked {
                from: vault.to_account_info(),
                mint: mint.to_account_info(),
                to: destination.to_account_info(),
                authority: schedule.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        mint.decimals,
    )
}
