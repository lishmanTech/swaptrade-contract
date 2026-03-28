#![cfg_attr(not(test), no_std)]
use soroban_sdk::{Address, Env, Map, Vec};
use crate::nft_types::*;
use crate::nft_errors::NFTError;
use crate::nft_storage::*;
use crate::nft_minting::{get_nft, is_owner};
use crate::emergency;

/// Maximum number of shares for fractionalization
const MAX_SHARES: u64 = 1_000_000;
/// Minimum number of shares for fractionalization
const MIN_SHARES: u64 = 2;

/// Fractionalize an NFT into tradable shares
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - NFT owner
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `total_shares` - Total number of shares to create
/// * `initial_price_per_share` - Initial price per share
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn fractionalize_nft(
    env: &Env,
    owner: Address,
    collection_id: u64,
    token_id: u64,
    total_shares: u64,
    initial_price_per_share: i128,
) -> Result<(), NFTError> {
    owner.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, owner.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate share count
    if total_shares < MIN_SHARES || total_shares > MAX_SHARES {
        return Err(NFTError::FractionalizationLimit);
    }
    
    // Validate price
    if initial_price_per_share <= 0 {
        return Err(NFTError::InvalidPrice);
    }
    
    // Get NFT
    let mut nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .ok_or(NFTError::NFTNotFound)?;
    
    let mut nft = nft_registry.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    
    // Verify ownership
    if nft.owner != owner {
        return Err(NFTError::NotOwner);
    }
    
    // Check if already fractionalized
    if nft.is_fractionalized {
        return Err(NFTError::AlreadyFractionalized);
    }
    
    // Only ERC-721 can be fractionalized
    if nft.standard != NFTStandard::ERC721 {
        return Err(NFTError::UnsupportedOperation);
    }
    
    // Check if NFT is collateralized
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    if loan_registry.get_loan_by_collateral(collection_id, token_id).is_some() {
        return Err(NFTError::AlreadyCollateralized);
    }
    
    // Mark NFT as fractionalized
    nft.is_fractionalized = true;
    nft.total_supply = total_shares;
    nft.circulating_supply = 0; // Will be distributed
    
    nft_registry.update_nft(nft);
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    
    // Create fractional shares for owner (all shares initially)
    let mut fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .unwrap_or_else(|| FractionalRegistry::new(env));
    
    let share = FractionalShare {
        token_id,
        collection_id,
        shareholder: owner.clone(),
        shares: total_shares,
        total_shares,
        purchase_price: initial_price_per_share,
        acquired_at: env.ledger().timestamp(),
    };
    
    fractional_registry.set_shares(collection_id, token_id, owner.clone(), share);
    env.storage().instance().set(&FRACTIONAL_SHARES_KEY, &fractional_registry);
    
    // Emit event
    crate::nft_events::emit_nft_fractionalized(
        env, collection_id, token_id, owner, total_shares, initial_price_per_share
    );
    
    Ok(())
}

/// Buy fractional shares of an NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `buyer` - Share buyer
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `shares_to_buy` - Number of shares to purchase
/// * `max_price_per_share` - Maximum price willing to pay per share
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn buy_fractional_shares(
    env: &Env,
    buyer: Address,
    collection_id: u64,
    token_id: u64,
    shares_to_buy: u64,
    max_price_per_share: i128,
) -> Result<(), NFTError> {
    buyer.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, buyer.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate shares
    if shares_to_buy == 0 {
        return Err(NFTError::InvalidShareAmount);
    }
    
    // Get NFT
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .ok_or(NFTError::NFTNotFound)?;
    
    let nft = nft_registry.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    
    // Check if fractionalized
    if !nft.is_fractionalized {
        return Err(NFTError::NotFractionalized);
    }
    
    // Check available shares
    let available_shares = nft.total_supply.saturating_sub(nft.circulating_supply);
    if shares_to_buy > available_shares {
        return Err(NFTError::NoFractionsAvailable);
    }
    
    // Get fractional registry
    let mut fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .unwrap_or_else(|| FractionalRegistry::new(env));
    
    // Get or create buyer's share
    let mut buyer_share = fractional_registry.get_shares(collection_id, token_id, buyer.clone())
        .unwrap_or_else(|| FractionalShare {
            token_id,
            collection_id,
            shareholder: buyer.clone(),
            shares: 0,
            total_shares: nft.total_supply,
            purchase_price: max_price_per_share,
            acquired_at: env.ledger().timestamp(),
        });
    
    // Update buyer's shares
    buyer_share.shares = buyer_share.shares.saturating_add(shares_to_buy);
    buyer_share.purchase_price = max_price_per_share;
    buyer_share.acquired_at = env.ledger().timestamp();
    
    fractional_registry.set_shares(collection_id, token_id, buyer.clone(), buyer_share);
    
    // Update NFT circulating supply
    let mut nft_registry_mut: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    let mut nft_mut = nft_registry_mut.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    nft_mut.circulating_supply = nft_mut.circulating_supply.saturating_add(shares_to_buy);
    nft_registry_mut.update_nft(nft_mut);
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry_mut);
    
    env.storage().instance().set(&FRACTIONAL_SHARES_KEY, &fractional_registry);
    
    // Emit event
    crate::nft_events::emit_fractional_shares_purchased(
        env, collection_id, token_id, buyer, shares_to_buy, max_price_per_share
    );
    
    Ok(())
}

/// Sell fractional shares back to the pool
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `seller` - Share seller
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `shares_to_sell` - Number of shares to sell
/// * `min_price_per_share` - Minimum price per share
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn sell_fractional_shares(
    env: &Env,
    seller: Address,
    collection_id: u64,
    token_id: u64,
    shares_to_sell: u64,
    min_price_per_share: i128,
) -> Result<(), NFTError> {
    seller.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    // Validate shares
    if shares_to_sell == 0 {
        return Err(NFTError::InvalidShareAmount);
    }
    
    // Get fractional registry
    let mut fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .ok_or(NFTError::NotFractionalized)?;
    
    // Get seller's shares
    let mut seller_share = fractional_registry.get_shares(collection_id, token_id, seller.clone())
        .ok_or(NFTError::InsufficientBalance)?;
    
    // Check if seller has enough shares
    if seller_share.shares < shares_to_sell {
        return Err(NFTError::InsufficientBalance);
    }
    
    // Update seller's shares
    seller_share.shares = seller_share.shares.saturating_sub(shares_to_sell);
    
    if seller_share.shares == 0 {
        // Remove shareholder if no shares left
        fractional_registry.remove_shareholder(env, collection_id, token_id, seller.clone());
    } else {
        fractional_registry.set_shares(collection_id, token_id, seller.clone(), seller_share);
    }
    
    // Update NFT circulating supply
    let mut nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    let mut nft = nft_registry.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    nft.circulating_supply = nft.circulating_supply.saturating_sub(shares_to_sell);
    nft_registry.update_nft(nft);
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    
    env.storage().instance().set(&FRACTIONAL_SHARES_KEY, &fractional_registry);
    
    // Emit event
    crate::nft_events::emit_fractional_shares_sold(
        env, collection_id, token_id, seller, shares_to_sell, min_price_per_share
    );
    
    Ok(())
}

/// Transfer fractional shares to another address
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `from` - Sender address
/// * `to` - Recipient address
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `shares` - Number of shares to transfer
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn transfer_fractional_shares(
    env: &Env,
    from: Address,
    to: Address,
    collection_id: u64,
    token_id: u64,
    shares: u64,
) -> Result<(), NFTError> {
    from.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, from.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate shares
    if shares == 0 {
        return Err(NFTError::InvalidShareAmount);
    }
    
    // Prevent self-transfer
    if from == to {
        return Err(NFTError::SelfDealing);
    }
    
    // Get NFT
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .ok_or(NFTError::NFTNotFound)?;
    
    let nft = nft_registry.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    
    // Check if fractionalized
    if !nft.is_fractionalized {
        return Err(NFTError::NotFractionalized);
    }
    
    // Get fractional registry
    let mut fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .ok_or(NFTError::NotFractionalized)?;
    
    // Get sender's shares
    let mut from_share = fractional_registry.get_shares(collection_id, token_id, from.clone())
        .ok_or(NFTError::InsufficientBalance)?;
    
    // Check if sender has enough shares
    if from_share.shares < shares {
        return Err(NFTError::InsufficientBalance);
    }
    
    // Update sender's shares
    from_share.shares = from_share.shares.saturating_sub(shares);
    
    if from_share.shares == 0 {
        fractional_registry.remove_shareholder(env, collection_id, token_id, from.clone());
    } else {
        fractional_registry.set_shares(collection_id, token_id, from.clone(), from_share);
    }
    
    // Get or create recipient's shares
    let mut to_share = fractional_registry.get_shares(collection_id, token_id, to.clone())
        .unwrap_or_else(|| FractionalShare {
            token_id,
            collection_id,
            shareholder: to.clone(),
            shares: 0,
            total_shares: nft.total_supply,
            purchase_price: 0,
            acquired_at: env.ledger().timestamp(),
        });
    
    // Update recipient's shares
    to_share.shares = to_share.shares.saturating_add(shares);
    to_share.acquired_at = env.ledger().timestamp();
    
    fractional_registry.set_shares(collection_id, token_id, to.clone(), to_share);
    env.storage().instance().set(&FRACTIONAL_SHARES_KEY, &fractional_registry);
    
    // Emit event
    crate::nft_events::emit_fractional_shares_transferred(
        env, collection_id, token_id, from, to, shares
    );
    
    Ok(())
}

/// Defractionalize an NFT (requires owning all shares)
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - Shareholder attempting to defractionalize
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn defractionalize_nft(
    env: &Env,
    owner: Address,
    collection_id: u64,
    token_id: u64,
) -> Result<(), NFTError> {
    owner.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    // Get NFT
    let mut nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .ok_or(NFTError::NFTNotFound)?;
    
    let mut nft = nft_registry.get_nft(collection_id, token_id)
        .ok_or(NFTError::NFTNotFound)?;
    
    // Check if fractionalized
    if !nft.is_fractionalized {
        return Err(NFTError::NotFractionalized);
    }
    
    // Get fractional registry
    let fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .ok_or(NFTError::NotFractionalized)?;
    
    // Get owner's shares
    let owner_share = fractional_registry.get_shares(collection_id, token_id, owner.clone())
        .ok_or(NFTError::InsufficientBalance)?;
    
    // Check if owner has all shares
    if owner_share.shares != nft.total_supply {
        return Err(NFTError::InsufficientBalance);
    }
    
    // Defractionalize the NFT
    nft.is_fractionalized = false;
    nft.total_supply = 1;
    nft.circulating_supply = 1;
    
    nft_registry.update_nft(nft);
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    
    // Remove all fractional shares
    let mut fractional_registry_mut: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .unwrap_or_else(|| FractionalRegistry::new(env));
    
    let shareholders = fractional_registry_mut.get_shareholders(collection_id, token_id);
    for i in 0..shareholders.len() {
        if let Some(shareholder) = shareholders.get(i) {
            fractional_registry_mut.remove_shareholder(env, collection_id, token_id, shareholder);
        }
    }
    
    env.storage().instance().set(&FRACTIONAL_SHARES_KEY, &fractional_registry_mut);
    
    // Emit event
    crate::nft_events::emit_nft_defractionalized(env, collection_id, token_id, owner);
    
    Ok(())
}

/// Get fractional shares for a shareholder
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `shareholder` - Shareholder address
/// 
/// # Returns
/// * `Option<FractionalShare>` - Share info if exists
pub fn get_fractional_shares(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    shareholder: Address,
) -> Option<FractionalShare> {
    let fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .unwrap_or_else(|| FractionalRegistry::new(env));
    fractional_registry.get_shares(collection_id, token_id, shareholder)
}

/// Get all shareholders for a fractionalized NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `Vec<Address>` - List of shareholders
pub fn get_shareholders(env: &Env, collection_id: u64, token_id: u64) -> Vec<Address> {
    let fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .unwrap_or_else(|| FractionalRegistry::new(env));
    fractional_registry.get_shareholders(collection_id, token_id)
}

/// Get total shares for an NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `u64` - Total shares (0 if not fractionalized)
pub fn get_total_shares(env: &Env, collection_id: u64, token_id: u64) -> u64 {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    if let Some(nft) = nft_registry.get_nft(collection_id, token_id) {
        if nft.is_fractionalized {
            nft.total_supply
        } else {
            0
        }
    } else {
        0
    }
}

/// Get circulating shares for an NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `u64` - Circulating shares
pub fn get_circulating_shares(env: &Env, collection_id: u64, token_id: u64) -> u64 {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    if let Some(nft) = nft_registry.get_nft(collection_id, token_id) {
        nft.circulating_supply
    } else {
        0
    }
}

/// Check if an NFT is fractionalized
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `bool` - True if fractionalized
pub fn is_fractionalized(env: &Env, collection_id: u64, token_id: u64) -> bool {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    if let Some(nft) = nft_registry.get_nft(collection_id, token_id) {
        nft.is_fractionalized
    } else {
        false
    }
}

/// Calculate the ownership percentage for a shareholder
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `shareholder` - Shareholder address
/// 
/// # Returns
/// * `u32` - Ownership percentage in basis points (e.g., 5000 = 50%)
pub fn get_ownership_percentage(
    env: &Env,
    collection_id: u64,
    token_id: u64,
    shareholder: Address,
) -> u32 {
    let fractional_registry: FractionalRegistry = env.storage().instance()
        .get(&FRACTIONAL_SHARES_KEY)
        .unwrap_or_else(|| FractionalRegistry::new(env));
    
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    if let (Some(share), Some(nft)) = (
        fractional_registry.get_shares(collection_id, token_id, shareholder),
        nft_registry.get_nft(collection_id, token_id)
    ) {
        if nft.total_supply > 0 {
            ((share.shares as u128 * 10000) / nft.total_supply as u128) as u32
        } else {
            0
        }
    } else {
        0
    }
}
