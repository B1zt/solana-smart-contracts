use anchor_lang::prelude::*;

/// A collection being minted.
#[account]
#[derive(InitSpace)]
pub struct Collection {
    pub authority: Pubkey,

    /// Where mint proceeds go.
    pub treasury: Pubkey,

    /// Hard cap on how many can ever be minted.
    pub max_supply: u32,

    /// Minted so far.
    pub minted: u32,

    /// Royalty taken on secondary sales, in basis points.
    ///
    /// Capped at `MAX_ROYALTY_BPS`. Solana has no protocol-level royalty enforcement, so this is
    /// honoured by this marketplace and by nobody else. That limitation is real and is stated
    /// plainly rather than implied away.
    pub royalty_bps: u16,

    /// Who receives royalties.
    pub royalty_recipient: Pubkey,

    /// Collection name, used for the derived metadata URI.
    #[max_len(32)]
    pub name: String,

    /// Base URI. Token metadata lives at `{base_uri}{index}.json`.
    #[max_len(128)]
    pub base_uri: String,

    /// Number of configured mint phases.
    pub phase_count: u8,

    pub bump: u8,
}

impl Collection {
    pub const SEED: &'static [u8] = b"collection";

    /// Ceiling on royalties. Ten percent, and the authority cannot exceed it.
    pub const MAX_ROYALTY_BPS: u16 = 1_000;
}

/// One mint phase.
///
/// Phases are separate accounts rather than a vector inside `Collection`. A Solana account has a
/// fixed size decided at creation, so an inline vector would have to be sized for the maximum number
/// of phases up front and would pay rent for that space forever. Separate accounts also mean adding
/// a phase does not rewrite the collection account that every mint reads.
#[account]
#[derive(InitSpace)]
pub struct MintPhase {
    pub collection: Pubkey,

    /// Index within the collection.
    pub index: u8,

    /// Allowlist root over `(wallet, allowance)` leaves. Zero means the phase is public.
    pub merkle_root: [u8; 32],

    /// Price per mint in lamports. Zero is a valid free mint.
    pub price: u64,

    /// Inclusive start.
    pub start_ts: i64,

    /// Exclusive end.
    pub end_ts: i64,

    /// Per-wallet cap for public phases. Ignored when a root is set, since the leaf carries it.
    pub max_per_wallet: u16,

    /// Cap on mints in this phase. Zero means bounded only by the collection's max supply.
    pub max_supply: u32,

    /// Minted in this phase.
    pub minted: u32,

    pub bump: u8,
}

impl MintPhase {
    pub const SEED: &'static [u8] = b"phase";
}

/// How many a wallet has minted in one phase.
///
/// A separate account per (phase, wallet). On EVM this would be a mapping entry; on Solana every
/// piece of state needs an account, and the rent is paid by whoever mints. That is the trade-off
/// for Solana's flat, parallelisable account model.
#[account]
#[derive(InitSpace)]
pub struct MintReceipt {
    pub wallet: Pubkey,
    pub phase: Pubkey,
    pub minted: u16,
    pub bump: u8,
}

impl MintReceipt {
    pub const SEED: &'static [u8] = b"receipt";
}

/// An NFT listed for sale.
///
/// The NFT itself is escrowed in a program-owned token account while listed. A signature-based
/// listing that leaves the seller in custody looks cheaper, but the seller can move the asset
/// mid-listing and every purchase then fails; escrow means a buyer who pays always receives it.
#[account]
#[derive(InitSpace)]
pub struct Listing {
    pub seller: Pubkey,

    /// Mint of the NFT being sold.
    pub mint: Pubkey,

    /// Program-owned account holding the NFT while listed.
    pub escrow: Pubkey,

    /// Collection this NFT belongs to, so royalties can be resolved.
    pub collection: Pubkey,

    /// Asking price in lamports.
    pub price: u64,

    pub created_ts: i64,

    pub bump: u8,
}

impl Listing {
    pub const SEED: &'static [u8] = b"listing";
    pub const ESCROW_SEED: &'static [u8] = b"listing-escrow";
}

/// A standing offer on an NFT, backed by escrowed lamports.
///
/// The lamports are held by the offer PDA itself rather than promised. An offer whose funds are
/// merely approved can evaporate the moment the seller accepts, which wastes the seller's fee and
/// makes every offer untrustworthy.
#[account]
#[derive(InitSpace)]
pub struct Offer {
    pub buyer: Pubkey,
    pub mint: Pubkey,

    /// Lamports escrowed for this offer.
    pub amount: u64,

    /// After this the offer can be reclaimed by the buyer.
    pub expires_ts: i64,

    pub bump: u8,
}

impl Offer {
    pub const SEED: &'static [u8] = b"offer";
}

/// Marketplace-wide configuration.
#[account]
#[derive(InitSpace)]
pub struct Marketplace {
    pub authority: Pubkey,

    /// Where protocol fees go.
    pub fee_recipient: Pubkey,

    /// Protocol fee in basis points, capped at `MAX_FEE_BPS`.
    pub fee_bps: u16,

    /// Cumulative volume, for reporting.
    pub total_volume: u64,

    pub total_sales: u64,

    pub bump: u8,
}

impl Marketplace {
    pub const SEED: &'static [u8] = b"marketplace";

    /// Ceiling on the protocol fee. Five percent, and the authority cannot exceed it.
    pub const MAX_FEE_BPS: u16 = 500;
}
