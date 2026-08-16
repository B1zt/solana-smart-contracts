use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface};

use crate::error::MarketplaceError;
use crate::state::{Collection, Listing, Marketplace, Offer};

/// Create the marketplace configuration.
#[derive(Accounts)]
pub struct InitializeMarketplace<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Marketplace::INIT_SPACE,
        seeds = [Marketplace::SEED],
        bump,
    )]
    pub marketplace: Account<'info, Marketplace>,

    /// CHECK: fee destination, only stored.
    pub fee_recipient: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_marketplace(
    ctx: Context<InitializeMarketplace>,
    fee_bps: u16,
) -> Result<()> {
    require!(fee_bps <= Marketplace::MAX_FEE_BPS, MarketplaceError::FeeTooHigh);

    let marketplace = &mut ctx.accounts.marketplace;
    marketplace.authority = ctx.accounts.authority.key();
    marketplace.fee_recipient = ctx.accounts.fee_recipient.key();
    marketplace.fee_bps = fee_bps;
    marketplace.total_volume = 0;
    marketplace.total_sales = 0;
    marketplace.bump = ctx.bumps.marketplace;

    Ok(())
}

/// List an NFT, escrowing it in a program-owned account.
#[derive(Accounts)]
pub struct ListNft<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        init,
        payer = seller,
        space = 8 + Listing::INIT_SPACE,
        seeds = [Listing::SEED, nft_mint.key().as_ref()],
        bump,
    )]
    pub listing: Account<'info, Listing>,

    /// Escrow owned by the listing PDA.
    #[account(
        init,
        payer = seller,
        seeds = [Listing::ESCROW_SEED, listing.key().as_ref()],
        bump,
        token::mint = nft_mint,
        token::authority = listing,
        token::token_program = token_program,
    )]
    pub escrow: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = nft_mint, token::authority = seller)]
    pub seller_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(mint::decimals = 0)]
    pub nft_mint: InterfaceAccount<'info, Mint>,

    /// Collection the NFT belongs to, so royalties can be resolved at sale time.
    #[account(
        seeds = [Collection::SEED, collection.authority.as_ref(), collection.name.as_bytes()],
        bump = collection.bump,
    )]
    pub collection: Account<'info, Collection>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_list_nft(ctx: Context<ListNft>, price: u64) -> Result<()> {
    require!(price > 0, MarketplaceError::ZeroPrice);

    // The seller must actually hold it. Without this a listing could be created for an NFT the
    // seller does not own, and the escrow transfer below would simply fail later.
    require!(
        ctx.accounts.seller_token_account.amount == 1,
        MarketplaceError::NotTokenOwner
    );

    let listing = &mut ctx.accounts.listing;
    listing.seller = ctx.accounts.seller.key();
    listing.mint = ctx.accounts.nft_mint.key();
    listing.escrow = ctx.accounts.escrow.key();
    listing.collection = ctx.accounts.collection.key();
    listing.price = price;
    listing.created_ts = Clock::get()?.unix_timestamp;
    listing.bump = ctx.bumps.listing;

    // Escrow it. A signature-based listing that leaves the seller in custody lets them move the
    // asset mid-listing, and every purchase then fails after the buyer has already paid gas.
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.seller_token_account.to_account_info(),
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.escrow.to_account_info(),
                authority: ctx.accounts.seller.to_account_info(),
            },
        ),
        1,
        0,
    )?;

    Ok(())
}

/// Buy a listed NFT.
///
/// The price splits three ways: protocol fee, creator royalty, and the rest to the seller.
///
/// Solana has no protocol-level royalty enforcement. The royalty here is honoured because this
/// marketplace chooses to read the collection account and pay it, and a different marketplace can
/// simply not. That is a real property of the chain rather than a gap in this program, and pretending
/// otherwise is how creators end up surprised.
#[derive(Accounts)]
pub struct BuyNft<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        mut,
        seeds = [Marketplace::SEED],
        bump = marketplace.bump,
    )]
    pub marketplace: Account<'info, Marketplace>,

    #[account(
        mut,
        seeds = [Listing::SEED, listing.mint.as_ref()],
        bump = listing.bump,
        has_one = escrow @ MarketplaceError::Unauthorized,
        has_one = collection @ MarketplaceError::Unauthorized,
        // Rent returns to the seller, who paid it when listing.
        close = seller,
    )]
    pub listing: Account<'info, Listing>,

    #[account(mut)]
    pub escrow: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = nft_mint,
        token::authority = buyer,
    )]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = listing.mint @ MarketplaceError::Unauthorized)]
    pub nft_mint: InterfaceAccount<'info, Mint>,

    /// CHECK: paid out and closed to; checked against the listing's stored seller.
    #[account(mut, address = listing.seller @ MarketplaceError::Unauthorized)]
    pub seller: UncheckedAccount<'info>,

    #[account(
        seeds = [Collection::SEED, collection.authority.as_ref(), collection.name.as_bytes()],
        bump = collection.bump,
    )]
    pub collection: Account<'info, Collection>,

    /// CHECK: checked against the collection's stored royalty recipient.
    #[account(mut, address = collection.royalty_recipient @ MarketplaceError::Unauthorized)]
    pub royalty_recipient: UncheckedAccount<'info>,

    /// CHECK: checked against the marketplace's stored fee recipient.
    #[account(mut, address = marketplace.fee_recipient @ MarketplaceError::Unauthorized)]
    pub fee_recipient: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_buy_nft(ctx: Context<BuyNft>) -> Result<()> {
    let price = ctx.accounts.listing.price;

    // A seller buying their own listing would move lamports in a circle and inflate volume for no
    // reason. Cheap to reject.
    require_keys_neq!(
        ctx.accounts.buyer.key(),
        ctx.accounts.listing.seller,
        MarketplaceError::SelfTrade
    );

    let fee = (price as u128)
        .checked_mul(ctx.accounts.marketplace.fee_bps as u128)
        .ok_or(MarketplaceError::MathOverflow)?
        / 10_000;

    let royalty = (price as u128)
        .checked_mul(ctx.accounts.collection.royalty_bps as u128)
        .ok_or(MarketplaceError::MathOverflow)?
        / 10_000;

    // Both are capped at their maxima, so this cannot underflow.
    let proceeds = price
        .checked_sub(fee as u64)
        .and_then(|value| value.checked_sub(royalty as u64))
        .ok_or(MarketplaceError::MathOverflow)?;

    if fee > 0 {
        transfer_lamports(
            &ctx.accounts.buyer,
            &ctx.accounts.fee_recipient,
            &ctx.accounts.system_program,
            fee as u64,
        )?;
    }

    if royalty > 0 {
        transfer_lamports(
            &ctx.accounts.buyer,
            &ctx.accounts.royalty_recipient,
            &ctx.accounts.system_program,
            royalty as u64,
        )?;
    }

    transfer_lamports(
        &ctx.accounts.buyer,
        &ctx.accounts.seller,
        &ctx.accounts.system_program,
        proceeds,
    )?;

    // Release the NFT from escrow.
    let mint_key = ctx.accounts.listing.mint;
    let signer_seeds: &[&[&[u8]]] = &[&[
        Listing::SEED,
        mint_key.as_ref(),
        &[ctx.accounts.listing.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.escrow.to_account_info(),
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.buyer_token_account.to_account_info(),
                authority: ctx.accounts.listing.to_account_info(),
            },
            signer_seeds,
        ),
        1,
        0,
    )?;

    // Close the now-empty escrow so its rent returns to the seller too.
    token_interface::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        token_interface::CloseAccount {
            account: ctx.accounts.escrow.to_account_info(),
            destination: ctx.accounts.seller.to_account_info(),
            authority: ctx.accounts.listing.to_account_info(),
        },
        signer_seeds,
    ))?;

    let marketplace = &mut ctx.accounts.marketplace;
    marketplace.total_volume = marketplace.total_volume.saturating_add(price);
    marketplace.total_sales = marketplace.total_sales.saturating_add(1);

    msg!(
        "sold for {} lamports: {} fee, {} royalty, {} to seller",
        price,
        fee,
        royalty,
        proceeds
    );

    Ok(())
}

/// Cancel a listing and take the NFT back.
#[derive(Accounts)]
pub struct DelistNft<'info> {
    #[account(mut, address = listing.seller @ MarketplaceError::Unauthorized)]
    pub seller: Signer<'info>,

    #[account(
        mut,
        seeds = [Listing::SEED, listing.mint.as_ref()],
        bump = listing.bump,
        has_one = escrow @ MarketplaceError::Unauthorized,
        close = seller,
    )]
    pub listing: Account<'info, Listing>,

    #[account(mut)]
    pub escrow: InterfaceAccount<'info, TokenAccount>,

    #[account(mut, token::mint = nft_mint, token::authority = seller)]
    pub seller_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = listing.mint @ MarketplaceError::Unauthorized)]
    pub nft_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn handle_delist_nft(ctx: Context<DelistNft>) -> Result<()> {
    let mint_key = ctx.accounts.listing.mint;
    let signer_seeds: &[&[&[u8]]] = &[&[
        Listing::SEED,
        mint_key.as_ref(),
        &[ctx.accounts.listing.bump],
    ]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.escrow.to_account_info(),
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.seller_token_account.to_account_info(),
                authority: ctx.accounts.listing.to_account_info(),
            },
            signer_seeds,
        ),
        1,
        0,
    )?;

    token_interface::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        token_interface::CloseAccount {
            account: ctx.accounts.escrow.to_account_info(),
            destination: ctx.accounts.seller.to_account_info(),
            authority: ctx.accounts.listing.to_account_info(),
        },
        signer_seeds,
    ))?;

    Ok(())
}

/// Make an offer, escrowing the lamports in the offer PDA.
#[derive(Accounts)]
pub struct MakeOffer<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        init,
        payer = buyer,
        space = 8 + Offer::INIT_SPACE,
        seeds = [Offer::SEED, nft_mint.key().as_ref(), buyer.key().as_ref()],
        bump,
    )]
    pub offer: Account<'info, Offer>,

    #[account(mint::decimals = 0)]
    pub nft_mint: InterfaceAccount<'info, Mint>,

    pub system_program: Program<'info, System>,
}

pub fn handle_make_offer(ctx: Context<MakeOffer>, amount: u64, expires_ts: i64) -> Result<()> {
    require!(amount > 0, MarketplaceError::ZeroPrice);

    let now = Clock::get()?.unix_timestamp;
    require!(expires_ts > now, MarketplaceError::OfferExpired);

    let offer = &mut ctx.accounts.offer;
    offer.buyer = ctx.accounts.buyer.key();
    offer.mint = ctx.accounts.nft_mint.key();
    offer.amount = amount;
    offer.expires_ts = expires_ts;
    offer.bump = ctx.bumps.offer;

    // Escrow the lamports into the offer account itself. An offer whose funds are merely promised
    // can evaporate the moment a seller accepts, wasting their fee and making every offer suspect.
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.key(),
            system_program::Transfer {
                from: ctx.accounts.buyer.to_account_info(),
                to: ctx.accounts.offer.to_account_info(),
            },
        ),
        amount,
    )?;

    Ok(())
}

/// Withdraw an offer and reclaim the escrowed lamports.
#[derive(Accounts)]
pub struct CancelOffer<'info> {
    #[account(mut, address = offer.buyer @ MarketplaceError::Unauthorized)]
    pub buyer: Signer<'info>,

    #[account(
        mut,
        seeds = [Offer::SEED, offer.mint.as_ref(), offer.buyer.as_ref()],
        bump = offer.bump,
        // Closing returns both the escrowed lamports and the rent in one step, because an account's
        // whole lamport balance goes to the close destination.
        close = buyer,
    )]
    pub offer: Account<'info, Offer>,
}

pub fn handle_cancel_offer(_ctx: Context<CancelOffer>) -> Result<()> {
    Ok(())
}

/// Accept an offer: the NFT goes to the buyer, the escrowed lamports to the seller.
#[derive(Accounts)]
pub struct AcceptOffer<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        mut,
        seeds = [Marketplace::SEED],
        bump = marketplace.bump,
    )]
    pub marketplace: Account<'info, Marketplace>,

    #[account(
        mut,
        seeds = [Offer::SEED, offer.mint.as_ref(), offer.buyer.as_ref()],
        bump = offer.bump,
        close = seller,
    )]
    pub offer: Account<'info, Offer>,

    #[account(mut, token::mint = nft_mint, token::authority = seller)]
    pub seller_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = nft_mint,
        token::authority = offer.buyer,
    )]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(address = offer.mint @ MarketplaceError::Unauthorized)]
    pub nft_mint: InterfaceAccount<'info, Mint>,

    #[account(
        seeds = [Collection::SEED, collection.authority.as_ref(), collection.name.as_bytes()],
        bump = collection.bump,
    )]
    pub collection: Account<'info, Collection>,

    /// CHECK: checked against the collection's stored royalty recipient.
    #[account(mut, address = collection.royalty_recipient @ MarketplaceError::Unauthorized)]
    pub royalty_recipient: UncheckedAccount<'info>,

    /// CHECK: checked against the marketplace's stored fee recipient.
    #[account(mut, address = marketplace.fee_recipient @ MarketplaceError::Unauthorized)]
    pub fee_recipient: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

pub fn handle_accept_offer(ctx: Context<AcceptOffer>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(now < ctx.accounts.offer.expires_ts, MarketplaceError::OfferExpired);

    require_keys_neq!(
        ctx.accounts.seller.key(),
        ctx.accounts.offer.buyer,
        MarketplaceError::SelfTrade
    );

    require!(
        ctx.accounts.seller_token_account.amount == 1,
        MarketplaceError::NotTokenOwner
    );

    let price = ctx.accounts.offer.amount;

    let fee = (price as u128 * ctx.accounts.marketplace.fee_bps as u128) / 10_000;
    let royalty = (price as u128 * ctx.accounts.collection.royalty_bps as u128) / 10_000;
    let proceeds = price
        .checked_sub(fee as u64)
        .and_then(|value| value.checked_sub(royalty as u64))
        .ok_or(MarketplaceError::MathOverflow)?;

    // The lamports are already inside the offer account, so paying out means moving them directly
    // rather than through the System program: a PDA that holds data cannot be a System transfer
    // source, so its lamports are adjusted in place.
    let offer_info = ctx.accounts.offer.to_account_info();

    if fee > 0 {
        move_lamports(&offer_info, &ctx.accounts.fee_recipient.to_account_info(), fee as u64)?;
    }
    if royalty > 0 {
        move_lamports(
            &offer_info,
            &ctx.accounts.royalty_recipient.to_account_info(),
            royalty as u64,
        )?;
    }
    move_lamports(&offer_info, &ctx.accounts.seller.to_account_info(), proceeds)?;

    // Transfer the NFT. The seller signs directly; no escrow is involved on this path.
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.seller_token_account.to_account_info(),
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.buyer_token_account.to_account_info(),
                authority: ctx.accounts.seller.to_account_info(),
            },
        ),
        1,
        0,
    )?;

    let marketplace = &mut ctx.accounts.marketplace;
    marketplace.total_volume = marketplace.total_volume.saturating_add(price);
    marketplace.total_sales = marketplace.total_sales.saturating_add(1);

    Ok(())
}

/// Move lamports out of a program-owned account.
///
/// A PDA holding data cannot be the source of a System program transfer, because the System program
/// refuses to move lamports out of an account it does not own. Adjusting both balances directly is
/// the supported way, and it only works because this program owns the source account.
fn move_lamports(from: &AccountInfo<'_>, to: &AccountInfo<'_>, amount: u64) -> Result<()> {
    let mut from_lamports = from.try_borrow_mut_lamports()?;
    let mut to_lamports = to.try_borrow_mut_lamports()?;

    **from_lamports = from_lamports
        .checked_sub(amount)
        .ok_or(MarketplaceError::InsufficientEscrow)?;
    **to_lamports = to_lamports
        .checked_add(amount)
        .ok_or(MarketplaceError::MathOverflow)?;

    Ok(())
}

/// System transfer from a signer.
fn transfer_lamports<'info>(
    from: &Signer<'info>,
    to: &UncheckedAccount<'info>,
    system_program: &Program<'info, System>,
    amount: u64,
) -> Result<()> {
    system_program::transfer(
        CpiContext::new(
            system_program.key(),
            system_program::Transfer {
                from: from.to_account_info(),
                to: to.to_account_info(),
            },
        ),
        amount,
    )
}
