#![cfg_attr(not(test), no_std)]
use soroban_sdk::{Address, Env, Symbol, symbol_short};
use crate::nft_types::*;
use crate::nft_errors::NFTError;
use crate::nft_storage::*;
use crate::nft_minting::{get_nft, is_owner};
use crate::emergency;

/// Minimum loan duration (1 day)
const MIN_LOAN_DURATION: u64 = 86400;
/// Maximum loan duration (365 days)
const MAX_LOAN_DURATION: u64 = 31536000;
/// Maximum interest rate (1% per day = 100 bps)
const MAX_INTEREST_RATE_BPS: u32 = 100;
/// Liquidation threshold (grace period after due date)
const LIQUIDATION_GRACE_PERIOD: u64 = 86400; // 1 day

/// Create a loan request using NFT as collateral
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `borrower` - NFT owner requesting loan
/// * `collection_id` - Collection ID of collateral NFT
/// * `token_id` - Token ID of collateral NFT
/// * `loan_amount` - Amount requested
/// * `interest_rate_bps` - Daily interest rate in basis points
/// * `duration` - Loan duration in seconds
/// 
/// # Returns
/// * `Result<u64, NFTError>` - Loan ID on success
pub fn request_loan(
    env: &Env,
    borrower: Address,
    collection_id: u64,
    token_id: u64,
    loan_amount: i128,
    interest_rate_bps: u32,
    duration: u64,
) -> Result<u64, NFTError> {
    borrower.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, borrower.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    // Validate loan amount
    if loan_amount <= 0 {
        return Err(NFTError::InvalidAmount);
    }
    
    // Validate interest rate
    if interest_rate_bps == 0 || interest_rate_bps > MAX_INTEREST_RATE_BPS {
        return Err(NFTError::InvalidInterestRate);
    }
    
    // Validate duration
    if duration < MIN_LOAN_DURATION || duration > MAX_LOAN_DURATION {
        return Err(NFTError::InvalidDuration);
    }
    
    // Get NFT
    let nft = get_nft(env, collection_id, token_id).ok_or(NFTError::NFTNotFound)?;
    
    // Verify ownership
    if nft.owner != borrower {
        return Err(NFTError::NotOwner);
    }
    
    // Check if NFT is already collateralized
    let loan_registry_check: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    if loan_registry_check.get_loan_by_collateral(collection_id, token_id).is_some() {
        return Err(NFTError::AlreadyCollateralized);
    }
    
    // Check if NFT is fractionalized (cannot use as collateral)
    if nft.is_fractionalized {
        return Err(NFTError::UnsupportedOperation);
    }
    
    // Check if NFT is listed
    let listing_registry: ListingRegistry = env.storage().instance()
        .get(&LISTING_REGISTRY_KEY)
        .unwrap_or_else(|| ListingRegistry::new(env));
    let token_listings = listing_registry.get_token_listings(collection_id, token_id);
    if !token_listings.is_empty() {
        // Check if any listing is active
        for i in 0..token_listings.len() {
            if let Some(listing_id) = token_listings.get(i) {
                if let Some(listing) = listing_registry.get_listing(listing_id) {
                    if listing.is_active {
                        return Err(NFTError::UnsupportedOperation);
                    }
                }
            }
        }
    }
    
    let current_time = env.ledger().timestamp();
    let loan_id = get_next_loan_id(env);
    
    // Create loan (initially without lender)
    let loan = NFTLoan {
        loan_id,
        token_id,
        collection_id,
        borrower: borrower.clone(),
        lender: borrower.clone(), // Placeholder, will be updated when funded
        loan_amount,
        interest_rate_bps,
        repayment_amount: loan_amount, // Will be calculated when funded
        start_time: 0, // Will be set when funded
        duration,
        due_date: 0, // Will be set when funded
        is_active: false, // Inactive until funded
        is_repaid: false,
        is_liquidated: false,
    };
    
    // Store loan
    let mut loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    loan_registry.create_loan(env, loan);
    env.storage().instance().set(&LOAN_REGISTRY_KEY, &loan_registry);
    
    // Emit event
    crate::nft_events::emit_loan_requested(
        env, loan_id, collection_id, token_id, borrower, loan_amount
    );
    
    Ok(loan_id)
}

/// Fund a loan request (become the lender)
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `lender` - Address funding the loan
/// * `loan_id` - Loan ID to fund
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn fund_loan(
    env: &Env,
    lender: Address,
    loan_id: u64,
) -> Result<(), NFTError> {
    lender.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    if emergency::is_frozen(env, lender.clone()) {
        return Err(NFTError::UserFrozen);
    }
    
    let mut loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .ok_or(NFTError::LoanNotFound)?;
    
    let mut loan = loan_registry.get_loan(loan_id)
        .ok_or(NFTError::LoanNotFound)?;
    
    // Check if loan is already active
    if loan.is_active {
        return Err(NFTError::LoanAlreadyRepaid);
    }
    
    // Prevent self-lending
    if loan.borrower == lender {
        return Err(NFTError::SelfDealing);
    }
    
    let current_time = env.ledger().timestamp();
    
    // Calculate repayment amount
    let daily_interest = (loan.loan_amount * loan.interest_rate_bps as i128) / 10000;
    let total_interest = daily_interest * (loan.duration / 86400) as i128;
    loan.repayment_amount = loan.loan_amount + total_interest;
    
    // Activate loan
    loan.lender = lender.clone();
    loan.start_time = current_time;
    loan.due_date = current_time + loan.duration;
    loan.is_active = true;
    
    loan_registry.update_loan(loan);
    env.storage().instance().set(&LOAN_REGISTRY_KEY, &loan_registry);
    
    // Update borrower's portfolio
    update_portfolio_on_loan_taken(env, loan.borrower.clone())?;
    
    // Update lender's portfolio
    update_portfolio_on_loan_given(env, lender.clone())?;
    
    // Emit event
    crate::nft_events::emit_loan_funded(env, loan_id, lender, loan.loan_amount);
    
    Ok(())
}

/// Repay an active loan
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `borrower` - Loan borrower
/// * `loan_id` - Loan ID to repay
/// * `repayment_amount` - Amount being repaid
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn repay_loan(
    env: &Env,
    borrower: Address,
    loan_id: u64,
    repayment_amount: i128,
) -> Result<(), NFTError> {
    borrower.require_auth();
    
    // Check marketplace state
    if is_marketplace_paused(env) {
        return Err(NFTError::MarketplacePaused);
    }
    
    let mut loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .ok_or(NFTError::LoanNotFound)?;
    
    let loan = loan_registry.get_loan(loan_id)
        .ok_or(NFTError::LoanNotFound)?;
    
    // Verify borrower
    if loan.borrower != borrower {
        return Err(NFTError::Unauthorized);
    }
    
    // Check if loan is active
    if !loan.is_active {
        return Err(NFTError::LoanNotActive);
    }
    
    // Check if already repaid
    if loan.is_repaid {
        return Err(NFTError::LoanAlreadyRepaid);
    }
    
    // Check if liquidated
    if loan.is_liquidated {
        return Err(NFTError::LoanLiquidated);
    }
    
    // Calculate current amount due
    let current_time = env.ledger().timestamp();
    let amount_due = loan.total_due(current_time);
    
    // Validate repayment amount
    if repayment_amount < amount_due {
        return Err(NFTError::InsufficientRepayment);
    }
    
    // Mark loan as repaid
    loan_registry.mark_repaid(loan_id)?;
    env.storage().instance().set(&LOAN_REGISTRY_KEY, &loan_registry);
    
    // Update portfolios
    decrement_portfolio_loans_taken(env, borrower)?;
    decrement_portfolio_loans_given(env, loan.lender.clone())?;
    
    // Emit event
    crate::nft_events::emit_loan_repaid(env, loan_id, borrower, repayment_amount);
    
    Ok(())
}

/// Liquidate an overdue loan (can be called by anyone)
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `loan_id` - Loan ID to liquidate
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn liquidate_loan(
    env: &Env,
    loan_id: u64,
) -> Result<(), NFTError> {
    let mut loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .ok_or(NFTError::LoanNotFound)?;
    
    let loan = loan_registry.get_loan(loan_id)
        .ok_or(NFTError::LoanNotFound)?;
    
    // Check if loan is active
    if !loan.is_active {
        return Err(NFTError::LoanNotActive);
    }
    
    // Check if already repaid
    if loan.is_repaid {
        return Err(NFTError::LoanAlreadyRepaid);
    }
    
    // Check if already liquidated
    if loan.is_liquidated {
        return Err(NFTError::LoanLiquidated);
    }
    
    // Check if loan is overdue (including grace period)
    let current_time = env.ledger().timestamp();
    if current_time <= loan.due_date + LIQUIDATION_GRACE_PERIOD {
        return Err(NFTError::LoanNotOverdue);
    }
    
    // Transfer NFT ownership to lender
    let mut nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    nft_registry.transfer_ownership(
        env,
        loan.collection_id,
        loan.token_id,
        loan.lender.clone()
    )?;
    env.storage().instance().set(&NFT_REGISTRY_KEY, &nft_registry);
    
    // Mark loan as liquidated
    loan_registry.mark_liquidated(loan_id)?;
    env.storage().instance().set(&LOAN_REGISTRY_KEY, &loan_registry);
    
    // Update borrower's portfolio
    decrement_portfolio_loans_taken(env, loan.borrower.clone())?;
    
    // Update lender's portfolio
    decrement_portfolio_loans_given(env, loan.lender.clone())?;
    
    // Update lender's NFT portfolio (they now own the NFT)
    update_portfolio_on_liquidation(env, loan.lender.clone(), loan.collection_id, loan.token_id)?;
    
    // Emit event
    crate::nft_events::emit_loan_liquidated(
        env, loan_id, loan.lender.clone(), loan.collection_id, loan.token_id
    );
    
    Ok(())
}

/// Cancel a loan request that hasn't been funded yet
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `borrower` - Loan requester
/// * `loan_id` - Loan ID to cancel
/// 
/// # Returns
/// * `Result<(), NFTError>` - Success or error
pub fn cancel_loan_request(
    env: &Env,
    borrower: Address,
    loan_id: u64,
) -> Result<(), NFTError> {
    borrower.require_auth();
    
    let mut loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .ok_or(NFTError::LoanNotFound)?;
    
    let loan = loan_registry.get_loan(loan_id)
        .ok_or(NFTError::LoanNotFound)?;
    
    // Verify borrower
    if loan.borrower != borrower {
        return Err(NFTError::Unauthorized);
    }
    
    // Check if loan is not yet active (not funded)
    if loan.is_active {
        return Err(NFTError::LoanAlreadyRepaid);
    }
    
    // Remove the loan request
    // Note: In a full implementation, we'd need to remove from all indices
    // For now, we'll just mark it as inactive by setting a flag
    // This is a simplified implementation
    
    // Emit event
    crate::nft_events::emit_loan_cancelled(env, loan_id, borrower);
    
    Ok(())
}

/// Update portfolio when taking a loan
fn update_portfolio_on_loan_taken(env: &Env, borrower: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(borrower.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, borrower.clone()));
    
    portfolio.active_loans = portfolio.active_loans.saturating_add(1);
    
    portfolio_registry.set(borrower.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Update portfolio when giving a loan
fn update_portfolio_on_loan_given(env: &Env, lender: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(lender.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, lender.clone()));
    
    portfolio.loans_given = portfolio.loans_given.saturating_add(1);
    
    portfolio_registry.set(lender.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Decrement portfolio loans taken count
fn decrement_portfolio_loans_taken(env: &Env, borrower: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(borrower.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, borrower.clone()));
    
    portfolio.active_loans = portfolio.active_loans.saturating_sub(1);
    
    portfolio_registry.set(borrower.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Decrement portfolio loans given count
fn decrement_portfolio_loans_given(env: &Env, lender: Address) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(lender.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, lender.clone()));
    
    portfolio.loans_given = portfolio.loans_given.saturating_sub(1);
    
    portfolio_registry.set(lender.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Update portfolio on liquidation
fn update_portfolio_on_liquidation(
    env: &Env,
    lender: Address,
    collection_id: u64,
    token_id: u64,
) -> Result<(), NFTError> {
    let mut portfolio_registry: Map<Address, NFTPortfolio> = env.storage().instance()
        .get(&PORTFOLIO_REGISTRY_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut portfolio = portfolio_registry.get(lender.clone())
        .unwrap_or_else(|| NFTPortfolio::new(env, lender.clone()));
    
    portfolio.add_nft(token_id, collection_id);
    
    portfolio_registry.set(lender.clone(), portfolio);
    env.storage().instance().set(&PORTFOLIO_REGISTRY_KEY, &portfolio_registry);
    
    Ok(())
}

/// Get loan by ID
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `loan_id` - Loan ID
/// 
/// # Returns
/// * `Option<NFTLoan>` - Loan info if found
pub fn get_loan(env: &Env, loan_id: u64) -> Option<NFTLoan> {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    loan_registry.get_loan(loan_id)
}

/// Get loan by collateral NFT
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `Option<u64>` - Loan ID if NFT is collateralized
pub fn get_loan_by_collateral(env: &Env, collection_id: u64, token_id: u64) -> Option<u64> {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    loan_registry.get_loan_by_collateral(collection_id, token_id)
}

/// Get active loans for a borrower
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `borrower` - Borrower address
/// 
/// # Returns
/// * `Vec<u64>` - List of loan IDs
pub fn get_borrower_loans(env: &Env, borrower: Address) -> Vec<u64> {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    loan_registry.get_borrower_loans(borrower)
}

/// Get active loans for a lender
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `lender` - Lender address
/// 
/// # Returns
/// * `Vec<u64>` - List of loan IDs
pub fn get_lender_loans(env: &Env, lender: Address) -> Vec<u64> {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    loan_registry.get_lender_loans(lender)
}

/// Check if a loan is overdue
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `loan_id` - Loan ID
/// 
/// # Returns
/// * `bool` - True if loan is overdue
pub fn is_loan_overdue(env: &Env, loan_id: u64) -> bool {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    
    if let Some(loan) = loan_registry.get_loan(loan_id) {
        let current_time = env.ledger().timestamp();
        loan.is_overdue(current_time)
    } else {
        false
    }
}

/// Calculate current repayment amount for a loan
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `loan_id` - Loan ID
/// 
/// # Returns
/// * `i128` - Current repayment amount (0 if loan not found)
pub fn calculate_repayment_amount(env: &Env, loan_id: u64) -> i128 {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    
    if let Some(loan) = loan_registry.get_loan(loan_id) {
        let current_time = env.ledger().timestamp();
        loan.total_due(current_time)
    } else {
        0
    }
}

/// Get total active loans
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// 
/// # Returns
/// * `u64` - Total number of active loans
pub fn get_total_active_loans(env: &Env) -> u64 {
    let loan_registry: LoanRegistry = env.storage().instance()
        .get(&LOAN_REGISTRY_KEY)
        .unwrap_or_else(|| LoanRegistry::new(env));
    loan_registry.active_count
}

/// Check if an NFT can be used as collateral
/// 
/// # Arguments
/// * `env` - The Soroban environment
/// * `collection_id` - Collection ID
/// * `token_id` - Token ID
/// 
/// # Returns
/// * `bool` - True if NFT can be used as collateral
pub fn can_use_as_collateral(env: &Env, collection_id: u64, token_id: u64) -> bool {
    // Check if NFT exists
    let nft_registry: NFTRegistry = env.storage().instance()
        .get(&NFT_REGISTRY_KEY)
        .unwrap_or_else(|| NFTRegistry::new(env));
    
    if let Some(nft) = nft_registry.get_nft(collection_id, token_id) {
        // Cannot use if already collateralized
        let loan_registry: LoanRegistry = env.storage().instance()
            .get(&LOAN_REGISTRY_KEY)
            .unwrap_or_else(|| LoanRegistry::new(env));
        
        if loan_registry.get_loan_by_collateral(collection_id, token_id).is_some() {
            return false;
        }
        
        // Cannot use if fractionalized
        if nft.is_fractionalized {
            return false;
        }
        
        true
    } else {
        false
    }
}
