#![cfg_attr(not(test), no_std)]

// Re-export all NFT modules
pub mod nft_types;
pub mod nft_errors;
pub mod nft_storage;
pub mod nft_events;
pub mod nft_minting;
pub mod nft_marketplace;
pub mod nft_fractional;
pub mod nft_lending;

// Re-export commonly used types
pub use nft_types::*;
pub use nft_errors::NFTError;
pub use nft_storage::*;

use soroban_sdk::{Address, Env, Map, Symbol, Vec, String};
use crate::emergency;

/// Initialize the NFT marketplace
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `platform_fee_bps` - Platform fee in basis points (e.g., 250 = 2.5%)
/// * `fee_recipient` - Address to receive platform fees
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn initialize(
    env: &Env,
    platform_fee_bps: u32,
    fee_recipient: Address,
) -> Result<(), NFTError> {
    // Set platform fee
    set_platform_fee_bps(env, platform_fee_bps);
    
    // Set fee recipient
    set_fee_recipient(env, fee_recipient);
    
    // Initialize registries
    let nft_registry = NFTRegistry::new(env);
    let collection_registry = CollectionRegistry::new(env);
    let listing_registry = ListingRegistry::new(env);
    let offer_registry = OfferRegistry::new(env);
    let loan_registry = LoanRegistry::new(env);
    let trade_history = TradeHistory::new(env);
    let valuation_registry = ValuationRegistry::new(env);
    let fractional_registry = FractionalRegistry::new(env);
    
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry);
    env.storage().instance().set(&LISTING_REGISTRY_KEY, &listing_registry);
    env.storage().instance().set(&OFFER_REGISTRY_KEY, &offer_registry);
    env.storage().instance().set(&LOAN_REGISTRY_KEY, &loan_registry);
    env.storage().instance().set(&TRADE_HISTORY_KEY, &trade_history);
    env.storage().instance().set(&VALUATION_REGISTRY_KEY, &valuation_registry);
    env.storage().instance().set(&FRACTIONAL_SHARES_KEY, &fractional_registry);
    
    // Marketplace starts unpaused
    set_marketplace_paused(env, false);
    
    Ok(())
}

/// Get NFT portfolio for a user
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - User address
/// 
/// # Returns
/// * `NFTPortfolio` - User's NFT portfolio
pub fn get_nft_portfolio(env: &Env, owner: Address) -> NFTPortfolio {
    let portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    portfolio_registry.get(owner.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, owner))
}

/// Get total NFT marketplace volume
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// 
/// # Returns
/// * `i128` - Total trading volume
pub fn get_total_volume(env: &Env) -> i128 {
    let trade_history: TradeHistory = env.storage().instance()
        .get(&TRADE_HISTORY_KEY)
        .unwrap_or_else(|| TradeHistory::new(env));
    trade_history.total_volume
}

/// Get NFT trade history for a specific token
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `limit` - Maximum number of trades to return
/// 
/// # Returns
/// * `Vec<NFTTrade>` - Trade history for the token
pub fn get_token_trade_history(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    limit: u32,
) -> Vec<NFTTrade> {
    let trade_history: TradeHistory = env.storage().instance()
        .get(&TRADE_HISTORY_KEY)
        .unwrap_or_else(|| TradeHistory::new(env));
    
    let trade_indices = trade_history.get_token_trades(collection_id, token_id);
    let mut result = Vec::new(env);
    
    let max_results = if limit > 100 { 100 } else { limit } as u32;
    let start = if trade_indices.len() > max_results {
        trade_indices.len() - max_results
    } else {
        0
    };
    
    for i in start..trade_indices.len() {
        if let Some(index) = trade_indices.get(i) {
            if let Some(trade) = trade_history.trades.get(index) {
                result.push_back(trade);
            }
        }
    }
    
    result
}

/// Set NFT valuation
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `estimated_value` - Estimated value
/// * `method` - Valuation method
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn set_nft_valuation(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    estimated_value: i128,
    method: ValuationMethod,
) -> Result<(), NFTError> {
    // Verify NFT exists
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .ok_or(NFTError::NFTNotFound)?;
    
    if nft_registry.get_nft(collection_id, token_id).is_none() {
        return Err(NFTError::NFTNotFound);
    }
    
    let mut valuation_registry: ValuationRegistry = env.storage().instance()
        .get(&VALUATION_REGISTRY_KEY)
        .unwrap_or_else(|| ValuationRegistry::new(env));
    
    // Get trade history for last sale price
    let trade_history: TradeHistory = env.storage().instance()
        .get(&TRADE_HISTORY_KEY)
        .unwrap_or_else(|| TradeHistory::new(env));
    
    let trade_indices = trade_history.get_token_trades(collection_id, token_id);
    let last_sale_price = if trade_indices.is_empty() {
        0
    } else {
        if let Some(last_index) = trade_indices.get(trade_indices.len() - 1) {
            if let Some(last_trade) = trade_history.trades.get(last_index) {
                last_trade.price
            } else {
                0
            }
        } else {
            0
        }
    };
    
    // Calculate average sale price
    let mut total_price: i128 = 0;
    let mut sale_count: u32 = 0;
    for i in 0..trade_indices.len() {
        if let Some(index) = trade_indices.get(i) {
            if let Some(trade) = trade_history.trades.get(index) {
                total_price = total_price.saturating_add(trade.price);
                sale_count = sale_count.saturating_add(1);
            }
        }
    }
    let avg_sale_price = if sale_count > 0 {
        total_price / sale_count as i128
    } else {
        0
    };
    
    // Get collection floor price
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    
    let collection_floor = if let Some(collection) = collection_registry.get_collection(collection_id) {
        collection.floor_price
    } else {
        0
    };
    
    let valuation = NFTValuation {
        token_id,
        collection_id,
        estimated_value,
        last_sale_price,
        sale_count,
        avg_sale_price,
        collection_floor,
        valued_at: env.ledger().timestamp(),
        method,
    };
    
    valuation_registry.set_valuation(collection_id, token_id, valuation);
    env.storage().instance().set(&VALUATION_REGISTRY_KEY, &valuation_registry);
    
    // Emit event
    nft_events::emit_valuation_updated(env, collection_id, token_id, estimated_value, method);
    
    Ok(())
}

/// Get NFT valuation
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `Option<NFTValuation>` - Valuation info if available
pub fn get_nft_valuation(env: &Env, collection_id: u64, token_id: u64) -> Option<NFTValuation> {
    let valuation_registry: ValuationRegistry = env.storage().instance()
        .get(&VALUATION_REGISTRY_KEY)
        .unwrap_or_else(|| ValuationRegistry::new(env));
    valuation_registry.get_valuation(collection_id, token_id)
}

/// Update collection floor price
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `new_floor_price` - New floor price
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn update_collection_floor_price(
    env: &Env,
    collection_id: u64,
    new_floor_price: i128,
) -> Result<(), NFTError> {
    let mut valuation_registry: ValuationRegistry = env.storage().instance()
        .get(&VALUATION_REGISTRY_KEY)
        .unwrap_or_else(|| ValuationRegistry::new(env));
    
    valuation_registry.set_floor_price(collection_id, new_floor_price);
    env.storage().instance().set(&VALUATION_REGISTRY_KEY, &valuation_registry);
    
    // Emit event
    nft_events::emit_floor_price_updated(env, collection_id, new_floor_price);
    
    Ok(())
}

/// Get collection floor price
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// 
/// # Returns
/// * `i128` - Floor price (0 if not set)
pub fn get_collection_floor_price(env: &Env, collection_id: u64) -> i128 {
    let valuation_registry: ValuationRegistry = env.storage().instance()
        .get(&VALUATION_REGISTRY_KEY)
        .unwrap_or_else(|| ValuationRegistry::new(env));
    valuation_registry.get_floor_price(collection_id)
}

/// Admin function to pause/unpause the marketplace
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `admin` - Admin address
/// * `paused` - New paused state
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn set_marketplace_pause(
    env: &Env,
    admin: Address,
    paused: bool,
) -> Result<(), NFTError> {
    admin.require_auth();
    
    // Verify admin (using existing admin module)
    crate::admin::require_admin(env, &admin)
        .map_err(|_| NFTError::Unauthorized)?;
    
    set_marketplace_paused(env, paused);
    
    // Emit event
    nft_events::emit_marketplace_paused(env, paused, admin);
    
    Ok(())
}

/// Admin function to update platform fee
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `admin` - Admin address
/// * `new_fee_bps` - New platform fee in basis points
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn update_platform_fee(
    env: &Env,
    admin: Address,
    new_fee_bps: u32,
) -> Result<(), NFTError> {
    admin.require_auth();
    
    // Verify admin
    crate::admin::require_admin(env, &admin)
        .map_err(|_| NFTError::Unauthorized)?;
    
    let old_fee = get_platform_fee_bps(env);
    set_platform_fee_bps(env, new_fee_bps);
    
    // Emit event
    nft_events::emit_platform_fee_updated(env, old_fee, new_fee_bps);
    
    Ok(())
}

/// Admin function to update fee recipient
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `admin` - Admin address
/// * `new_recipient` - New fee recipient
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn update_fee_recipient(
    env: &Env,
    admin: Address,
    new_recipient: Address,
) -> Result<(), NFTError> {
    admin.require_auth();
    
    // Verify admin
    crate::admin::require_admin(env, &admin)
        .map_err(|_| NFTError::Unauthorized)?;
    
    set_fee_recipient(env, new_recipient);
    
    Ok(())
}

/// Get marketplace statistics
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// 
/// # Returns
/// * `(u64, u64, u64, u64, i128)` - (collections, nfts, listings, offers, volume)
pub fn get_marketplace_stats(env: &Env) -> (u64, u64, u64, u64, i128) {
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    let listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .unwrap_or_else(|| ListingRegistry::new(env));
    
    let offer_registry: OfferRegistry = env.storage().instance()
        .get(&OFFER_REGISTRY_KEY)
        .unwrap_or_else(|| OfferRegistry::new(env));
    
    let trade_history: TradeHistory = env.storage().instance()
        .get(&TRADE_HISTORY_KEY)
        .unwrap_or_else(|| TradeHistory::new(env));
    
    (
        collection_registry.total_collections,
        nft_registry.total_nfts,
        listing_registry.active_count,
        offer_registry.active_count,
        trade_history.total_volume,
    )
}

/// Get all collections with pagination
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `start` - Start index
/// * `limit` - Maximum number to return
/// 
/// # Returns
/// * `Vec<NFTCollection>` - List of collections
pub fn get_collections(
    env: &Env,
    start: u64,
    limit: u32,
) -> Vec<NFTCollection> {
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    
    let mut result = Vec::new(env);
    let max_limit = if limit > 50 { 50 } else { limit } as u64;
    
    for i in start..(start + max_limit) {
        if let Some(collection) = collection_registry.get_collection(i) {
            result.push_back(collection);
        }
    }
    
    result
}

/// Search NFTs by owner with pagination
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - Owner address
/// * `start` - Start index
/// * `limit` - Maximum number to return
/// 
/// # Returns
/// * `Vec<NFT>` - List of NFTs owned by the address
pub fn get_nfts_by_owner_paginated(
    env: &Env,
    owner: Address,
    start: u32,
    limit: u32,
) -> Vec<NFT> {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    let tokens = nft_registry.get_tokens_by_owner(owner);
    let mut result = Vec::new(env);
    
    let max_limit = if limit > 50 { 50 } else { limit };
    let end = (start + max_limit).min(tokens.len());
    
    for i in start..end {
        if let Some((collection_id, token_id)) = tokens.get(i) {
            if let Some(nft) = nft_registry.get_nft(collection_id, token_id) {
                result.push_back(nft);
            }
        }
    }
    
    result
}

/// Check if an address has any NFT activity
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `address` - Address to check
/// 
/// # Returns
/// * `bool` - True if address has NFT activity
pub fn has_nft_activity(env: &Env, address: Address) -> bool {
    let portfolio = get_nft_portfolio(env, address);
    portfolio.total_nfts > 0 || 
    portfolio.trades_count > 0 || 
    portfolio.active_listings > 0 || 
    portfolio.active_offers > 0
}

/// Get trending collections by volume
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `limit` - Maximum number to return
/// 
/// # Returns
/// * `Vec<(u64, i128)>` - List of (collection_id, volume) sorted by volume
pub fn get_trending_collections(env: &Env, limit: u32) -> Vec<(u64, i128)> {
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    
    let mut collections: Vec<(u64, i128)> = Vec::new(env);
    
    // Collect all collections with their volume
    for i in 1..=collection_registry.total_collections {
        if let Some(collection) = collection_registry.get_collection(i) {
            collections.push_back((collection.collection_id, collection.total_volume));
        }
    }
    
    // Simple bubble sort by volume (descending)
    let len = collections.len();
    for i in 0..len {
        for j in 0..(len - 1 - i) {
            if let (Some((_, vol1)), Some((_, vol2))) = (collections.get(j), collections.get(j + 1)) {
                if vol1 < vol2 {
                    let temp1 = collections.get(j).unwrap();
                    let temp2 = collections.get(j + 1).unwrap();
                    collections.set(j, temp2);
                    collections.set(j + 1, temp1);
                }
            }
        }
    }
    
    // Return top N
    let max_results = if limit > 20 { 20 } else { limit };
    let mut result = Vec::new(env);
    for i in 0..max_results.min(collections.len()) {
        if let Some(item) = collections.get(i) {
            result.push_back(item);
        }
    }
    
    result
}
