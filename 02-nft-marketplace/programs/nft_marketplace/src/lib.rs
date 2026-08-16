//! # NFT Marketplace
//!
//! Collection minting with Merkle allowlist phases, and a marketplace where listed NFTs are held in
//! program-owned escrow.
//!
//! Two things here are specifically Solana-shaped rather than ported from Ethereum:
//!
//! **State is spread across accounts, not packed into one.** Mint phases and per-wallet counters are
//! separate PDAs rather than fields on the collection. A Solana account is fixed-size at creation,
//! so an inline vector would pay rent for its maximum size forever, and every mint would contend on
//! writing the same account. Splitting them lets unrelated mints proceed in parallel, which is the
//! whole reason Solana's account model looks the way it does.
//!
//! **Royalties are a convention, not a guarantee.** Solana has no protocol-level royalty
//! enforcement. This marketplace reads the collection's royalty setting and pays it; a different
//! marketplace can simply not. That is a property of the chain, and the README says so rather than
//! implying creators are protected.
//!
//! The companion `escrow_native` program in this workspace implements a minimal escrow without
//! Anchor, to show what the framework's `#[derive(Accounts)]` blocks are actually generating.

pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use instructions::*;
pub use state::*;

declare_id!("MKTPmzcYRt3EWo5oPgQsUt1V5AqQbEPQeq7pQBhMqRj");

#[program]
pub mod nft_marketplace {
    use super::*;

    /* ------------------------------------------------------------ minting --- */

    /// Create a collection with a supply cap and royalty setting.
    pub fn initialize_collection(
        ctx: Context<InitializeCollection>,
        name: String,
        base_uri: String,
        max_supply: u32,
        royalty_bps: u16,
    ) -> Result<()> {
        instructions::minting::handle_initialize_collection(
            ctx,
            name,
            base_uri,
            max_supply,
            royalty_bps,
        )
    }

    /// Add a mint phase, optionally gated by a Merkle allowlist.
    pub fn add_phase(
        ctx: Context<AddPhase>,
        merkle_root: [u8; 32],
        price: u64,
        start_ts: i64,
        end_ts: i64,
        max_per_wallet: u16,
        max_supply: u32,
    ) -> Result<()> {
        instructions::minting::handle_add_phase(
            ctx,
            merkle_root,
            price,
            start_ts,
            end_ts,
            max_per_wallet,
            max_supply,
        )
    }

    /// Mint one NFT from a phase.
    pub fn mint_nft(ctx: Context<MintNft>, allowance: u16, proof: Vec<[u8; 32]>) -> Result<()> {
        instructions::minting::handle_mint_nft(ctx, allowance, proof)
    }

    /* ------------------------------------------------------------ trading --- */

    /// Create the marketplace configuration.
    pub fn initialize_marketplace(ctx: Context<InitializeMarketplace>, fee_bps: u16) -> Result<()> {
        instructions::trading::handle_initialize_marketplace(ctx, fee_bps)
    }

    /// List an NFT, escrowing it in a program-owned account.
    pub fn list_nft(ctx: Context<ListNft>, price: u64) -> Result<()> {
        instructions::trading::handle_list_nft(ctx, price)
    }

    /// Buy a listed NFT. Splits the price into fee, royalty and seller proceeds.
    pub fn buy_nft(ctx: Context<BuyNft>) -> Result<()> {
        instructions::trading::handle_buy_nft(ctx)
    }

    /// Cancel a listing and reclaim the NFT.
    pub fn delist_nft(ctx: Context<DelistNft>) -> Result<()> {
        instructions::trading::handle_delist_nft(ctx)
    }

    /// Make an offer, escrowing the lamports.
    pub fn make_offer(ctx: Context<MakeOffer>, amount: u64, expires_ts: i64) -> Result<()> {
        instructions::trading::handle_make_offer(ctx, amount, expires_ts)
    }

    /// Withdraw an offer and reclaim the lamports.
    pub fn cancel_offer(ctx: Context<CancelOffer>) -> Result<()> {
        instructions::trading::handle_cancel_offer(ctx)
    }

    /// Accept an offer.
    pub fn accept_offer(ctx: Context<AcceptOffer>) -> Result<()> {
        instructions::trading::handle_accept_offer(ctx)
    }
}
