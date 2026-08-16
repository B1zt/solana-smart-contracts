use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface};

use crate::constants::MAX_COOLDOWN_SECONDS;
use crate::error::TokenPlatformError;
use crate::state::{StakeAccount, StakePool};

/// Create a staking pool.
#[derive(Accounts)]
pub struct InitializePool<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + StakePool::INIT_SPACE,
        seeds = [StakePool::SEED, stake_mint.key().as_ref(), authority.key().as_ref()],
        bump,
    )]
    pub pool: Account<'info, StakePool>,

    #[account(
        init,
        payer = authority,
        seeds = [StakePool::STAKE_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = stake_mint,
        token::authority = pool,
        token::token_program = token_program,
    )]
    pub stake_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init,
        payer = authority,
        seeds = [StakePool::REWARD_VAULT_SEED, pool.key().as_ref()],
        bump,
        token::mint = reward_mint,
        token::authority = pool,
        token::token_program = token_program,
    )]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,

    pub stake_mint: InterfaceAccount<'info, Mint>,
    pub reward_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_pool(
    ctx: Context<InitializePool>,
    reward_rate: u64,
    reward_duration: i64,
    cooldown_seconds: i64,
) -> Result<()> {
    require!(reward_duration > 0, TokenPlatformError::InvalidDuration);
    require!(cooldown_seconds >= 0, TokenPlatformError::InvalidDuration);
    require!(
        cooldown_seconds <= MAX_COOLDOWN_SECONDS,
        TokenPlatformError::CooldownTooLong
    );

    let now = Clock::get()?.unix_timestamp;

    let pool = &mut ctx.accounts.pool;
    pool.authority = ctx.accounts.authority.key();
    pool.stake_mint = ctx.accounts.stake_mint.key();
    pool.reward_mint = ctx.accounts.reward_mint.key();
    pool.stake_vault = ctx.accounts.stake_vault.key();
    pool.reward_vault = ctx.accounts.reward_vault.key();
    pool.total_staked = 0;
    pool.reward_rate = reward_rate;
    pool.reward_per_token = 0;
    pool.last_update_ts = now;
    pool.reward_end_ts = now
        .checked_add(reward_duration)
        .ok_or(TokenPlatformError::MathOverflow)?;
    pool.cooldown_seconds = cooldown_seconds;
    pool.pending_rewards = 0;
    pool.bump = ctx.bumps.pool;

    Ok(())
}

/// Advance the reward accumulator to `now`.
///
/// The heart of the whole design. Instead of tracking what each staker is owed, the pool tracks one
/// running figure: rewards accrued per staked token since inception. A staker's balance is that
/// figure minus whatever it was when they last touched the pool, multiplied by their stake.
///
/// Nothing here iterates over stakers, which is not merely an optimisation on Solana: a transaction
/// can only touch a bounded set of accounts, so a design that loops over participants stops working
/// the moment the pool gets popular.
fn accrue(pool: &mut StakePool, now: i64) -> Result<()> {
    let accrual_now = pool.accrual_ts(now);

    if accrual_now <= pool.last_update_ts {
        return Ok(());
    }

    // Nothing staked means nothing accrues, but the clock still moves. Skipping the timestamp
    // update would later pay out rewards for a period when nobody was staked.
    if pool.total_staked == 0 {
        pool.last_update_ts = accrual_now;
        return Ok(());
    }

    let elapsed = accrual_now.saturating_sub(pool.last_update_ts) as u128;
    let emitted = elapsed.saturating_mul(pool.reward_rate as u128);

    pool.reward_per_token = pool
        .reward_per_token
        .checked_add(
            emitted
                .checked_mul(StakePool::ACC_PRECISION)
                .ok_or(TokenPlatformError::MathOverflow)?
                / pool.total_staked as u128,
        )
        .ok_or(TokenPlatformError::MathOverflow)?;

    pool.last_update_ts = accrual_now;

    Ok(())
}

/// Settle one staker against the current accumulator.
fn settle(pool: &StakePool, stake: &mut StakeAccount) -> Result<()> {
    if stake.amount > 0 {
        let owed = (stake.amount as u128)
            .checked_mul(pool.reward_per_token.saturating_sub(stake.reward_debt))
            .ok_or(TokenPlatformError::MathOverflow)?
            / StakePool::ACC_PRECISION;

        stake.pending_reward = stake
            .pending_reward
            .checked_add(owed as u64)
            .ok_or(TokenPlatformError::MathOverflow)?;
    }

    stake.reward_debt = pool.reward_per_token;

    Ok(())
}

#[derive(Accounts)]
pub struct Stake<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [StakePool::SEED, pool.stake_mint.as_ref(), pool.authority.as_ref()],
        bump = pool.bump,
        has_one = stake_vault,
    )]
    pub pool: Account<'info, StakePool>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + StakeAccount::INIT_SPACE,
        seeds = [StakeAccount::SEED, pool.key().as_ref(), owner.key().as_ref()],
        bump,
    )]
    pub stake_account: Account<'info, StakeAccount>,

    #[account(mut)]
    pub stake_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = stake_mint, token::authority = owner)]
    pub owner_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = pool.stake_mint)]
    pub stake_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_stake(ctx: Context<Stake>, amount: u64) -> Result<()> {
    require!(amount > 0, TokenPlatformError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;

    // Accrue before touching balances, so the depositor cannot claim rewards from before they
    // arrived and existing stakers keep everything earned up to this moment.
    accrue(&mut ctx.accounts.pool, now)?;

    let stake = &mut ctx.accounts.stake_account;

    // Fresh account: `init_if_needed` leaves it zeroed, so identity is set on first use.
    if stake.owner == Pubkey::default() {
        stake.owner = ctx.accounts.owner.key();
        stake.pool = ctx.accounts.pool.key();
        stake.bump = ctx.bumps.stake_account;
    } else {
        // Guard against `init_if_needed` reusing an account that belongs to somebody else. The PDA
        // seeds already bind it to this owner and pool, so this is belt and braces, but the
        // combination of init_if_needed and a missing ownership check is a known Anchor footgun.
        require_keys_eq!(stake.owner, ctx.accounts.owner.key(), TokenPlatformError::Unauthorized);
        require_keys_eq!(stake.pool, ctx.accounts.pool.key(), TokenPlatformError::Unauthorized);
    }

    settle(&ctx.accounts.pool, stake)?;

    // Measure what arrived rather than trusting `amount`. A Token-2022 mint with a transfer fee
    // delivers less than was sent, and crediting the requested figure would let the pool promise
    // more than it holds.
    let balance_before = ctx.accounts.stake_vault.amount;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.owner_token_account.to_account_info(),
                mint: ctx.accounts.stake_mint.to_account_info(),
                to: ctx.accounts.stake_vault.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.stake_mint.decimals,
    )?;

    ctx.accounts.stake_vault.reload()?;
    let received = ctx.accounts.stake_vault.amount.saturating_sub(balance_before);
    require!(received > 0, TokenPlatformError::ZeroAmount);

    let stake = &mut ctx.accounts.stake_account;
    stake.amount = stake
        .amount
        .checked_add(received)
        .ok_or(TokenPlatformError::MathOverflow)?;

    ctx.accounts.pool.total_staked = ctx
        .accounts
        .pool
        .total_staked
        .checked_add(received)
        .ok_or(TokenPlatformError::MathOverflow)?;

    Ok(())
}

/// Begin unstaking. Tokens move out of the earning balance and start the cooldown.
#[derive(Accounts)]
pub struct Unstake<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [StakePool::SEED, pool.stake_mint.as_ref(), pool.authority.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, StakePool>,

    #[account(
        mut,
        seeds = [StakeAccount::SEED, pool.key().as_ref(), owner.key().as_ref()],
        bump = stake_account.bump,
        has_one = owner @ TokenPlatformError::Unauthorized,
        has_one = pool @ TokenPlatformError::Unauthorized,
    )]
    pub stake_account: Account<'info, StakeAccount>,
}

pub fn handle_unstake(ctx: Context<Unstake>, amount: u64) -> Result<()> {
    require!(amount > 0, TokenPlatformError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;

    accrue(&mut ctx.accounts.pool, now)?;

    let stake = &mut ctx.accounts.stake_account;
    require!(stake.amount >= amount, TokenPlatformError::InsufficientStake);

    settle(&ctx.accounts.pool, stake)?;

    stake.amount = stake.amount.saturating_sub(amount);
    stake.unstaking_amount = stake
        .unstaking_amount
        .checked_add(amount)
        .ok_or(TokenPlatformError::MathOverflow)?;

    // Each unstake restarts the clock on the whole pending balance. Tracking a separate deadline
    // per tranche would need an unbounded list, which an account cannot hold.
    stake.unstake_ready_ts = now
        .checked_add(ctx.accounts.pool.cooldown_seconds)
        .ok_or(TokenPlatformError::MathOverflow)?;

    ctx.accounts.pool.total_staked = ctx.accounts.pool.total_staked.saturating_sub(amount);

    Ok(())
}

/// Withdraw unstaked tokens once the cooldown has elapsed.
#[derive(Accounts)]
pub struct WithdrawUnstaked<'info> {
    pub owner: Signer<'info>,

    #[account(
        seeds = [StakePool::SEED, pool.stake_mint.as_ref(), pool.authority.as_ref()],
        bump = pool.bump,
        has_one = stake_vault,
    )]
    pub pool: Account<'info, StakePool>,

    #[account(
        mut,
        seeds = [StakeAccount::SEED, pool.key().as_ref(), owner.key().as_ref()],
        bump = stake_account.bump,
        has_one = owner @ TokenPlatformError::Unauthorized,
        has_one = pool @ TokenPlatformError::Unauthorized,
    )]
    pub stake_account: Account<'info, StakeAccount>,

    #[account(mut)]
    pub stake_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = stake_mint, token::authority = owner)]
    pub owner_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = pool.stake_mint)]
    pub stake_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_withdraw_unstaked(ctx: Context<WithdrawUnstaked>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    let amount = ctx.accounts.stake_account.unstaking_amount;
    require!(amount > 0, TokenPlatformError::NothingToWithdraw);
    require!(
        now >= ctx.accounts.stake_account.unstake_ready_ts,
        TokenPlatformError::StillCoolingDown
    );

    ctx.accounts.stake_account.unstaking_amount = 0;
    ctx.accounts.stake_account.unstake_ready_ts = 0;

    let stake_mint = ctx.accounts.pool.stake_mint;
    let pool_authority = ctx.accounts.pool.authority;
    let signer_seeds: &[&[&[u8]]] = &[&[
        StakePool::SEED,
        stake_mint.as_ref(),
        pool_authority.as_ref(),
        &[ctx.accounts.pool.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.stake_vault.to_account_info(),
                mint: ctx.accounts.stake_mint.to_account_info(),
                to: ctx.accounts.owner_token_account.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.stake_mint.decimals,
    )?;

    Ok(())
}

/// Claim accrued rewards without touching the stake.
#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [StakePool::SEED, pool.stake_mint.as_ref(), pool.authority.as_ref()],
        bump = pool.bump,
        has_one = reward_vault,
    )]
    pub pool: Account<'info, StakePool>,

    #[account(
        mut,
        seeds = [StakeAccount::SEED, pool.key().as_ref(), owner.key().as_ref()],
        bump = stake_account.bump,
        has_one = owner @ TokenPlatformError::Unauthorized,
        has_one = pool @ TokenPlatformError::Unauthorized,
    )]
    pub stake_account: Account<'info, StakeAccount>,

    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = reward_mint, token::authority = owner)]
    pub owner_reward_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = pool.reward_mint)]
    pub reward_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    accrue(&mut ctx.accounts.pool, now)?;
    settle(&ctx.accounts.pool, &mut ctx.accounts.stake_account)?;

    let amount = ctx.accounts.stake_account.pending_reward;
    require!(amount > 0, TokenPlatformError::NothingToWithdraw);

    // Pay at most what the vault actually holds. A pool whose reward funding runs out should wind
    // down gracefully rather than making every claim, stake and unstake revert.
    let available = ctx.accounts.reward_vault.amount;
    let payout = amount.min(available);
    require!(payout > 0, TokenPlatformError::InsufficientRewardBalance);

    ctx.accounts.stake_account.pending_reward = amount.saturating_sub(payout);

    let stake_mint = ctx.accounts.pool.stake_mint;
    let pool_authority = ctx.accounts.pool.authority;
    let signer_seeds: &[&[&[u8]]] = &[&[
        StakePool::SEED,
        stake_mint.as_ref(),
        pool_authority.as_ref(),
        &[ctx.accounts.pool.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.reward_vault.to_account_info(),
                mint: ctx.accounts.reward_mint.to_account_info(),
                to: ctx.accounts.owner_reward_account.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            },
            signer_seeds,
        ),
        payout,
        ctx.accounts.reward_mint.decimals,
    )?;

    msg!("claimed {} rewards", payout);

    Ok(())
}

/// Top up the reward vault.
#[derive(Accounts)]
pub struct FundRewards<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,

    #[account(
        mut,
        seeds = [StakePool::SEED, pool.stake_mint.as_ref(), pool.authority.as_ref()],
        bump = pool.bump,
        has_one = reward_vault,
    )]
    pub pool: Account<'info, StakePool>,

    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = reward_mint, token::authority = funder)]
    pub funder_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = pool.reward_mint)]
    pub reward_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_fund_rewards(ctx: Context<FundRewards>, amount: u64, extend_seconds: i64) -> Result<()> {
    require!(amount > 0, TokenPlatformError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;

    // Settle at the old rate before changing the schedule, so the change is not applied
    // retroactively to a period that already accrued.
    accrue(&mut ctx.accounts.pool, now)?;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.funder_token_account.to_account_info(),
                mint: ctx.accounts.reward_mint.to_account_info(),
                to: ctx.accounts.reward_vault.to_account_info(),
                authority: ctx.accounts.funder.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.reward_mint.decimals,
    )?;

    if extend_seconds > 0 {
        let pool = &mut ctx.accounts.pool;

        // Extend from now if the schedule already lapsed, otherwise from where it ends. Extending
        // from a lapsed end time would silently shorten the new period.
        let base = if pool.reward_end_ts > now { pool.reward_end_ts } else { now };

        pool.reward_end_ts = base
            .checked_add(extend_seconds)
            .ok_or(TokenPlatformError::MathOverflow)?;

        // Restart accrual from now, so the gap while the schedule was lapsed pays nothing.
        pool.last_update_ts = now;
    }

    Ok(())
}
