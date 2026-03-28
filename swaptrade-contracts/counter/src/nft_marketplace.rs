#![cfg_attr(not(test), no_std)]
use soroban_sdk::{Address, Env, Symbol, symbol_short};
use crate::nft_types::*;
use crate::nft_errors::NFTError;
use crate::nft_storage::*;
use crate::nft_minting::{get_nft, get_collection, is_owner, update_portfolio_on_sale};
use crate::emergency;

/// Default listing duration (7 days in seconds)
const DEFAULT_LISTING_DURATION: u64 = 7 * 24 * 60 * 60;
/// Default offer duration (3 days in seconds)
const DEFAULT_OFFER_DURATION: u64 = 3 * 24 * 60 * 60;
/// Maximum listing duration (30 days)
const MAX_LISTING_DURATION: u64 = 30 * 24 * 60 * 60;
/// Maximum offer duration (7 days)
const MAX_OFFER_DURATION: u64 = 7 * 24 * 60 * 60;

/// Create a new NFT listing
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `seller` - The NFT owner listing the token
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `price` - Listing price
/// * `payment_token` - Payment token symbol (XLM for native)
/// * `amount` - Amount for ERC-1155 (1 for ERC-721)
/// * `duration` - Listing duration in seconds (0 for default)
/// * `is_auction` - Whether this is an auction
/// * `min_bid` - Minimum bid for auction
/// 
/// # Returns
/// * `Result<u64, NFTError>` - Listing ID on success
pub fn create_listing(
    env: &Env,
    seller: Address,
    collection_id: u64,
    token_id: u64,
    price: i128,
    payment_token: Symbol,
    amount: u64,
    duration: u64,
    is_auction: bool,
    min_bid: i128,
) -> Result<u64, NFTError> {
    seller.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, seller.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate price
    if price <= 0 {
        return Err(NFTError::InvalidPrice);
    }
    
    // Get NFT
    let nft = get_nft(env, collection_id, token_id).ok_or(NFTError::NFTNotFound)?;
    
    // Verify ownership
    if nft.owner != seller {
        return Err(NFTError::NotOwner);
    }
    
    // Check if NFT is collateralized
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    if loan_registry.get_loan_by_collateral(collection_id, token_id).is_some() {
        return Err(NFTError::AlreadyCollateralized);
    }
    
    // Validate amount based on NFT standard
    let final_amount = match nft.standard {
        NFTStandard::ERC721 => {
            if amount != 1 {
                return Err(NFTError::InvalidAmount);
            }
            1
        }
        NFTStandard::ERC1155 => {
            if amount == 0 || amount > nft.circulating_supply {
                return Err(NFTError::InvalidAmount);
            }
            amount
        }
    };
    
    // Validate auction params
    if is_auction && min_bid <= 0 {
        return Err(NFTError::InvalidBid);
    }
    
    // Set duration
    let final_duration = if duration == 0 {
        DEFAULT_LISTING_DURATION
    } else {
        duration.min(MAX_LISTING_DURATION)
    };
    
    let current_time = env.ledger().timestamp();
    
    // Get listing registry
    let mut listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .unwrap_or_else(|| ListingRegistry::new(env));
    
    // Generate listing ID
    let listing_id = get_next_listing_id(env);
    
    // Create listing
    let listing = NFTListing {
        listing_id,
        token_id,
        collection_id,
        seller: seller.clone(),
        price,
        payment_token,
        amount: final_amount,
        created_at: current_time,
        expires_at: current_time + final_duration,
        is_auction,
        min_bid: if is_auction { min_bid } else { 0 },
        highest_bid: 0,
        highest_bidder: None,
        is_active: true,
    };
    
    // Store listing
    listing_registry.create_listing(env, listing);
    env.storage().instance().set(&LISTING_REGISTRY_KEY, &listing_registry);
    
    // Update seller's portfolio
    update_portfolio_on_listing(env, seller.clone())?;
    
    // Emit event
    crate::nft_events::emit_listing_created(
        env, listing_id, collection_id, token_id, seller, price, is_auction
    );
    
    Ok(listing_id)
}

/// Cancel a listing
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `seller` - The listing creator
/// * `listing_id` - Listing ID to cancel
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn cancel_listing(
    env: &Env,
    seller: Address,
    listing_id: u64,
) -> Result<(), NFTError> {
    seller.require_auth();
    
    let mut listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .ok_or(NFTError::ListingNotFound)?;
    
    let listing = listing_registry.get_listing(listing_id)
        .ok_or(NFTError::ListingNotFound)?;
    
    // Verify seller
    if listing.seller != seller {
        return Err(NFTError::NotOwner);
    }
    
    // Deactivate listing
    listing_registry.deactivate_listing(env, listing_id)?;
    env.storage().instance().set(&LISTING_REGISTRY_KEY, &listing_registry);
    
    // Update seller's portfolio
    decrement_portfolio_listings(env, seller)?;
    
    // Emit event
    crate::nft_events::emit_listing_cancelled(env, listing_id);
    
    Ok(())
}

/// Place a bid on an auction listing
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `bidder` - Bidder address
/// * `listing_id` - Listing ID
/// * `bid_amount` - Bid amount
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn place_bid(
    env: &Env,
    bidder: Address,
    listing_id: u64,
    bid_amount: i128,
) -> Result<(), NFTError> {
    bidder.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, bidder.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    let mut listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .ok_or(NFTError::ListingNotFound)?;
    
    let mut listing = listing_registry.get_listing(listing_id)
        .ok_or(NFTError::ListingNotFound)?;
    
    // Check if it's an auction
    if !listing.is_auction {
        return Err(NFTError::UnsupportedOperation);
    }
    
    // Check if listing is valid
    let current_time = env.ledger().timestamp();
    if !listing.is_valid(current_time) {
        return Err(NFTError::ListingExpired);
    }
    
    // Prevent self-dealing
    if listing.seller == bidder {
        return Err(NFTError::SelfDealing);
    }
    
    // Validate bid
    if bid_amount < listing.min_bid {
        return Err(NFTError::InvalidBid);
    }
    
    if bid_amount <= listing.highest_bid {
        return Err(NFTError::InvalidBid);
    }
    
    // Update listing with new bid
    listing.highest_bid = bid_amount;
    listing.highest_bidder = Some(bidder.clone());
    listing_registry.update_listing(listing);
    env.storage().instance().set(&LISTING_REGISTRY_KEY, &listing_registry);
    
    // Emit event
    crate::nft_events::emit_bid_placed(env, listing_id, bidder, bid_amount);
    
    Ok(())
}

/// Create an offer/bid for an NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `buyer` - Offer creator
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `amount` - Offer amount
/// * `payment_token` - Payment token
/// * `offer_amount` - Quantity for ERC-1155
/// * `duration` - Offer duration
/// 
/// # Returns
/// * `Result<u64, NFTError>` - Offer ID on success
pub fn create_offer(
    env: &Env,
    buyer: Address,
    collection_id: u64,
    token_id: u64,
    amount: i128,
    payment_token: Symbol,
    offer_amount: u64,
    duration: u64,
) -> Result<u64, NFTError> {
    buyer.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, buyer.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate amount
    if amount <= 0 {
        return Err(NFTError::InvalidPrice);
    }
    
    // Get NFT
    let nft = get_nft(env, collection_id, token_id).ok_or(NFTError::NFTNotFound)?;
    
    // Prevent self-dealing
    if nft.owner == buyer {
        return Err(NFTError::SelfDealing);
    }
    
    // Validate offer amount
    let final_amount = match nft.standard {
        NFTStandard::ERC721 => {
            if offer_amount != 1 && offer_amount != 0 {
                return Err(NFTError::InvalidAmount);
            }
            1
        }
        NFTStandard::ERC1155 => {
            if offer_amount == 0 {
                return Err(NFTError::InvalidAmount);
            }
            offer_amount
        }
    };
    
    // Set duration
    let final_duration = if duration == 0 {
        DEFAULT_OFFER_DURATION
    } else {
        duration.min(MAX_OFFER_DURATION)
    };
    
    let current_time = env.ledger().timestamp();
    
    // Get offer registry
    let mut offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .unwrap_or_else(|| OfferRegistry::new(env));
    
    // Generate offer ID
    let offer_id = get_next_offer_id(env);
    
    // Create offer
    let offer = NFTOffer {
        offer_id,
        token_id,
        collection_id,
        buyer: buyer.clone(),
        amount,
        payment_token,
        quantity: final_amount,
        created_at: current_time,
        expires_at: current_time + final_duration,
        is_active: true,
    };
    
    // Store offer
    offer_registry.create_offer(env, offer);
    env.storage().instance().set(&OFFER_REGISTRY_KEY, &offer_registry);
    
    // Update buyer's portfolio
    update_portfolio_on_offer(env, buyer.clone())?;
    
    // Emit event
    crate::nft_events::emit_offer_created(
        env, offer_id, collection_id, token_id, buyer, amount
    );
    
    Ok(offer_id)
}

/// Cancel an offer
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `buyer` - Offer creator
/// * `offer_id` - Offer ID
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn cancel_offer(
    env: &Env,
    buyer: Address,
    offer_id: u64,
) -> Result<(), NFTError> {
    buyer.require_auth();
    
    let mut offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .ok_or(NFTError::OfferNotFound)?;
    
    let offer = offer_registry.get_offer(offer_id)
        .ok_or(NFTError::OfferNotFound)?;
    
    // Verify buyer
    if offer.buyer != buyer {
        return Err(NFTError::Unauthorized);
    }
    
    // Deactivate offer
    offer_registry.deactivate_offer(env, offer_id)?;
    env.storage().instance().set(&OFFER_REGISTRY_KEY, &offer_registry);
    
    // Update buyer's portfolio
    decrement_portfolio_offers(env, buyer)?;
    
    // Emit event
    crate::nft_events::emit_offer_cancelled(env, offer_id);
    
    Ok(())
}

/// Accept an offer (called by NFT owner)
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `seller` - NFT owner
/// * `offer_id` - Offer ID to accept
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn accept_offer(
    env: &Env,
    seller: Address,
    offer_id: u64,
) -> Result<(), NFTError> {
    seller.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    let mut offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .ok_or(NFTError::OfferNotFound)?;
    
    let offer = offer_registry.get_offer(offer_id)
        .ok_or(NFTError::OfferNotFound)?;
    
    // Check if offer is valid
    let current_time = env.ledger().timestamp();
    if !offer.is_valid(current_time) {
        return Err(NFTError::OfferExpired);
    }
    
    // Verify seller owns the NFT
    if !is_owner(env, offer.collection_id, offer.token_id, seller.clone()) {
        return Err(NFTError::NotOwner);
    }
    
    // Execute the trade
    execute_trade(
        env,
        offer.collection_id,
        offer.token_id,
        seller.clone(),
        offer.buyer.clone(),
        offer.amount,
        offer.payment_token,
        offer.quantity,
    )?;
    
    // Deactivate offer
    offer_registry.deactivate_offer(env, offer_id)?;
    env.storage().instance().set(&OFFER_REGISTRY_KEY, &offer_registry);
    
    // Update portfolios
    decrement_portfolio_offers(env, offer.buyer.clone())?;
    
    // Emit event
    crate::nft_events::emit_offer_accepted(env, offer_id, seller);
    
    Ok(())
}

/// Buy NFT from a fixed-price listing
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `buyer` - Buyer address
/// * `listing_id` - Listing ID
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn buy_nft(
    env: &Env,
    buyer: Address,
    listing_id: u64,
) -> Result<(), NFTError> {
    buyer.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, buyer.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    let mut listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .ok_or(NFTError::ListingNotFound)?;
    
    let listing = listing_registry.get_listing(listing_id)
        .ok_or(NFTError::ListingNotFound)?;
    
    // Check if it's a fixed-price listing (not auction)
    if listing.is_auction {
        return Err(NFTError::UnsupportedOperation);
    }
    
    // Check if listing is valid
    let current_time = env.ledger().timestamp();
    if !listing.is_valid(current_time) {
        return Err(NFTError::ListingExpired);
    }
    
    // Prevent self-dealing
    if listing.seller == buyer {
        return Err(NFTError::SelfDealing);
    }
    
    // Execute the trade
    execute_trade(
        env,
        listing.collection_id,
        listing.token_id,
        listing.seller.clone(),
        buyer.clone(),
        listing.price,
        listing.payment_token,
        listing.amount,
    )?;
    
    // Deactivate listing
    listing_registry.deactivate_listing(env, listing_id)?;
    env.storage().instance().set(&LISTING_REGISTRY_KEY, &listing_registry);
    
    // Update seller's portfolio
    decrement_portfolio_listings(env, listing.seller.clone())?;
    
    // Emit event
    crate::nft_events::emit_nft_sold(env, listing_id, buyer, listing.price);
    
    Ok(())
}

/// Finalize an auction (can be called by anyone after expiry)
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `listing_id` - Auction listing ID
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn finalize_auction(
    env: &Env,
    listing_id: u64,
) -> Result<(), NFTError> {
    let mut listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .ok_or(NFTError::ListingNotFound)?;
    
    let listing = listing_registry.get_listing(listing_id)
        .ok_or(NFTError::ListingNotFound)?;
    
    // Check if it's an auction
    if !listing.is_auction {
        return Err(NFTError::UnsupportedOperation);
    }
    
    // Check if auction has ended
    let current_time = env.ledger().timestamp();
    if current_time <= listing.expires_at {
        return Err(NFTError::AuctionActive);
    }
    
    // Check if there were any bids
    let highest_bidder = listing.highest_bidder.clone()
        .ok_or(NFTError::InvalidBid)?;
    
    // Execute the trade
    execute_trade(
        env,
        listing.collection_id,
        listing.token_id,
        listing.seller.clone(),
        highest_bidder.clone(),
        listing.highest_bid,
        listing.payment_token,
        listing.amount,
    )?;
    
    // Deactivate listing
    listing_registry.deactivate_listing(env, listing_id)?;
    env.storage().instance().set(&LISTING_REGISTRY_KEY, &listing_registry);
    
    // Update seller's portfolio
    decrement_portfolio_listings(env, listing.seller.clone())?;
    
    // Emit event
    crate::nft_events::emit_auction_finalized(
        env, listing_id, highest_bidder, listing.highest_bid
    );
    
    Ok(())
}

/// Execute a trade between seller and buyer
fn execute_trade(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    seller: Address,
    buyer: Address,
    price: i128,
    payment_token: Symbol,
    amount: u64,
) -> Result<(), NFTError> {
    // Get NFT
    let mut nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .ok_or(NFTError::NFTNotFound)?;
    
    let mut nft = nft_registry.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    
    // Verify ownership
    if nft.owner != seller {
        return Err(NFTError::NotOwner);
    }
    
    // Get collection for royalty info
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .ok_or(NFTError::CollectionNotFound)?;
    
    let collection = collection_registry.get_collection(collection_id)
        .ok_or(NFTError::CollectionNotFound)?;
    
    // Calculate fees
    let platform_fee_bps = get_platform_fee_bps(env);
    let platform_fee = (price * platform_fee_bps as i128) / 10000;
    let royalty_amount = collection.calculate_royalty(price);
    let seller_proceeds = price - platform_fee - royalty_amount;
    
    // Transfer ownership
    nft_registry.transfer_ownership(env, collection_id, token_id, buyer.clone())?;
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    
    // Update collection stats
    let mut collection_registry_mut: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    let mut collection_mut = collection_registry_mut.get_collection(collection_id)
        .ok_or(NFTError::CollectionNotFound)?;
    collection_mut.add_volume(price);
    collection_mut.update_floor_price(price);
    collection_registry_mut.update_collection(collection_mut);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry_mut);
    
    // Record trade
    let mut trade_history: TradeHistory = env.storage().instance()
        .get(&TRADE_HISTORY_KEY)
        .unwrap_or_else(|| TradeHistory::new(env));
    
    let trade = NFTTrade {
        token_id,
        collection_id,
        seller: seller.clone(),
        buyer: buyer.clone(),
        price,
        payment_token,
        quantity: amount,
        timestamp: env.ledger().timestamp(),
        royalty_amount,
        platform_fee,
    };
    trade_history.record_trade(env, trade);
    env.storage().instance().set(&TRADE_HISTORY_KEY, &trade_history);
    
    // Update portfolios
    update_portfolio_on_sale(env, seller.clone(), collection_id, token_id)?;
    update_portfolio_on_purchase(env, buyer.clone(), collection_id, token_id, price)?;
    
    // Emit trade event
    crate::nft_events::emit_nft_traded(
        env, collection_id, token_id, seller, buyer, price, royalty_amount, platform_fee
    );
    
    Ok(())
}

/// Update portfolio when creating a listing
fn update_portfolio_on_listing(env: &Env, seller: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(seller.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, seller.clone()));
    
    portfolio.active_listings = portfolio.active_listings.saturating_add(1);
    
    portfolio_registry.set(seller.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Decrement portfolio listing count
fn decrement_portfolio_listings(env: &Env, seller: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(seller.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, seller.clone()));
    
    portfolio.active_listings = portfolio.active_listings.saturating_sub(1);
    
    portfolio_registry.set(seller.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Update portfolio when creating an offer
fn update_portfolio_on_offer(env: &Env, buyer: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(buyer.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, buyer.clone()));
    
    portfolio.active_offers = portfolio.active_offers.saturating_add(1);
    
    portfolio_registry.set(buyer.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Decrement portfolio offer count
fn decrement_portfolio_offers(env: &Env, buyer: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(buyer.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, buyer.clone()));
    
    portfolio.active_offers = portfolio.active_offers.saturating_sub(1);
    
    portfolio_registry.set(buyer.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Update portfolio on purchase
fn update_portfolio_on_purchase(
    env: &Env,
    buyer: Address,
    collection_id: u64,
    token_id: u64,
    price: i128,
) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(buyer.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, buyer.clone()));
    
    portfolio.add_nft(token_id, collection_id);
    portfolio.record_trade(price);
    
    portfolio_registry.set(buyer.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Get listing by ID
pub fn get_listing(env: &Env, listing_id: u64) -> Option<NFTListing> {
    let listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .unwrap_or_else(|| ListingRegistry::new(env));
    listing_registry.get_listing(listing_id)
}

/// Get offer by ID
pub fn get_offer(env: &Env, offer_id: u64) -> Option<NFTOffer> {
    let offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .unwrap_or_else(|| OfferRegistry::new(env));
    offer_registry.get_offer(offer_id)
}

/// Get active listings for a token
pub fn get_token_listings(env: &Env, collection_id: u64, token_id: u64) -> Vec<u64> {
    let listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .unwrap_or_else(|| ListingRegistry::new(env));
    listing_registry.get_token_listings(collection_id, token_id)
}

/// Get active offers for a token
pub fn get_token_offers(env: &Env, collection_id: u64, token_id: u64) -> Vec<u64> {
    let offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .unwrap_or_else(|| OfferRegistry::new(env));
    offer_registry.get_token_offers(collection_id, token_id)
}

/// Get total active listings
pub fn get_total_listings(env: &Env) -> u64 {
    let listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .unwrap_or_else(|| ListingRegistry::new(env));
    listing_registry.active_count
}

/// Get total active offers
pub fn get_total_offers(env: &Env) -> u64 {
    let offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .unwrap_or_else(|| OfferRegistry::new(env));
    offer_registry.active_count
}
