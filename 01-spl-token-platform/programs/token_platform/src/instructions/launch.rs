use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface};

use crate::error::TokenPlatformError;
use crate::state::LaunchConfig;

/// Set up a token launch and hand the mint authority to a program-derived address.
///
/// The mint itself is created by the client, because creating a Token-2022 mint with extensions
/// requires sizing the account for exactly the extensions wanted, and that is a decision the
/// project makes rather than something this program should hardcode. What this instruction does is
/// the part that matters for safety: it takes custody of the mint authority so that from here on,
/// minting is bounded by the supply cap rather than by whoever holds a keypair.
#[derive(Accounts)]
pub struct InitializeLaunch<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + LaunchConfig::INIT_SPACE,
        seeds = [LaunchConfig::SEED, mint.key().as_ref()],
        bump,
    )]
    pub config: Account<'info, LaunchConfig>,

    /// The mint being governed.
    ///
    /// `mint::authority = authority` requires the caller to currently hold it, so a launch cannot
    /// be created over somebody else's token.
    #[account(mut, mint::authority = authority)]
    pub mint: InterfaceAccount<'info, Mint>,

    /// CHECK: PDA that becomes the mint authority. It holds no data and is never read; it exists
    /// purely so this program can sign mints. Derivation is constrained by the seeds below.
    #[account(seeds = [LaunchConfig::MINT_AUTHORITY_SEED, mint.key().as_ref()], bump)]
    pub mint_authority: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_launch(ctx: Context<InitializeLaunch>, supply_cap: u64) -> Result<()> {
    require!(supply_cap > 0, TokenPlatformError::InvalidSupplyCap);

    let mint_supply = ctx.accounts.mint.supply;
    require!(mint_supply <= supply_cap, TokenPlatformError::SupplyCapExceeded);

    let config = &mut ctx.accounts.config;
    config.authority = ctx.accounts.authority.key();
    config.mint = ctx.accounts.mint.key();
    config.supply_cap = supply_cap;
    // Any pre-existing supply counts against the cap. Ignoring it would let a project pre-mint
    // before creating the launch and then mint the full cap again on top.
    config.minted = mint_supply;
    config.minting_finished = false;
    config.bump = ctx.bumps.config;
    config.mint_authority_bump = ctx.bumps.mint_authority;

    // Transfer the mint authority to the PDA. After this the authority keypair cannot mint at all;
    // every mint must go through this program and respect the cap.
    token_interface::set_authority(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token_interface::SetAuthority {
                current_authority: ctx.accounts.authority.to_account_info(),
                account_or_mint: ctx.accounts.mint.to_account_info(),
            },
        ),
        anchor_spl::token_2022::spl_token_2022::instruction::AuthorityType::MintTokens,
        Some(ctx.accounts.mint_authority.key()),
    )?;

    msg!(
        "launch initialised: cap {}, already minted {}",
        supply_cap,
        mint_supply
    );

    Ok(())
}

/// Mint tokens, bounded by the cap.
#[derive(Accounts)]
pub struct MintTokens<'info> {
    #[account(address = config.authority @ TokenPlatformError::Unauthorized)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [LaunchConfig::SEED, mint.key().as_ref()],
        bump = config.bump,
        has_one = mint,
    )]
    pub config: Account<'info, LaunchConfig>,

    #[account(mut)]
    pub mint: InterfaceAccount<'info, Mint>,

    /// CHECK: the mint authority PDA, validated by seeds and signing via `signer_seeds`.
    #[account(
        seeds = [LaunchConfig::MINT_AUTHORITY_SEED, mint.key().as_ref()],
        bump = config.mint_authority_bump,
    )]
    pub mint_authority: UncheckedAccount<'info>,

    #[account(mut, token::mint = mint)]
    pub destination: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_mint_tokens(ctx: Context<MintTokens>, amount: u64) -> Result<()> {
    require!(amount > 0, TokenPlatformError::ZeroAmount);

    let config = &ctx.accounts.config;
    require!(!config.minting_finished, TokenPlatformError::MintingFinished);

    let new_minted = config
        .minted
        .checked_add(amount)
        .ok_or(TokenPlatformError::MathOverflow)?;
    require!(
        new_minted <= config.supply_cap,
        TokenPlatformError::SupplyCapExceeded
    );

    let mint_key = ctx.accounts.mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        LaunchConfig::MINT_AUTHORITY_SEED,
        mint_key.as_ref(),
        &[config.mint_authority_bump],
    ]];

    token_interface::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::MintTo {
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.mint_authority.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )?;

    ctx.accounts.config.minted = new_minted;

    Ok(())
}

/// Permanently disable minting.
///
/// One-way, and stronger than reassigning the mint authority: the flag is checked on every mint, so
/// even a future authority change cannot reopen it through this program.
#[derive(Accounts)]
pub struct FinishMinting<'info> {
    #[account(address = config.authority @ TokenPlatformError::Unauthorized)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [LaunchConfig::SEED, config.mint.as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, LaunchConfig>,
}

pub fn handle_finish_minting(ctx: Context<FinishMinting>) -> Result<()> {
    require!(
        !ctx.accounts.config.minting_finished,
        TokenPlatformError::MintingFinished
    );

    ctx.accounts.config.minting_finished = true;
    msg!("minting permanently disabled at supply {}", ctx.accounts.config.minted);

    Ok(())
}
