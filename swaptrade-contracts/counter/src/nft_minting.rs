#![cfg_attr(not(test), no_std)]
use soroban_sdk::{Address, Env, String, Symbol, symbol_short};
use crate::nft_types::*;
use crate::nft_errors::NFTError;
use crate::nft_storage::*;
use crate::emergency;

/// Maximum royalty in basis points (10%)
const MAX_ROYALTY_BPS: u32 = 1000;
/// Maximum supply for a collection
const MAX_COLLECTION_SUPPLY: u64 = 1_000_000;

/// Create a new NFT collection
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - The collection owner/creator
/// * `name` - Collection name
/// * `symbol` - Collection symbol/ticker
/// * `description` - Collection description
/// * `base_uri` - Base URI for metadata
/// * `max_supply` - Maximum supply (0 for unlimited)
/// * `royalty_bps` - Royalty percentage in basis points
/// * `royalty_recipient` - Address to receive royalties
/// 
/// # Returns
/// * `Result<u64, NFTError>` - Collection ID on success
pub fn create_collection(
    env: &Env,
    owner: Address,
    name: String,
    symbol: String,
    description: String,
    base_uri: String,
    max_supply: u64,
    royalty_bps: u32,
    royalty_recipient: Address,
) -> Result<u64, NFTError> {
    owner.require_auth();
    
    // Check if marketplace is paused
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    // Check if user is frozen
    if emergency::is_frozen(env, owner.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate inputs
    if name.is_empty() {
        return Err(NFTError::InvalidMetadata);
    }
    
    if royalty_bps > MAX_ROYALTY_BPS {
        return Err(NFTError::ExcessiveRoyalty);
    }
    
    if max_supply > MAX_COLLECTION_SUPPLY {
        return Err(NFTError::MaxSupplyReached);
    }
    
    // Check if collection name already exists
    let name_symbol = Symbol::new(env, &name.to_string());
    let mut collection_registry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    
    // Generate collection ID
    let collection_id = get_next_collection_id(env);
    
    // Create collection
    let collection = NFTCollection {
        collection_id,
        owner: owner.clone(),
        name,
        symbol,
        description,
        base_uri,
        total_supply: 0,
        unique_holders: 0,
        floor_price: 0,
        total_volume: 0,
        minting_active: true,
        max_supply,
        royalty_bps,
        royalty_recipient,
        created_at: env.ledger().timestamp(),
    };
    
    // Store collection
    collection_registry.store_collection(env, collection);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry);
    
    // Emit event
    crate::nft_events::emit_collection_created(env, collection_id, owner);
    
    Ok(collection_id)
}

/// Mint a new NFT in a collection
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `creator` - The NFT creator (must be collection owner or authorized)
/// * `collection_id` - The collection to mint in
/// * `metadata_uri` - URI to NFT metadata
/// * `standard` - NFT standard type (ERC721 or ERC1155)
/// * `amount` - Amount for ERC-1155 (1 for ERC-721)
/// 
/// # Returns
/// * `Result<u64, NFTError>` - Token ID on success
pub fn mint_nft(
    env: &Env,
    creator: Address,
    collection_id: u64,
    metadata_uri: String,
    standard: NFTStandard,
    amount: u64,
) -> Result<u64, NFTError> {
    creator.require_auth();
    
    // Check if marketplace is paused
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    // Check if user is frozen
    if emergency::is_frozen(env, creator.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Get collection registry
    let mut collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .ok_or(NFTError::CollectionNotFound)?;
    
    // Get collection
    let mut collection = collection_registry.get_collection(collection_id)
        .ok_or(NFTError::CollectionNotFound)?;
    
    // Verify creator is collection owner
    if collection.owner != creator {
        return Err(NFTError::NotCreator);
    }
    
    // Check if minting is active
    if !collection.minting_active {
        return Err(NFTError::MintingNotActive);
    }
    
    // Check max supply
    if collection.max_supply > 0 && collection.total_supply >= collection.max_supply {
        return Err(NFTError::MaxSupplyReached);
    }
    
    // Validate amount based on standard
    let final_amount = match standard {
        NFTStandard::ERC721 => {
            if amount != 1 && amount != 0 {
                return Err(NFTError::InvalidAmount);
            }
            1
        }
        NFTStandard::ERC1155 => {
            if amount == 0 {
                return Err(NFTError::InvalidAmount);
            }
            amount
        }
    };
    
    // Generate token ID
    let token_id = get_next_token_id(env);
    
    // Get NFT registry
    let mut nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    // Create NFT
    let nft = NFT {
        token_id,
        contract_address: env.current_contract_address(),
        owner: creator.clone(),
        creator: creator.clone(),
        collection_id,
        standard: standard.clone(),
        metadata_uri,
        is_fractionalized: false,
        total_supply: final_amount,
        circulating_supply: final_amount,
        created_at: env.ledger().timestamp(),
    };
    
    // Store NFT
    nft_registry.store_nft(env, nft.clone());
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    
    // Update collection
    collection.total_supply = collection.total_supply.saturating_add(1);
    collection_registry.update_collection(collection);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry);
    
    // Update user's NFT portfolio
    update_portfolio_on_mint(env, creator.clone(), collection_id, token_id)?;
    
    // Emit event
    crate::nft_events::emit_nft_minted(env, collection_id, token_id, creator, final_amount);
    
    Ok(token_id)
}

/// Batch mint multiple NFTs
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `creator` - The NFT creator
/// * `collection_id` - The collection to mint in
/// * `metadata_uris` - Vector of metadata URIs
/// * `standard` - NFT standard type
/// 
/// # Returns
/// * `Result<Vec<u64>, NFTError>` - Vector of token IDs on success
pub fn batch_mint(
    env: &Env,
    creator: Address,
    collection_id: u64,
    metadata_uris: Vec<String>,
    standard: NFTStandard,
) -> Result<Vec<u64>, NFTError> {
    creator.require_auth();
    
    let mut token_ids = Vec::new(env);
    
    for i in 0..metadata_uris.len() {
        if let Some(uri) = metadata_uris.get(i) {
            let token_id = mint_nft(env, creator.clone(), collection_id, uri, standard.clone(), 1)?;
            token_ids.push_back(token_id);
        }
    }
    
    Ok(token_ids)
}

/// Toggle minting status for a collection
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - Collection owner
/// * `collection_id` - Collection ID
/// * `active` - New minting status
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn set_minting_status(
    env: &Env,
    owner: Address,
    collection_id: u64,
    active: bool,
) -> Result<(), NFTError> {
    owner.require_auth();
    
    let mut collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .ok_or(NFTError::CollectionNotFound)?;
    
    let mut collection = collection_registry.get_collection(collection_id)
        .ok_or(NFTError::CollectionNotFound)?;
    
    // Verify ownership
    if collection.owner != owner {
        return Err(NFTError::NotOwner);
    }
    
    collection.minting_active = active;
    collection_registry.update_collection(collection);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry);
    
    // Emit event
    crate::nft_events::emit_minting_status_changed(env, collection_id, active);
    
    Ok(())
}

/// Update collection metadata
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - Collection owner
/// * `collection_id` - Collection ID
/// * `new_base_uri` - New base URI (optional)
/// * `new_royalty_bps` - New royalty in basis points (optional)
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn update_collection_metadata(
    env: &Env,
    owner: Address,
    collection_id: u64,
    new_base_uri: Option<String>,
    new_royalty_bps: Option<u32>,
) -> Result<(), NFTError> {
    owner.require_auth();
    
    let mut collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .ok_or(NFTError::CollectionNotFound)?;
    
    let mut collection = collection_registry.get_collection(collection_id)
        .ok_or(NFTError::CollectionNotFound)?;
    
    // Verify ownership
    if collection.owner != owner {
        return Err(NFTError::NotOwner);
    }
    
    // Update base URI if provided
    if let Some(uri) = new_base_uri {
        collection.base_uri = uri;
    }
    
    // Update royalty if provided
    if let Some(royalty) = new_royalty_bps {
        if royalty > MAX_ROYALTY_BPS {
            return Err(NFTError::ExcessiveRoyalty);
        }
        collection.royalty_bps = royalty;
    }
    
    collection_registry.update_collection(collection);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry);
    
    Ok(())
}

/// Transfer collection ownership
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `current_owner` - Current collection owner
/// * `collection_id` - Collection ID
/// * `new_owner` - New owner address
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn transfer_collection_ownership(
    env: &Env,
    current_owner: Address,
    collection_id: u64,
    new_owner: Address,
) -> Result<(), NFTError> {
    current_owner.require_auth();
    
    let mut collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .ok_or(NFTError::CollectionNotFound)?;
    
    let mut collection = collection_registry.get_collection(collection_id)
        .ok_or(NFTError::CollectionNotFound)?;
    
    // Verify ownership
    if collection.owner != current_owner {
        return Err(NFTError::NotOwner);
    }
    
    collection.owner = new_owner.clone();
    collection_registry.update_collection(collection);
    env.storage().instance().set(&COLLECTION_REGISTRY_KEY, &collection_registry);
    
    // Emit event
    crate::nft_events::emit_collection_ownership_transferred(env, collection_id, current_owner, new_owner);
    
    Ok(())
}

/// Update user's portfolio when minting
fn update_portfolio_on_mint(
    env: &Env,
    owner: Address,
    collection_id: u64,
    token_id: u64,
) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(owner.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, owner.clone()));
    
    portfolio.add_nft(token_id, collection_id);
    
    portfolio_registry.set(owner.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Get collection info
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// 
/// # Returns
/// * `Option<NFTCollection>` - Collection info if found
pub fn get_collection(env: &Env, collection_id: u64) -> Option<NFTCollection> {
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)?;
    collection_registry.get_collection(collection_id)
}

/// Get NFT info
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `Option<NFT>` - NFT info if found
pub fn get_nft(env: &Env, collection_id: u64, token_id: u64) -> Option<NFT> {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)?;
    nft_registry.get_nft(collection_id, token_id)
}

/// Get all collections owned by an address
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - Owner address
/// 
/// # Returns
/// * `Vec<u64>` - Vector of collection IDs
pub fn get_collections_by_owner(env: &Env, owner: Address) -> Vec<u64> {
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    collection_registry.get_collections_by_owner(owner)
}

/// Get all NFTs owned by an address
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `owner` - Owner address
/// 
/// # Returns
/// * `Vec<(u64, u64)>` - Vector of (collection_id, token_id) tuples
pub fn get_nfts_by_owner(env: &Env, owner: Address) -> Vec<(u64, u64)> {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    nft_registry.get_tokens_by_owner(owner)
}

/// Check if an address owns a specific NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// * `owner` - Address to check
/// 
/// # Returns
/// * `bool` - True if address owns the NFT
pub fn is_owner(env: &Env, collection_id: u64, token_id: u64, owner: Address) -> bool {
    if let Some(nft) = get_nft(env, collection_id, token_id) {
        nft.owner == owner
    } else {
        false
    }
}

/// Get total collections count
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// 
/// # Returns
/// * `u64` - Total number of collections
pub fn get_total_collections(env: &Env) -> u64 {
    let collection_registry: CollectionRegistry = env.storage().instance()
        .get(&COLLECTION_REGISTRY_KEY)
        .unwrap_or_else(|| CollectionRegistry::new(env));
    collection_registry.total_collections
}

/// Get total NFTs minted
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// 
/// # Returns
/// * `u64` - Total number of NFTs
pub fn get_total_nfts(env: &Env) -> u64 {
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    nft_registry.total_nfts
}
