use anchor_lang::prelude::*;

#[error_code]
pub enum MarketplaceError {
    /* ------------------------------------------------------------ minting --- */
    #[msg("Max supply must be greater than zero")]
    InvalidSupply,
    #[msg("Collection name exceeds 32 bytes")]
    NameTooLong,
    #[msg("Base URI exceeds 128 bytes")]
    UriTooLong,
    #[msg("Royalty exceeds the maximum allowed")]
    RoyaltyTooHigh,
    #[msg("Phase window is invalid")]
    InvalidPhaseWindow,
    #[msg("This phase is not currently active")]
    PhaseNotActive,
    #[msg("Wallet has reached its limit for this phase")]
    WalletLimitReached,
    #[msg("This phase has sold out")]
    PhaseSoldOut,
    #[msg("The collection has sold out")]
    SoldOut,
    #[msg("Merkle proof did not verify")]
    InvalidProof,
    #[msg("A proof was supplied for a public phase")]
    ProofNotRequired,
    #[msg("Proof exceeds the maximum supported depth")]
    ProofTooLong,

    /* ------------------------------------------------------------ trading --- */
    #[msg("Price must be greater than zero")]
    ZeroPrice,
    #[msg("Protocol fee exceeds the maximum allowed")]
    FeeTooHigh,
    #[msg("Signer does not hold this NFT")]
    NotTokenOwner,
    #[msg("Buyer and seller cannot be the same account")]
    SelfTrade,
    #[msg("This offer has expired")]
    OfferExpired,
    #[msg("Escrow does not hold enough lamports")]
    InsufficientEscrow,

    /* ------------------------------------------------------------- shared --- */
    #[msg("Arithmetic overflowed")]
    MathOverflow,
    #[msg("Caller is not authorised for this action")]
    Unauthorized,
}
