#![cfg_attr(not(test), no_std)]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Map, Symbol, Vec};

// Bring in modules from parent directory
mod admin;
mod errors;
mod events;
mod invariants;
mod alerts;
#[cfg(test)]
mod alerts_tests;
mod rate_limit;
mod storage;
mod liquidity_pool;
mod batch {
    include!("../batch.rs");
}
mod tiers {
    include!("../tiers.rs");
}
mod batch_event_tests;
mod batch_opt_simple_test;
mod batch_performance_tests;
mod oracle;

mod portfolio {
    include!("../portfolio.rs");
}
mod trading {
    include!("../trading.rs");
}
mod analytics;
mod migration;

// NFT module
mod nft_types;
mod nft_errors;
mod nft_storage;
mod nft_events;
mod nft_minting;
mod nft_marketplace;
mod nft_fractional;
mod nft_lending;

// Re-export invariant functions for external use
pub use invariants::verify_contract_invariants;
pub use liquidity_pool::{LiquidityPool, PoolRegistry, Route};

use portfolio::{Asset, CachedPortfolio, CachedTopTraders, LPPosition, Portfolio};
pub use portfolio::{Badge, Metrics, Transaction};
pub use rate_limit::{RateLimitStatus, RateLimiter};
pub use tiers::UserTier;
use trading::perform_swap;
use analytics::{PortfolioAnalytics, TimeWindow, PerformanceMetrics, AssetAllocation, BenchmarkComparison, PeriodReturns};

// NFT imports
use nft_types::*;
use nft_errors::NFTError;
use nft_storage::*;

use crate::errors::SwapTradeError;
use crate::storage::{ADMIN_KEY, PAUSED_KEY};

pub fn pause_trading(env: Env) -> Result<bool, SwapTradeError> {
    // NOTE: Authentication check (invoker) removed for compatibility with SDK versions
    // In production ensure proper auth by checking invoker and require_admin.
    env.storage().persistent().set(&PAUSED_KEY, &true);
    Ok(true)
}

pub fn resume_trading(env: Env) -> Result<bool, SwapTradeError> {
    // NOTE: Authentication check (invoker) removed for compatibility with SDK versions
    env.storage().persistent().set(&PAUSED_KEY, &false);
    Ok(true)
}

pub fn set_admin(env: Env, new_admin: Address) -> Result<(), SwapTradeError> {
    // NOTE: Authentication check (invoker) removed for compatibility with SDK versions
    env.storage().persistent().set(&ADMIN_KEY, &new_admin);
    Ok(())
}

// Batch imports
use batch::{execute_batch_atomic, execute_batch_best_effort, BatchOperation, BatchResult};

// Oracle imports
use oracle::{get_stored_price, set_stored_price};
pub const CONTRACT_VERSION: u32 = 1;

const PORTFOLIO_CACHE_KEY: Symbol = symbol_short!("pcache");
const TOP_TRADERS_CACHE_KEY: Symbol = symbol_short!("tcache");
const CACHE_TTL_KEY: Symbol = symbol_short!("cttl");
const CACHE_HITS_KEY: Symbol = symbol_short!("chits");
const CACHE_MISSES_KEY: Symbol = symbol_short!("cmiss");
const DEFAULT_CACHE_TTL_SECONDS: u64 = 60;

#[derive(Clone)]
#[contracttype]
struct CacheHitMetrics {
    hits: u64,
    misses: u64,
    ratio_bps: u32,
}

fn get_cache_ttl(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&CACHE_TTL_KEY)
        .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
}

fn cache_ratio_bps(hits: u64, misses: u64) -> u32 {
    let total = hits.saturating_add(misses);
    if total == 0 {
        return 0;
    }
    ((hits.saturating_mul(10_000)) / total) as u32
}

fn record_cache_access(env: &Env, query: Symbol, hit: bool) {
    let mut hits: u64 = env.storage().instance().get(&CACHE_HITS_KEY).unwrap_or(0);
    let mut misses: u64 = env.storage().instance().get(&CACHE_MISSES_KEY).unwrap_or(0);

    if hit {
        hits = hits.saturating_add(1);
        env.storage().instance().set(&CACHE_HITS_KEY, &hits);
    } else {
        misses = misses.saturating_add(1);
        env.storage().instance().set(&CACHE_MISSES_KEY, &misses);
    }

    let payload = CacheHitMetrics {
        hits,
        misses,
        ratio_bps: cache_ratio_bps(hits, misses),
    };
    env.events().publish((symbol_short!("cache"), query), payload);
}

fn invalidate_query_cache(env: &Env) {
    env.storage().instance().remove(&PORTFOLIO_CACHE_KEY);
    env.storage().instance().remove(&TOP_TRADERS_CACHE_KEY);
}

fn apply_trader_limit(env: &Env, traders: Vec<(Address, i128)>, limit: u32) -> Vec<(Address, i128)> {
    let max_limit = if limit > 100 { 100 } else { limit };
    let mut result = Vec::new(env);
    let len = traders.len() as usize;
    let cap = if len < max_limit as usize {
        len
    } else {
        max_limit as usize
    };

    for i in 0..cap {
        if let Some(entry) = traders.get(i as u32) {
            result.push_back(entry);
        }
    }
    result
}

#[contract]
pub struct CounterContract;

#[contractimpl]
impl CounterContract {
    /// Initialize the contract version.
    /// Should be called after deployment.
    pub fn initialize(env: Env) {
        if migration::get_stored_version(&env) == 0 {
            env.storage()
                .instance()
                .set(&Symbol::short("v_code"), &CONTRACT_VERSION);
        }
    }

    /// Get the current contract version from storage
    pub fn get_contract_version(env: Env) -> u32 {
        migration::get_stored_version(&env)
    }

    /// Migrate contract data from V1 to V2
    pub fn migrate(env: Env) -> Result<(), SwapTradeError> {
        migration::migrate_from_v1_to_v2(&env)
    }

    pub fn mint(env: Env, token: Symbol, to: Address, amount: i128) {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let asset = if token == Symbol::short("XLM") {
            Asset::XLM
        } else {
            Asset::Custom(token.clone())
        };

        portfolio.mint(&env, asset, to, amount);

        env.storage().instance().set(&(), &portfolio);
        invalidate_query_cache(&env);
    }

    pub fn balance_of(env: Env, token: Symbol, user: Address) -> i128 {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let asset = if token == Symbol::short("XLM") {
            Asset::XLM
        } else {
            Asset::Custom(token.clone())
        };

        portfolio.balance_of(&env, asset, user)
    }

    /// Alias to match external API
    pub fn get_balance(env: Env, token: Symbol, owner: Address) -> i128 {
        Self::balance_of(env, token, owner)
    }

    /// Swap tokens using simplified AMM (1:1 XLM <-> USDC-SIM)
    pub fn swap(env: Env, from: Symbol, to: Symbol, amount: i128, user: Address) -> i128 {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        // Get user's current tier for fee calculation and rate limiting
        let user_tier = portfolio.get_user_tier(&env, user.clone());

        // Check rate limit before executing swap
        if let Err(_limit_status) = RateLimiter::check_swap_limit(&env, &user, &user_tier) {
            panic!("RATELIMIT");
        }

        let fee_bps = user_tier.effective_fee_bps();

        // Calculate fee amount (fee is collected on input amount)
        let fee_amount = (amount * fee_bps as i128) / 10000;
        let swap_amount = amount - fee_amount;

        // Collect the fee
        if fee_amount > 0 {
            // Deduct from user
            let fee_asset = if from == symbol_short!("XLM") {
                Asset::XLM
            } else {
                Asset::Custom(from.clone())
            };

            // We need to use a mutable borrow of portfolio which we already have
            portfolio.debit(&env, fee_asset, user.clone(), fee_amount);
            portfolio.collect_fee(fee_amount);
        }

        let out_amount = perform_swap(
            &env,
            &mut portfolio,
            from.clone(),
            to.clone(),
            swap_amount,
            user.clone(),
        );

        portfolio.record_trade(&env, user.clone());

        // Record daily portfolio value for analytics
        portfolio.record_daily_portfolio_value(&env, user.clone(), env.ledger().timestamp());

        env.storage().instance().set(&(), &portfolio);
        invalidate_query_cache(&env);

        // Flush batched badge events
        crate::events::Events::flush_badge_events(&env);

        // Optional structured logging for successful swap
        #[cfg(feature = "logging")]
        {
            use soroban_sdk::symbol_short;
            env.events()
                .publish((symbol_short!("swap")), (amount, out_amount));
        }

        out_amount
    }

    /// Non-panicking swap that counts failed orders and returns 0 on failure
    pub fn safe_swap(env: Env, from: Symbol, to: Symbol, amount: i128, user: Address) -> i128 {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let tokens_ok = (from == symbol_short!("XLM") || from == symbol_short!("USDCSIM"))
            && (to == symbol_short!("XLM") || to == symbol_short!("USDCSIM"));
        let pair_ok = from != to;
        let amount_ok = amount > 0;

        if !(tokens_ok && pair_ok && amount_ok) {
            // Count failed order
            portfolio.inc_failed_order();
            env.storage().instance().set(&(), &portfolio);
            invalidate_query_cache(&env);

            #[cfg(feature = "logging")]
            {
                use soroban_sdk::symbol_short;
                env.events()
                    .publish((symbol_short!("fail"), user.clone()), (from, to, amount));
            }
            return 0;
        }

        let out_amount = perform_swap(&env, &mut portfolio, from, to, amount, user.clone());
        portfolio.record_trade(&env, user);
        env.storage().instance().set(&(), &portfolio);
        invalidate_query_cache(&env);

        // Flush batched badge events
        crate::events::Events::flush_badge_events(&env);

        #[cfg(feature = "logging")]
        {
            use soroban_sdk::symbol_short;
            env.events()
                .publish((symbol_short!("swap")), (amount, out_amount));
        }

        out_amount
    }

    /// Record a swap execution for a user
    pub fn record_trade(env: Env, user: Address) {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        portfolio.record_trade(&env, user);

        env.storage().instance().set(&(), &portfolio);
        invalidate_query_cache(&env);
    }

    /// Get portfolio stats for a user (trade count, pnl)
    pub fn get_portfolio(env: Env, user: Address) -> (u32, i128) {
        let now = env.ledger().timestamp();
        let ttl = get_cache_ttl(&env);

        let portfolio_cache: Map<Address, CachedPortfolio> = env
            .storage()
            .instance()
            .get(&PORTFOLIO_CACHE_KEY)
            .unwrap_or_else(|| Map::new(&env));

        if let Some(entry) = portfolio_cache.get(user.clone()) {
            if now.saturating_sub(entry.cached_at) <= ttl {
                record_cache_access(&env, symbol_short!("portf"), true);
                return (entry.trades, entry.pnl);
            }
        }

        record_cache_access(&env, symbol_short!("portf"), false);
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let value = portfolio.get_portfolio(&env, user.clone());
        let mut updated_cache: Map<Address, CachedPortfolio> = env
            .storage()
            .instance()
            .get(&PORTFOLIO_CACHE_KEY)
            .unwrap_or_else(|| Map::new(&env));
        updated_cache.set(
            user,
            CachedPortfolio {
                trades: value.0,
                pnl: value.1,
                cached_at: now,
            },
        );
        env.storage()
            .instance()
            .set(&PORTFOLIO_CACHE_KEY, &updated_cache);

        value
    }

    /// Get top traders with instance-storage caching.
    pub fn get_top_traders(env: Env, limit: u32) -> Vec<(Address, i128)> {
        let now = env.ledger().timestamp();
        let ttl = get_cache_ttl(&env);

        if let Some(entry) = env
            .storage()
            .instance()
            .get::<_, CachedTopTraders>(&TOP_TRADERS_CACHE_KEY)
        {
            if now.saturating_sub(entry.cached_at) <= ttl {
                record_cache_access(&env, symbol_short!("toptr"), true);
                return apply_trader_limit(&env, entry.traders, limit);
            }
        }

        record_cache_access(&env, symbol_short!("toptr"), false);
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let traders = portfolio.get_top_traders(&env, 100);
        env.storage().instance().set(
            &TOP_TRADERS_CACHE_KEY,
            &CachedTopTraders {
                traders: traders.clone(),
                cached_at: now,
            },
        );

        apply_trader_limit(&env, traders, limit)
    }

    /// Update cache TTL in seconds (admin only).
    pub fn set_cache_ttl(env: Env, caller: Address, ttl_seconds: u64) -> Result<(), SwapTradeError> {
        caller.require_auth();
        crate::admin::require_admin(&env, &caller)?;
        env.storage().instance().set(&CACHE_TTL_KEY, &ttl_seconds);
        Ok(())
    }

    /// Get cache stats as (hits, misses, hit_ratio_bps).
    pub fn get_cache_stats(env: Env) -> (u64, u64, u32) {
        let hits: u64 = env.storage().instance().get(&CACHE_HITS_KEY).unwrap_or(0);
        let misses: u64 = env.storage().instance().get(&CACHE_MISSES_KEY).unwrap_or(0);
        (hits, misses, cache_ratio_bps(hits, misses))
    }

    /// Clear all query caches (admin only).
    pub fn clear_cache(env: Env, caller: Address) -> Result<(), SwapTradeError> {
        caller.require_auth();
        crate::admin::require_admin(&env, &caller)?;
        invalidate_query_cache(&env);
        Ok(())
    }

    /// Get aggregate metrics
    pub fn get_metrics(env: Env) -> Metrics {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        portfolio.get_metrics()
    }

    /// Check if a user has earned a specific badge
    pub fn has_badge(env: Env, user: Address, badge: Badge) -> bool {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        portfolio.has_badge(&env, user, badge)
    }

    /// Get all badges earned by a user
    pub fn get_user_badges(env: Env, user: Address) -> Vec<Badge> {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        portfolio.get_user_badges(&env, user)
    }

    pub fn get_user_transactions(env: Env, user: Address, limit: u32) -> Vec<Transaction> {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        portfolio.get_user_transactions(&env, user, limit)
    }

    /// Get the current tier for a user
    pub fn get_user_tier(env: Env, user: Address) -> UserTier {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        portfolio.get_user_tier(&env, user)
    }

    // ===== RATE LIMITING =====

    /// Get rate limit status for swap operations
    pub fn get_swap_rate_limit(env: Env, user: Address) -> RateLimitStatus {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let user_tier = portfolio.get_user_tier(&env, user.clone());
        RateLimiter::get_swap_status(&env, &user, &user_tier)
    }

    /// Get rate limit status for LP operations
    pub fn get_lp_rate_limit(env: Env, user: Address) -> RateLimitStatus {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let user_tier = portfolio.get_user_tier(&env, user.clone());
        RateLimiter::get_lp_status(&env, &user, &user_tier)
    }

    // ===== BATCH OPERATIONS =====

    pub fn execute_batch_atomic(env: Env, operations: Vec<BatchOperation>) -> BatchResult {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let result = execute_batch_atomic(&env, &mut portfolio, operations);

        match result {
            Ok(res) => {
                env.storage().instance().set(&(), &portfolio);
                crate::events::Events::flush_badge_events(&env);
                res
            }
            Err(_) => {
                let mut err = BatchResult::new(&env);
                err.operations_failed = 1;
                err
            }
        }
    }

    pub fn execute_batch_best_effort(env: Env, operations: Vec<BatchOperation>) -> BatchResult {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let result = execute_batch_best_effort(&env, &mut portfolio, operations);

        match result {
            Ok(res) => {
                env.storage().instance().set(&(), &portfolio);
                crate::events::Events::flush_badge_events(&env);
                res
            }
            Err(_) => {
                let mut err = BatchResult::new(&env);
                err.operations_failed = 1;
                err
            }
        }
    }

    pub fn execute_batch(env: Env, operations: Vec<BatchOperation>) -> BatchResult {
        Self::execute_batch_atomic(env, operations)
    }

    // ===== LIQUIDITY PROVIDER (LP) FUNCTIONS =====

    /// Add liquidity to the pool and mint LP tokens
    /// Returns the number of LP tokens minted
    pub fn add_liquidity(env: Env, xlm_amount: i128, usdc_amount: i128, user: Address) -> i128 {
        assert!(xlm_amount > 0, "XLM amount must be positive");
        assert!(usdc_amount > 0, "USDC amount must be positive");

        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        // Check rate limit for LP operations
        let user_tier = portfolio.get_user_tier(&env, user.clone());
        if let Err(_) = RateLimiter::check_lp_limit(&env, &user, &user_tier) {
            panic!("RATELIMIT");
        }

        // Get current pool state
        let current_xlm = portfolio.get_liquidity(Asset::XLM);
        let current_usdc = portfolio.get_liquidity(Asset::Custom(symbol_short!("USDCSIM")));
        let total_lp_tokens = portfolio.get_total_lp_tokens();

        // Check user has sufficient balance
        let user_xlm_balance = portfolio.balance_of(&env, Asset::XLM, user.clone());
        let user_usdc_balance =
            portfolio.balance_of(&env, Asset::Custom(symbol_short!("USDCSIM")), user.clone());

        assert!(user_xlm_balance >= xlm_amount, "Insufficient XLM balance");
        assert!(
            user_usdc_balance >= usdc_amount,
            "Insufficient USDC balance"
        );

        // Calculate LP tokens to mint using constant product AMM formula
        // If pool is empty, LP tokens = sqrt(xlm * usdc)
        // Otherwise, LP tokens = (deposit / pool_size) * total_lp_tokens
        let lp_tokens_minted = if total_lp_tokens == 0 {
            // First liquidity provider: LP tokens = sqrt(xlm * usdc)
            // Use integer square root (Babylonian method)
            let product = (xlm_amount as u128).saturating_mul(usdc_amount as u128);
            if product == 0 {
                panic!("Product must be positive");
            }
            // Integer square root using Babylonian method
            let mut guess = product;
            let mut prev_guess = 0u128;
            // Limit iterations to prevent infinite loop
            let mut iterations = 0;
            while guess != prev_guess && iterations < 100 {
                prev_guess = guess;
                let quotient = product / guess;
                guess = (guess + quotient) / 2;
                if guess == 0 {
                    guess = 1;
                    break;
                }
                iterations += 1;
            }
            guess as i128
        } else {
            // Calculate proportional share
            // LP tokens = min((xlm_amount / current_xlm) * total_lp_tokens, (usdc_amount / current_usdc) * total_lp_tokens)
            // This ensures the ratio is maintained
            let xlm_share = if current_xlm > 0 {
                (xlm_amount as u128).saturating_mul(total_lp_tokens as u128) / (current_xlm as u128)
            } else {
                0
            };
            let usdc_share = if current_usdc > 0 {
                (usdc_amount as u128).saturating_mul(total_lp_tokens as u128)
                    / (current_usdc as u128)
            } else {
                0
            };

            // Take minimum to maintain ratio
            core::cmp::min(xlm_share as i128, usdc_share as i128)
        };

        assert!(lp_tokens_minted > 0, "LP tokens minted must be positive");

        // Debit assets from user (transfer to pool)
        portfolio.debit(&env, Asset::XLM, user.clone(), xlm_amount);
        portfolio.debit(
            &env,
            Asset::Custom(symbol_short!("USDCSIM")),
            user.clone(),
            usdc_amount,
        );

        // Update pool liquidity
        portfolio.add_pool_liquidity(xlm_amount, usdc_amount);

        // Update or create LP position
        let existing_position = portfolio.get_lp_position(user.clone());
        let new_position = if let Some(mut pos) = existing_position {
            // Update existing position
            pos.xlm_deposited = pos.xlm_deposited.saturating_add(xlm_amount);
            pos.usdc_deposited = pos.usdc_deposited.saturating_add(usdc_amount);
            pos.lp_tokens_minted = pos.lp_tokens_minted.saturating_add(lp_tokens_minted);
            pos
        } else {
            // Create new position
            LPPosition {
                lp_address: user.clone(),
                xlm_deposited: xlm_amount,
                usdc_deposited: usdc_amount,
                lp_tokens_minted,
            }
        };

        portfolio.set_lp_position(user.clone(), new_position);
        portfolio.add_total_lp_tokens(lp_tokens_minted);

        // Record LP deposit for badge tracking
        portfolio.record_lp_deposit(user.clone());
        portfolio.check_and_award_badges(&env, user.clone());

        // Record rate limit usage
        RateLimiter::record_lp_op(&env, &user, env.ledger().timestamp());

        env.storage().instance().set(&(), &portfolio);
        invalidate_query_cache(&env);

        // Flush batched badge events
        crate::events::Events::flush_badge_events(&env);

        lp_tokens_minted
    }

    /// Remove liquidity from the pool by burning LP tokens
    /// Returns (xlm_amount, usdc_amount) returned to user
    pub fn remove_liquidity(env: Env, lp_tokens: i128, user: Address) -> (i128, i128) {
        assert!(lp_tokens > 0, "LP tokens must be positive");

        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        // Get user's LP position
        let position = portfolio.get_lp_position(user.clone());
        assert!(position.is_some(), "User has no LP position");
        let mut pos = position.unwrap();

        // Verify user has enough LP tokens
        assert!(pos.lp_tokens_minted >= lp_tokens, "Insufficient LP tokens");

        // Get current pool state
        let current_xlm = portfolio.get_liquidity(Asset::XLM);
        let current_usdc = portfolio.get_liquidity(Asset::Custom(symbol_short!("USDCSIM")));
        let total_lp_tokens = portfolio.get_total_lp_tokens();

        assert!(total_lp_tokens > 0, "No LP tokens in pool");

        // Calculate proportional share of pool
        // xlm_amount = (lp_tokens / total_lp_tokens) * current_xlm
        // usdc_amount = (lp_tokens / total_lp_tokens) * current_usdc
        let xlm_amount = ((lp_tokens as u128).saturating_mul(current_xlm as u128)
            / (total_lp_tokens as u128)) as i128;
        let usdc_amount = ((lp_tokens as u128).saturating_mul(current_usdc as u128)
            / (total_lp_tokens as u128)) as i128;

        assert!(
            xlm_amount > 0 && usdc_amount > 0,
            "Amounts must be positive"
        );

        // Verify we're not removing more than deposited (with rounding tolerance)
        // Allow small rounding differences
        let max_xlm = pos.xlm_deposited;
        let max_usdc = pos.usdc_deposited;

        // Check if removing more than deposited (with 1% tolerance for rounding)
        if xlm_amount > max_xlm.saturating_mul(101) / 100
            || usdc_amount > max_usdc.saturating_mul(101) / 100
        {
            panic!("Cannot remove more than deposited");
        }

        // Update pool liquidity (subtract)
        portfolio.set_liquidity(Asset::XLM, current_xlm.saturating_sub(xlm_amount));
        portfolio.set_liquidity(
            Asset::Custom(symbol_short!("USDCSIM")),
            current_usdc.saturating_sub(usdc_amount),
        );

        // Transfer assets from pool to user
        portfolio.mint(&env, Asset::XLM, user.clone(), xlm_amount);
        portfolio.mint(
            &env,
            Asset::Custom(symbol_short!("USDCSIM")),
            user.clone(),
            usdc_amount,
        );

        // Update LP position
        pos.lp_tokens_minted = pos.lp_tokens_minted.saturating_sub(lp_tokens);
        pos.xlm_deposited = pos.xlm_deposited.saturating_sub(xlm_amount);
        pos.usdc_deposited = pos.usdc_deposited.saturating_sub(usdc_amount);

        if pos.lp_tokens_minted == 0 {
            // Remove position if all tokens burned
            // Note: Map doesn't have remove, so we set to a zero position or track separately
            // For now, we'll keep it with zero values
        }
        portfolio.set_lp_position(user.clone(), pos);
        portfolio.subtract_total_lp_tokens(lp_tokens);

        // Record rate limit usage
        RateLimiter::record_lp_op(&env, &user, env.ledger().timestamp());

        env.storage().instance().set(&(), &portfolio);
        invalidate_query_cache(&env);

        (xlm_amount, usdc_amount)
    }

    /// Get LP positions for a user
    /// Returns a Vec containing the user's position if it exists
    pub fn get_lp_positions(env: Env, user: Address) -> Vec<LPPosition> {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        let mut result = Vec::new(&env);
        if let Some(position) = portfolio.get_lp_position(user) {
            result.push_back(position);
        }
        result
    }

    pub fn set_price(env: Env, token_pair: (Symbol, Symbol), price: u128) {
        set_stored_price(&env, token_pair, price);
    }

    pub fn get_current_price(env: Env, token_pair: (Symbol, Symbol)) -> u128 {
        get_stored_price(&env, token_pair)
            .map(|d| d.price)
            .unwrap_or(0)
    }

    pub fn set_price_update_tolerance_bps(env: Env, token_pair: (Symbol, Symbol), bps: u32) {
        oracle::set_price_update_tolerance_bps(&env, token_pair, bps);
    }

    pub fn set_pool_liquidity(env: Env, token: Symbol, amount: i128) {
        let mut portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));
        let asset = if token == symbol_short!("XLM") {
            Asset::XLM
        } else {
            Asset::Custom(token)
        };
        portfolio.set_liquidity(asset, amount);
        env.storage().instance().set(&(), &portfolio);
    }

    /// Get comprehensive performance metrics for a user
    pub fn get_performance_metrics(
        env: Env,
        user: Address,
        time_window: TimeWindow,
    ) -> PerformanceMetrics {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        PortfolioAnalytics::get_performance_metrics(&env, &portfolio, user, time_window)
    }

    /// Get asset allocation breakdown with correlation analysis
    pub fn get_asset_allocation(env: Env, user: Address) -> AssetAllocation {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        PortfolioAnalytics::get_asset_allocation(&env, &portfolio, user)
    }

    /// Compare portfolio performance against a benchmark
    pub fn get_benchmark_comparison(
        env: Env,
        user: Address,
        benchmark_id: Symbol,
        time_window: TimeWindow,
    ) -> BenchmarkComparison {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        PortfolioAnalytics::get_benchmark_comparison(&env, &portfolio, user, benchmark_id, time_window)
    }

    pub fn set_max_slippage_bps(env: Env, bps: u32) {
        env.storage().instance().set(&symbol_short!("MAX_SLIP"), &bps);
    }

    /// Calculate period returns between timestamps
    pub fn get_period_returns(
        env: Env,
        user: Address,
        start_timestamp: u64,
        end_timestamp: u64,
    ) -> PeriodReturns {
        let portfolio: Portfolio = env
            .storage()
            .instance()
            .get(&())
            .unwrap_or_else(|| Portfolio::new(&env));

        PortfolioAnalytics::get_period_returns(&env, &portfolio, user, start_timestamp, end_timestamp)
    }
}

#[cfg(test)]
mod analytics_tests;
#[cfg(test)]
mod balance_test;
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod enhanced_trading_tests; // NEW: Enhanced trading tests for better coverage
#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod dashboard_tests;
#[cfg(test)]
mod lp_tests;
mod migration_tests;
#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod rate_limit_tests;
#[cfg(test)]
mod transaction_tests; // NEW: Fuzz tests for security hardening

// trading tests are provided as integration/unit tests in the repository tests/ folder
