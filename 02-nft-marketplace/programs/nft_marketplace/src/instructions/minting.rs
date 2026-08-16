use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface};
use solana_keccak_hasher as keccak;

use crate::error::MarketplaceError;
use crate::state::{Collection, MintPhase, MintReceipt};

/// Create a collection.
#[derive(Accounts)]
// `name` is an instruction argument used in the PDA seeds, so Anchor needs it declared here to
// generate the derivation check.
#[instruction(name: String)]
pub struct InitializeCollection<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Collection::INIT_SPACE,
        seeds = [Collection::SEED, authority.key().as_ref(), name.as_bytes()],
        bump,
    )]
    pub collection: Account<'info, Collection>,

    /// CHECK: only stored as a payout destination. It never signs and is never read.
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: only stored as a royalty destination.
    pub royalty_recipient: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_collection(
    ctx: Context<InitializeCollection>,
    name: String,
    base_uri: String,
    max_supply: u32,
    royalty_bps: u16,
) -> Result<()> {
    require!(max_supply > 0, MarketplaceError::InvalidSupply);
    require!(name.len() <= 32, MarketplaceError::NameTooLong);
    require!(base_uri.len() <= 128, MarketplaceError::UriTooLong);
    require!(
        royalty_bps <= Collection::MAX_ROYALTY_BPS,
        MarketplaceError::RoyaltyTooHigh
    );

    let collection = &mut ctx.accounts.collection;
    collection.authority = ctx.accounts.authority.key();
    collection.treasury = ctx.accounts.treasury.key();
    collection.max_supply = max_supply;
    collection.minted = 0;
    collection.royalty_bps = royalty_bps;
    collection.royalty_recipient = ctx.accounts.royalty_recipient.key();
    collection.name = name;
    collection.base_uri = base_uri;
    collection.phase_count = 0;
    collection.bump = ctx.bumps.collection;

    Ok(())
}

/// Add a mint phase.
#[derive(Accounts)]
pub struct AddPhase<'info> {
    #[account(mut, address = collection.authority @ MarketplaceError::Unauthorized)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [Collection::SEED, collection.authority.as_ref(), collection.name.as_bytes()],
        bump = collection.bump,
    )]
    pub collection: Account<'info, Collection>,

    #[account(
        init,
        payer = authority,
        space = 8 + MintPhase::INIT_SPACE,
        seeds = [MintPhase::SEED, collection.key().as_ref(), &[collection.phase_count]],
        bump,
    )]
    pub phase: Account<'info, MintPhase>,

    pub system_program: Program<'info, System>,
}

pub fn handle_add_phase(
    ctx: Context<AddPhase>,
    merkle_root: [u8; 32],
    price: u64,
    start_ts: i64,
    end_ts: i64,
    max_per_wallet: u16,
    max_supply: u32,
) -> Result<()> {
    require!(start_ts < end_ts, MarketplaceError::InvalidPhaseWindow);

    // A public phase with no per-wallet cap lets one buyer take the whole drop.
    require!(
        merkle_root != [0u8; 32] || max_per_wallet > 0,
        MarketplaceError::InvalidPhaseWindow
    );

    let index = ctx.accounts.collection.phase_count;

    let phase = &mut ctx.accounts.phase;
    phase.collection = ctx.accounts.collection.key();
    phase.index = index;
    phase.merkle_root = merkle_root;
    phase.price = price;
    phase.start_ts = start_ts;
    phase.end_ts = end_ts;
    phase.max_per_wallet = max_per_wallet;
    phase.max_supply = max_supply;
    phase.minted = 0;
    phase.bump = ctx.bumps.phase;

    ctx.accounts.collection.phase_count = index
        .checked_add(1)
        .ok_or(MarketplaceError::MathOverflow)?;

    Ok(())
}

/// Mint one NFT from a phase.
///
/// The mint account is created by the client and passed in, because a Solana NFT is a mint with a
/// supply of one and zero decimals, and creating it is a System plus Token program interaction the
/// wallet does anyway. What this instruction owns is the part that needs enforcing: eligibility,
/// caps, payment and the actual mint.
#[derive(Accounts)]
pub struct MintNft<'info> {
    #[account(mut)]
    pub minter: Signer<'info>,

    #[account(
        mut,
        seeds = [Collection::SEED, collection.authority.as_ref(), collection.name.as_bytes()],
        bump = collection.bump,
    )]
    pub collection: Account<'info, Collection>,

    #[account(
        mut,
        seeds = [MintPhase::SEED, collection.key().as_ref(), &[phase.index]],
        bump = phase.bump,
        has_one = collection @ MarketplaceError::Unauthorized,
    )]
    pub phase: Account<'info, MintPhase>,

    /// Per-wallet mint count for this phase.
    #[account(
        init_if_needed,
        payer = minter,
        space = 8 + MintReceipt::INIT_SPACE,
        seeds = [MintReceipt::SEED, phase.key().as_ref(), minter.key().as_ref()],
        bump,
    )]
    pub receipt: Account<'info, MintReceipt>,

    /// The NFT mint, created by the client with the collection PDA as its authority.
    #[account(
        mut,
        mint::decimals = 0,
        mint::authority = collection,
    )]
    pub nft_mint: InterfaceAccount<'info, Mint>,

    #[account(
        init_if_needed,
        payer = minter,
        associated_token::mint = nft_mint,
        associated_token::authority = minter,
        associated_token::token_program = token_program,
    )]
    pub minter_token_account: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: payout destination, checked against the collection's stored treasury.
    #[account(mut, address = collection.treasury @ MarketplaceError::Unauthorized)]
    pub treasury: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_mint_nft(
    ctx: Context<MintNft>,
    allowance: u16,
    proof: Vec<[u8; 32]>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;

    require!(
        now >= ctx.accounts.phase.start_ts && now < ctx.accounts.phase.end_ts,
        MarketplaceError::PhaseNotActive
    );
    require!(proof.len() <= 32, MarketplaceError::ProofTooLong);

    // Eligibility. A public phase uses the phase's per-wallet cap; a gated phase takes the cap from
    // the Merkle leaf, so one root can express per-wallet tiers.
    let wallet_cap = if ctx.accounts.phase.merkle_root == [0u8; 32] {
        // Rejecting a stray proof rather than ignoring it surfaces frontend bugs immediately.
        require!(proof.is_empty(), MarketplaceError::ProofNotRequired);
        ctx.accounts.phase.max_per_wallet
    } else {
        let leaf = leaf_hash(&ctx.accounts.minter.key(), allowance);
        require!(
            verify_proof(&proof, ctx.accounts.phase.merkle_root, leaf),
            MarketplaceError::InvalidProof
        );
        allowance
    };

    // Receipt is zeroed on first use by `init_if_needed`.
    let receipt = &mut ctx.accounts.receipt;
    if receipt.wallet == Pubkey::default() {
        receipt.wallet = ctx.accounts.minter.key();
        receipt.phase = ctx.accounts.phase.key();
        receipt.bump = ctx.bumps.receipt;
    } else {
        // `init_if_needed` without an ownership check is a known Anchor footgun. The seeds already
        // bind this account, so this is belt and braces.
        require_keys_eq!(receipt.wallet, ctx.accounts.minter.key(), MarketplaceError::Unauthorized);
        require_keys_eq!(receipt.phase, ctx.accounts.phase.key(), MarketplaceError::Unauthorized);
    }

    require!(receipt.minted < wallet_cap, MarketplaceError::WalletLimitReached);

    // Phase cap. Zero means bounded only by the collection.
    if ctx.accounts.phase.max_supply > 0 {
        require!(
            ctx.accounts.phase.minted < ctx.accounts.phase.max_supply,
            MarketplaceError::PhaseSoldOut
        );
    }

    require!(
        ctx.accounts.collection.minted < ctx.accounts.collection.max_supply,
        MarketplaceError::SoldOut
    );

    // Payment. Lamports go straight to the treasury rather than through a program vault: there is
    // nothing to hold them for, and a vault would need a withdrawal path that could go wrong.
    let price = ctx.accounts.phase.price;
    if price > 0 {
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.key(),
                system_program::Transfer {
                    from: ctx.accounts.minter.to_account_info(),
                    to: ctx.accounts.treasury.to_account_info(),
                },
            ),
            price,
        )?;
    }

    // Counters before the mint.
    receipt.minted = receipt.minted.checked_add(1).ok_or(MarketplaceError::MathOverflow)?;
    ctx.accounts.phase.minted = ctx
        .accounts
        .phase
        .minted
        .checked_add(1)
        .ok_or(MarketplaceError::MathOverflow)?;
    ctx.accounts.collection.minted = ctx
        .accounts
        .collection
        .minted
        .checked_add(1)
        .ok_or(MarketplaceError::MathOverflow)?;

    let authority = ctx.accounts.collection.authority;
    let name = ctx.accounts.collection.name.clone();
    let signer_seeds: &[&[&[u8]]] = &[&[
        Collection::SEED,
        authority.as_ref(),
        name.as_bytes(),
        &[ctx.accounts.collection.bump],
    ]];

    // Exactly one, which with zero decimals is what makes this mint an NFT.
    token_interface::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::MintTo {
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.minter_token_account.to_account_info(),
                authority: ctx.accounts.collection.to_account_info(),
            },
            signer_seeds,
        ),
        1,
    )?;

    msg!(
        "minted {} of {} from phase {}",
        ctx.accounts.collection.minted,
        ctx.accounts.collection.max_supply,
        ctx.accounts.phase.index
    );

    Ok(())
}

/// `keccak(keccak(wallet || allowance_le))`.
///
/// Double hashed so a 32-byte internal node cannot be presented as a leaf, and little-endian to
/// match Rust's `to_le_bytes`, which the off-chain builder has to mirror exactly.
pub fn leaf_hash(wallet: &Pubkey, allowance: u16) -> [u8; 32] {
    let inner = keccak::hashv(&[wallet.as_ref(), &allowance.to_le_bytes()]);
    keccak::hash(inner.as_ref()).to_bytes()
}

/// Sorted-pair Merkle verification.
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
