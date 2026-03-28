#![cfg_attr(not(test), no_std)]
use soroban_sdk::{Address, Env, Symbol, String};
use crate::nft_types::{NFTStandard, ValuationMethod};

/// Emit event when a new collection is created
pub fn emit_collection_created(
    env: &Env,
    collection_id: u64,
    owner: Address,
) {
    env.events().publish(
        (Symbol::new(env, "CollectionCreated"), collection_id),
        (owner,),
    );
}

/// Emit event when collection ownership is transferred
pub fn emit_collection_ownership_transferred(
    env: &Env,
    collection_id: u64,
    previous_owner: Address,
    new_owner: Address,
) {
    env.events().publish(
        (Symbol::new(env, "CollectionOwnershipTransferred"), collection_id),
        (previous_owner, new_owner),
    );
}

/// Emit event when minting status changes
pub fn emit_minting_status_changed(
    env: &Env,
    collection_id: u64,
    active: bool,
) {
    env.events().publish(
        (Symbol::new(env, "MintingStatusChanged"), collection_id),
        (active,),
    );
}

/// Emit event when an NFT is minted
pub fn emit_nft_minted(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    creator: Address,
    amount: u64,
) {
    env.events().publish(
        (Symbol::new(env, "NFTMinted"), collection_id, token_id),
        (creator, amount),
    );
}

/// Emit event when an NFT is transferred
pub fn emit_nft_transferred(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    from: Address,
    to: Address,
    amount: u64,
) {
    env.events().publish(
        (Symbol::new(env, "NFTTransferred"), collection_id, token_id),
        (from, to, amount),
    );
}

/// Emit event when an NFT is burned
pub fn emit_nft_burned(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    owner: Address,
    amount: u64,
) {
    env.events().publish(
        (Symbol::new(env, "NFTBurned"), collection_id, token_id),
        (owner, amount),
    );
}

/// Emit event when a listing is created
pub fn emit_listing_created(
    env: &Env,
    listing_id: u64,
    collection_id: u64,
    token_id: u64,
    seller: Address,
    price: i128,
    is_auction: bool,
) {
    env.events().publish(
        (Symbol::new(env, "ListingCreated"), listing_id),
        (collection_id, token_id, seller, price, is_auction),
    );
}

/// Emit event when a listing is cancelled
pub fn emit_listing_cancelled(
    env: &Env,
    listing_id: u64,
) {
    env.events().publish(
        (Symbol::new(env, "ListingCancelled"), listing_id),
        (),
    );
}

/// Emit event when a listing is updated
pub fn emit_listing_updated(
    env: &Env,
    listing_id: u64,
    new_price: i128,
) {
    env.events().publish(
        (Symbol::new(env, "ListingUpdated"), listing_id),
        (new_price,),
    );
}

/// Emit event when a bid is placed on an auction
pub fn emit_bid_placed(
    env: &Env,
    listing_id: u64,
    bidder: Address,
    bid_amount: i128,
) {
    env.events().publish(
        (Symbol::new(env, "BidPlaced"), listing_id),
        (bidder, bid_amount),
    );
}

/// Emit event when an auction is finalized
pub fn emit_auction_finalized(
    env: &Env,
    listing_id: u64,
    winner: Address,
    winning_bid: i128,
) {
    env.events().publish(
        (Symbol::new(env, "AuctionFinalized"), listing_id),
        (winner, winning_bid),
    );
}

/// Emit event when an offer is created
pub fn emit_offer_created(
    env: &Env,
    offer_id: u64,
    collection_id: u64,
    token_id: u64,
    buyer: Address,
    amount: i128,
) {
    env.events().publish(
        (Symbol::new(env, "OfferCreated"), offer_id),
        (collection_id, token_id, buyer, amount),
    );
}

/// Emit event when an offer is cancelled
pub fn emit_offer_cancelled(
    env: &Env,
    offer_id: u64,
) {
    env.events().publish(
        (Symbol::new(env, "OfferCancelled"), offer_id),
        (),
    );
}

/// Emit event when an offer is accepted
pub fn emit_offer_accepted(
    env: &Env,
    offer_id: u64,
    seller: Address,
) {
    env.events().publish(
        (Symbol::new(env, "OfferAccepted"), offer_id),
        (seller,),
    );
}

/// Emit event when an NFT is sold
pub fn emit_nft_sold(
    env: &Env,
    listing_id: u64,
    buyer: Address,
    price: i128,
) {
    env.events().publish(
        (Symbol::new(env, "NFTSold"), listing_id),
        (buyer, price),
    );
}

/// Emit event when an NFT trade occurs
pub fn emit_nft_traded(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    seller: Address,
    buyer: Address,
    price: i128,
    royalty_amount: i128,
    platform_fee: i128,
) {
    env.events().publish(
        (Symbol::new(env, "NFTTraded"), collection_id, token_id),
        (seller, buyer, price, royalty_amount, platform_fee),
    );
}

/// Emit event when an NFT is fractionalized
pub fn emit_nft_fractionalized(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    owner: Address,
    total_shares: u64,
    initial_price: i128,
) {
    env.events().publish(
        (Symbol::new(env, "NFTFractionalized"), collection_id, token_id),
        (owner, total_shares, initial_price),
    );
}

/// Emit event when an NFT is defractionalized
pub fn emit_nft_defractionalized(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    owner: Address,
) {
    env.events().publish(
        (Symbol::new(env, "NFTDefractionalized"), collection_id, token_id),
        (owner,),
    );
}

/// Emit event when fractional shares are purchased
pub fn emit_fractional_shares_purchased(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    buyer: Address,
    shares: u64,
    price_per_share: i128,
) {
    env.events().publish(
        (Symbol::new(env, "FractionalSharesPurchased"), collection_id, token_id),
        (buyer, shares, price_per_share),
    );
}

/// Emit event when fractional shares are sold
pub fn emit_fractional_shares_sold(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    seller: Address,
    shares: u64,
    price_per_share: i128,
) {
    env.events().publish(
        (Symbol::new(env, "FractionalSharesSold"), collection_id, token_id),
        (seller, shares, price_per_share),
    );
}

/// Emit event when fractional shares are transferred
pub fn emit_fractional_shares_transferred(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    from: Address,
    to: Address,
    shares: u64,
) {
    env.events().publish(
        (Symbol::new(env, "FractionalSharesTransferred"), collection_id, token_id),
        (from, to, shares),
    );
}

/// Emit event when a loan is requested
pub fn emit_loan_requested(
    env: &Env,
    loan_id: u64,
    collection_id: u64,
    token_id: u64,
    borrower: Address,
    loan_amount: i128,
) {
    env.events().publish(
        (Symbol::new(env, "LoanRequested"), loan_id),
        (collection_id, token_id, borrower, loan_amount),
    );
}

/// Emit event when a loan is funded
pub fn emit_loan_funded(
    env: &Env,
    loan_id: u64,
    lender: Address,
    amount: i128,
) {
    env.events().publish(
        (Symbol::new(env, "LoanFunded"), loan_id),
        (lender, amount),
    );
}

/// Emit event when a loan is repaid
pub fn emit_loan_repaid(
    env: &Env,
    loan_id: u64,
    borrower: Address,
    repayment_amount: i128,
) {
    env.events().publish(
        (Symbol::new(env, "LoanRepaid"), loan_id),
        (borrower, repayment_amount),
    );
}

/// Emit event when a loan is liquidated
pub fn emit_loan_liquidated(
    env: &Env,
    loan_id: u64,
    lender: Address,
    collection_id: u64,
    token_id: u64,
) {
    env.events().publish(
        (Symbol::new(env, "LoanLiquidated"), loan_id),
        (lender, collection_id, token_id),
    );
}

/// Emit event when a loan is cancelled
pub fn emit_loan_cancelled(
    env: &Env,
    loan_id: u64,
    borrower: Address,
) {
    env.events().publish(
        (Symbol::new(env, "LoanCancelled"), loan_id),
        (borrower,),
    );
}

/// Emit event when royalty is paid
pub fn emit_royalty_paid(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    recipient: Address,
    amount: i128,
) {
    env.events().publish(
        (Symbol::new(env, "RoyaltyPaid"), collection_id, token_id),
        (recipient, amount),
    );
}

/// Emit event when platform fee is collected
pub fn emit_platform_fee_collected(
    env: &Env,
    amount: i128,
    recipient: Address,
) {
    env.events().publish(
        (Symbol::new(env, "PlatformFeeCollected"),),
        (amount, recipient),
    );
}

/// Emit event when NFT valuation is updated
pub fn emit_valuation_updated(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    estimated_value: i128,
    method: ValuationMethod,
) {
    env.events().publish(
        (Symbol::new(env, "ValuationUpdated"), collection_id, token_id),
        (estimated_value, method),
    );
}

/// Emit event when collection floor price is updated
pub fn emit_floor_price_updated(
    env: &Env,
    collection_id: u64,
    new_floor_price: i128,
) {
    env.events().publish(
        (Symbol::new(env, "FloorPriceUpdated"), collection_id),
        (new_floor_price,),
    );
}

/// Emit event when marketplace is paused/unpaused
pub fn emit_marketplace_paused(
    env: &Env,
    paused: bool,
    admin: Address,
) {
    env.events().publish(
        (Symbol::new(env, "MarketplacePaused"),),
        (paused, admin),
    );
}

/// Emit event when platform fee is updated
pub fn emit_platform_fee_updated(
    env: &Env,
    old_fee_bps: u32,
    new_fee_bps: u32,
) {
    env.events().publish(
        (Symbol::new(env, "PlatformFeeUpdated"),),
        (old_fee_bps, new_fee_bps),
    );
}

/// Emit event when NFT metadata is updated
pub fn emit_metadata_updated(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    new_metadata_uri: String,
) {
    env.events().publish(
        (Symbol::new(env, "MetadataUpdated"), collection_id, token_id),
        (new_metadata_uri,),
    );
}

/// Emit event when NFT is wrapped for cross-chain
pub fn emit_nft_wrapped(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    original_chain: u32,
    wrapped_address: Address,
) {
    env.events().publish(
        (Symbol::new(env, "NFTWrapped"), collection_id, token_id),
        (original_chain, wrapped_address),
    );
}

/// Emit event when NFT is unwrapped from cross-chain
pub fn emit_nft_unwrapped(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    target_chain: u32,
    recipient: Address,
) {
    env.events().publish(
        (Symbol::new(env, "NFTUnwrapped"), collection_id, token_id),
        (target_chain, recipient),
    );
}

/// Emit event when a new badge is awarded for NFT activity
pub fn emit_nft_badge_awarded(
    env: &Env,
    user: Address,
    badge_type: Symbol,
) {
    env.events().publish(
        (Symbol::new(env, "NFTBadgeAwarded"), user),
        (badge_type, env.ledger().timestamp()),
    );
}

/// Emit event for batch operations
pub fn emit_batch_mint_completed(
    env: &Env,
    collection_id: u64,
    start_token_id: u64,
    end_token_id: u64,
    creator: Address,
) {
    env.events().publish(
        (Symbol::new(env, "BatchMintCompleted"), collection_id),
        (start_token_id, end_token_id, creator),
    );
}

/// Emit event for batch transfers
pub fn emit_batch_transfer_completed(
    env: &Env,
    collection_id: u64,
    from: Address,
    to: Address,
    token_count: u32,
) {
    env.events().publish(
        (Symbol::new(env, "BatchTransferCompleted"), collection_id),
        (from, to, token_count),
    );
}
