use anchor_lang::prelude::*;
use solana_keccak_hasher as keccak;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface};

use crate::constants::MAX_PROOF_LENGTH;
use crate::error::TokenPlatformError;
use crate::state::{ClaimStatus, Distributor};

/// Create and fund a Merkle airdrop.
#[derive(Accounts)]
pub struct InitializeDistributor<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Distributor::INIT_SPACE,
        seeds = [Distributor::SEED, mint.key().as_ref(), &merkle_root_seed(&authority.key())],
        bump,
    )]
    pub distributor: Account<'info, Distributor>,

    #[account(
        init,
        payer = authority,
        seeds = [Distributor::VAULT_SEED, distributor.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = distributor,
        token::token_program = token_program,
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = mint, token::authority = authority)]
    pub funding_account: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// One distributor per authority per mint.
///
/// Keeping the authority in the seed lets several projects run airdrops for the same token without
/// their PDAs colliding, which a mint-only seed would not allow.
fn merkle_root_seed(authority: &Pubkey) -> [u8; 32] {
    authority.to_bytes()
}

pub fn handle_initialize_distributor(
    ctx: Context<InitializeDistributor>,
    merkle_root: [u8; 32],
    total_amount: u64,
    claim_deadline: i64,
) -> Result<()> {
    require!(total_amount > 0, TokenPlatformError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    require!(claim_deadline > now, TokenPlatformError::TimestampInPast);

    let distributor = &mut ctx.accounts.distributor;
    distributor.authority = ctx.accounts.authority.key();
    distributor.mint = ctx.accounts.mint.key();
    distributor.vault = ctx.accounts.vault.key();
    distributor.merkle_root = merkle_root;
    distributor.total_amount = total_amount;
    distributor.claimed_amount = 0;
    distributor.claim_count = 0;
    distributor.claim_deadline = claim_deadline;
    distributor.bump = ctx.bumps.distributor;

    // Funded in the same instruction. An airdrop that is announced but not funded takes claims and
    // fails on the first one.
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
        total_amount,
        ctx.accounts.mint.decimals,
    )?;

    Ok(())
}

/// Claim an airdrop allocation.
///
/// Permissionless in who submits it, but the tokens always go to the account owned by `claimant`,
/// which is the address committed to in the leaf. A relayer can pay the fee on a user's behalf and
/// cannot redirect a single token.
#[derive(Accounts)]
#[instruction(index: u64)]
pub struct ClaimAirdrop<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: the address committed to in the Merkle leaf. It never signs, because a relayer may
    /// submit on its behalf; the leaf binding is what makes that safe.
    pub claimant: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [Distributor::SEED, distributor.mint.as_ref(), distributor.authority.as_ref()],
        bump = distributor.bump,
        has_one = vault,
        has_one = mint,
    )]
    pub distributor: Account<'info, Distributor>,

    /// Existence of this account **is** the claimed flag.
    ///
    /// `init` fails at the runtime level if the PDA already exists, so a replayed claim is rejected
    /// before the program checks anything. This is Solana's equivalent of a claim bitmap, and it is
    /// stronger: there is no flag to forget to set.
    #[account(
        init,
        payer = payer,
        space = 8 + ClaimStatus::INIT_SPACE,
        seeds = [ClaimStatus::SEED, distributor.key().as_ref(), &index.to_le_bytes()],
        bump,
    )]
    pub claim_status: Account<'info, ClaimStatus>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    /// Destination, constrained to an account the claimant owns.
    #[account(
        mut,
        token::mint = mint,
        token::authority = claimant,
    )]
    pub claimant_token_account: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_claim_airdrop(
    ctx: Context<ClaimAirdrop>,
    index: u64,
    amount: u64,
    proof: Vec<[u8; 32]>,
) -> Result<()> {
    require!(proof.len() <= MAX_PROOF_LENGTH, TokenPlatformError::ProofTooLong);
    require!(amount > 0, TokenPlatformError::ZeroAmount);

    let now = Clock::get()?.unix_timestamp;
    require!(
        now <= ctx.accounts.distributor.claim_deadline,
        TokenPlatformError::ClaimWindowClosed
    );

    // Leaf commits to index, claimant and amount together. The index makes each leaf unique so the
    // claim PDA is unique; the claimant binds the allocation so a proof cannot be redirected; the
    // amount stops a valid proof being reused for a larger sum.
    let leaf = leaf_hash(index, &ctx.accounts.claimant.key(), amount);

    require!(
        verify_proof(&proof, ctx.accounts.distributor.merkle_root, leaf),
        TokenPlatformError::InvalidProof
    );

    let claim_status = &mut ctx.accounts.claim_status;
    claim_status.claimant = ctx.accounts.claimant.key();
    claim_status.amount = amount;
    claim_status.claimed_at = now;
    claim_status.bump = ctx.bumps.claim_status;

    let distributor = &mut ctx.accounts.distributor;
    distributor.claimed_amount = distributor
        .claimed_amount
        .checked_add(amount)
        .ok_or(TokenPlatformError::MathOverflow)?;
    distributor.claim_count = distributor
        .claim_count
        .checked_add(1)
        .ok_or(TokenPlatformError::MathOverflow)?;

    let mint_key = distributor.mint;
    let authority_key = distributor.authority;
    let signer_seeds: &[&[&[u8]]] = &[&[
        Distributor::SEED,
        mint_key.as_ref(),
        authority_key.as_ref(),
        &[distributor.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.claimant_token_account.to_account_info(),
                authority: ctx.accounts.distributor.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    msg!("claim {} paid {} to {}", index, amount, ctx.accounts.claimant.key());

    Ok(())
}

/// Recover unclaimed tokens once the window has closed.
#[derive(Accounts)]
pub struct ClawbackAirdrop<'info> {
    #[account(mut, address = distributor.authority @ TokenPlatformError::Unauthorized)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [Distributor::SEED, distributor.mint.as_ref(), distributor.authority.as_ref()],
        bump = distributor.bump,
        has_one = vault,
        has_one = mint,
    )]
    pub distributor: Account<'info, Distributor>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = mint, token::authority = authority)]
    pub authority_token_account: InterfaceAccount<'info, TokenAccount>,

    pub mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_clawback_airdrop(ctx: Context<ClawbackAirdrop>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    // Gated on the published deadline. Without this check, an authority could drain the vault the
    // moment claims looked slow, which makes the whole airdrop untrustworthy.
    require!(
        now > ctx.accounts.distributor.claim_deadline,
        TokenPlatformError::ClaimWindowOpen
    );

    let amount = ctx.accounts.vault.amount;
    require!(amount > 0, TokenPlatformError::NothingToWithdraw);

    let mint_key = ctx.accounts.distributor.mint;
    let authority_key = ctx.accounts.distributor.authority;
    let signer_seeds: &[&[&[u8]]] = &[&[
        Distributor::SEED,
        mint_key.as_ref(),
        authority_key.as_ref(),
        &[ctx.accounts.distributor.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.authority_token_account.to_account_info(),
                authority: ctx.accounts.distributor.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;

    msg!("clawed back {} unclaimed tokens", amount);

    Ok(())
}

/// `keccak(keccak(index || claimant || amount))`.
///
/// Double hashed for the same reason as on EVM: a single-hashed leaf is 32 bytes and so is an
/// internal node, so an attacker who sees a published proof could present an internal node as a
/// leaf and forge membership. Hashing twice makes the two domains distinguishable.
pub fn leaf_hash(index: u64, claimant: &Pubkey, amount: u64) -> [u8; 32] {
    let inner = keccak::hashv(&[
        &index.to_le_bytes(),
        claimant.as_ref(),
        &amount.to_le_bytes(),
    ]);

    keccak::hash(inner.as_ref()).to_bytes()
}

/// Verify a Merkle proof with sorted-pair hashing.
///
/// Pairs are ordered before hashing, which is why the proof carries no left/right flags. It has to
/// match whatever the off-chain tree builder does exactly; a mismatch produces proofs that look
/// well-formed and fail every single time.
pub fn verify_proof(proof: &[[u8; 32]], root: [u8; 32], leaf: [u8; 32]) -> bool {
    let mut computed = leaf;

    for sibling in proof {
        computed = if computed <= *sibling {
            keccak::hashv(&[&computed, sibling]).to_bytes()
        } else {
            keccak::hashv(&[sibling, &computed]).to_bytes()
        };
    }

    computed == root
}
