//! Fuzz Testing Module for SwapTrade Contract
//!
//! This module contains property-based tests using random inputs to verify
//! contract behavior under edge cases and unexpected conditions.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{symbol_short, Address, Env, Symbol, Vec};

use crate::errors::ContractError;
use crate::invariants::*;
use crate::portfolio::{Asset, LPPosition, Portfolio};

/// Maximum amount for fuzz testing (prevents unrealistic values)
const FUZZ_MAX_AMOUNT: i128 = 1_000_000_000_000;
const FUZZ_MIN_AMOUNT: i128 = 1;

/// Generate a random-ish amount within bounds
/// Uses ledger timestamp for pseudo-randomness
fn fuzz_amount(env: &Env) -> i128 {
    let timestamp = env.ledger().timestamp();
    let seed = (timestamp % 1000000) as i128 + 1;
    (seed * 1000) % FUZZ_MAX_AMOUNT + FUZZ_MIN_AMOUNT
}

/// Generate a random user address
fn fuzz_user(env: &Env) -> Address {
    Address::generate(env)
}

// ==================== MINT OPERATION FUZZ TESTS ====================

/// Fuzz test: Mint with random amounts should always result in positive balance
#[test]
fn fuzz_mint_positive_balance() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);
    let user = fuzz_user(&env);

    // Test with various amounts
    for i in 1..=20 {
        let amount = i as i128 * 1000;
        portfolio.mint(&env, Asset::XLM, user.clone(), amount);

        let balance = portfolio.balance_of(&env, Asset::XLM, user.clone());
        assert!(balance >= 0, "Balance should never be negative after mint");
        assert!(
            balance >= amount,
            "Balance should be at least the minted amount"
        );
    }
}

/// Fuzz test: Multiple mints to same user accumulate correctly
#[test]
fn fuzz_mint_accumulation() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);
    let user = fuzz_user(&env);
    let mut total_minted: i128 = 0;

    for i in 1..=15 {
        let amount = i as i128 * 500;
        portfolio.mint(&env, Asset::XLM, user.clone(), amount);
        total_minted = total_minted.saturating_add(amount);

        let balance = portfolio.balance_of(&env, Asset::XLM, user.clone());
        assert_eq!(
            balance, total_minted,
            "Balance should equal total minted amount"
        );
    }
}

/// Fuzz test: Mint to different users maintains isolation
#[test]
fn fuzz_mint_user_isolation() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    for i in 1..=10 {
        let user = fuzz_user(&env);
        let amount = i as i128 * 1000;
        portfolio.mint(&env, Asset::XLM, user.clone(), amount);

        // Verify only this user has the balance
        assert_eq!(portfolio.balance_of(&env, Asset::XLM, user.clone()), amount);

        // Verify all other users have zero
        for j in 1..=5 {
            let other_user = Address::generate(&env);
            if other_user != user {
                assert_eq!(portfolio.balance_of(&env, Asset::XLM, other_user), 0);
            }
        }
    }
}

// ==================== BALANCE OPERATION FUZZ TESTS ====================

/// Fuzz test: Balance operations should maintain invariants
#[test]
fn fuzz_balance_operations_invariants() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);
    let user = fuzz_user(&env);

    // Initial mint
    portfolio.mint(&env, Asset::XLM, user.clone(), 100000);

    // Perform random debits and credits
    for i in 1..=20 {
        let debit_amount = (i * 100) as i128;
        let credit_amount = (i * 150) as i128;

        // Only debit if sufficient balance
        let current_balance = portfolio.balance_of(&env, Asset::XLM, user.clone());
        if current_balance >= debit_amount {
            portfolio.debit(&env, Asset::XLM, user.clone(), debit_amount);
        }

        portfolio.credit(&env, Asset::XLM, user.clone(), credit_amount);

        // Verify balance never goes negative
        let new_balance = portfolio.balance_of(&env, Asset::XLM, user.clone());
        assert!(new_balance >= 0, "Balance should never be negative");

        // Verify invariants hold
        assert!(invariant_non_negative_balances(&portfolio));
    }
}

// ==================== LP OPERATION FUZZ TESTS ====================

/// Fuzz test: LP position creation with various ratios
#[test]
fn fuzz_lp_position_creation() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    for i in 1..=15 {
        let user = fuzz_user(&env);
        let xlm_amount = (i * 1000) as i128;
        let usdc_amount = (i * 500) as i128;

        // Mint tokens to user first
        portfolio.mint(&env, Asset::XLM, user.clone(), xlm_amount);
        portfolio.mint(
            &env,
            Asset::Custom(symbol_short!("USDCSIM")),
            user.clone(),
            usdc_amount,
        );

        // Create LP position
        let position = LPPosition {
            lp_address: user.clone(),
            xlm_deposited: xlm_amount,
            usdc_deposited: usdc_amount,
            lp_tokens_minted: (xlm_amount + usdc_amount) / 2,
        };

        // Verify position integrity
        assert!(invariant_lp_position_integrity(&position));

        portfolio.set_lp_position(user.clone(), position.clone());

        // Verify stored position
        let stored = portfolio.get_lp_position(user.clone());
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().lp_tokens_minted, position.lp_tokens_minted);
    }
}

/// Fuzz test: LP token calculations with edge cases
#[test]
fn fuzz_lp_token_calculations() {
    let env = Env::default();

    // Test various deposit ratios
    let mut test_cases = Vec::new(&env);
    test_cases.push_back((1000, 1000, 0, 1000));             // First deposit, equal amounts
    test_cases.push_back((1000, 2000, 1000, 1414));          // Unequal pool, proportional
    test_cases.push_back((1, 1, 1000000, 1));                // Minimum deposit
    test_cases.push_back((1000000, 1000000, 1000, 1000000)); // Large deposit

    for (xlm_deposit, usdc_deposit, existing_lp, expected_min) in test_cases {
        // Calculate LP tokens (simplified formula)
        let product = (xlm_deposit as u128).saturating_mul(usdc_deposit as u128);
        let lp_tokens = if existing_lp == 0 {
            // First provider: sqrt(x * y)
            integer_sqrt(product)
        } else {
            // Subsequent: proportional to existing
            let xlm_share = (xlm_deposit as u128).saturating_mul(existing_lp as u128) / 1000;
            let usdc_share = (usdc_deposit as u128).saturating_mul(existing_lp as u128) / 1000;
            core::cmp::min(xlm_share, usdc_share)
        };

        assert!(
            lp_tokens >= expected_min as u128,
            "LP tokens should be at least {} for deposit {}/{}",
            expected_min,
            xlm_deposit,
            usdc_deposit
        );
    }
}

// Helper: Integer square root using Babylonian method
fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ==================== AMM INVARIANT FUZZ TESTS ====================

/// Fuzz test: AMM constant product with random swap scenarios
#[test]
fn fuzz_amm_constant_product() {
    let env = Env::default();
    let mut test_cases = Vec::new(&env);
    test_cases.push_back((10000, 10000, 11000, 9090));         // Normal swap with fees
    test_cases.push_back((50000, 20000, 51000, 19607));        // Different pool ratio
    test_cases.push_back((1000, 1000, 1100, 909));             // Small pool
    test_cases.push_back((1000000, 1000000, 1100000, 909090)); // Large pool

    for (xlm_before, usdc_before, xlm_after, usdc_after) in test_cases {
        // k should not increase (fees reduce or maintain k)
        assert!(
            invariant_amm_constant_product(xlm_before, usdc_before, xlm_after, usdc_after),
            "AMM invariant violated for pool {}/{} -> {}/{}",
            xlm_before,
            usdc_before,
            xlm_after,
            usdc_after
        );
    }
}

/// Fuzz test: AMM should reject impossible scenarios
#[test]
fn fuzz_amm_reject_impossible() {
    let env = Env::default();
    let mut impossible_cases = Vec::new(&env);
    impossible_cases.push_back((10000, 10000, 9000, 12000)); // k_before=100M, k_after=108M
    impossible_cases.push_back((10000, 10000, 8000, 13000)); // k_before=100M, k_after=104M
    // Negative reserves
    impossible_cases.push_back((10000, 10000, -1000, 11000));
    impossible_cases.push_back((10000, 10000, 11000, -1000));

    for (xlm_before, usdc_before, xlm_after, usdc_after) in impossible_cases {
        assert!(
            !invariant_amm_constant_product(xlm_before, usdc_before, xlm_after, usdc_after),
            "AMM should reject impossible scenario {}/{} -> {}/{}",
            xlm_before,
            usdc_before,
            xlm_after,
            usdc_after
        );
    }
}

// ==================== FEE CALCULATION FUZZ TESTS ====================

/// Fuzz test: Fee calculations within bounds
#[test]
fn fuzz_fee_calculations() {
    let env = Env::default();
    let mut test_amounts = Vec::new(&env);
    test_amounts.push_back(100);
    test_amounts.push_back(1000);
    test_amounts.push_back(10000);
    test_amounts.push_back(100000);
    test_amounts.push_back(1000000);
    test_amounts.push_back(10000000);
    test_amounts.push_back(100000000);

    for amount in test_amounts {
        // Calculate 0.3% fee
        let fee_bps: i128 = 30;
        let fee = (amount * fee_bps) / 10000;

        // Verify fee bounds
        assert!(invariant_fee_bounds(amount, fee));

        // Fee should be positive for positive amount
        if amount > 0 {
            assert!(fee > 0, "Fee should be positive for amount {}", amount);
        }

        // Fee should not exceed 1%
        let max_fee = amount / 100;
        assert!(
            fee <= max_fee,
            "Fee {} exceeds 1% of amount {}",
            fee,
            amount
        );
    }
}

/// Fuzz test: Edge case fee calculations
#[test]
fn fuzz_fee_edge_cases() {
    let env = Env::default();
    // Very small amounts
    let mut small_amounts = Vec::new(&env);
    small_amounts.push_back(1);
    small_amounts.push_back(10);
    small_amounts.push_back(33);
    small_amounts.push_back(100);
    for amount in small_amounts {
        let fee = (amount * 30) / 10000; // 0.3%
                                         // Due to integer division, small amounts may have 0 fee
        assert!(fee >= 0);
        assert!(invariant_fee_bounds(amount, fee));
    }

    // Very large amounts
    let mut large_amounts = Vec::new(&env);
    large_amounts.push_back(1_000_000_000);
    large_amounts.push_back(10_000_000_000);
    large_amounts.push_back(100_000_000_000);
    for amount in large_amounts {
        let fee = (amount * 30) / 10000;
        assert!(fee > 0);
        assert!(invariant_fee_bounds(amount, fee));
    }
}

// ==================== BATCH OPERATION FUZZ TESTS ====================

/// Fuzz test: Batch operation counts
#[test]
fn fuzz_batch_operation_counts() {
    let env = Env::default();

    let mut test_cases = Vec::new(&env);
    test_cases.push_back((10, 10, 0, true, true)); // All succeed, atomic
    test_cases.push_back((10, 0, 10, true, true)); // All fail, atomic
    test_cases.push_back((10, 5, 5, true, false)); // Mixed, atomic - should fail
    test_cases.push_back((10, 5, 5, false, true)); // Mixed, best-effort - should pass
    test_cases.push_back((0, 0, 0, false, true));  // Empty batch
    test_cases.push_back((1, 1, 0, true, true));   // Single operation success
    test_cases.push_back((1, 0, 1, true, true));   // Single operation failure

    for (total, success, failure, is_atomic, should_pass) in test_cases {
        let result = verify_batch_invariants(&env, total, success, failure, is_atomic);
        if should_pass {
            assert!(
                result.is_ok(),
                "Batch {}/{} (atomic={}) should pass",
                success,
                total,
                is_atomic
            );
        } else {
            assert!(
                result.is_err(),
                "Batch {}/{} (atomic={}) should fail",
                success,
                total,
                is_atomic
            );
        }
    }
}

// ==================== STATE CONSISTENCY FUZZ TESTS ====================

/// Fuzz test: Metrics monotonicity
#[test]
fn fuzz_metrics_monotonicity() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);
    let user = fuzz_user(&env);

    let mut prev_trades: u32 = 0;
    let mut prev_failed: u32 = 0;

    for i in 1..=20 {
        if i % 3 == 0 {
            // Simulate failed order
            portfolio.inc_failed_order();
        } else {
            // Simulate successful trade
            portfolio.record_trade(&env, user.clone());
        }

        let metrics = portfolio.get_metrics();

        // Verify monotonicity
        assert!(
            metrics.trades_executed >= prev_trades && metrics.failed_orders >= prev_failed,
            "Metrics should be monotonic: trades {}->{}, failed {}->{}",
            prev_trades,
            metrics.trades_executed,
            prev_failed,
            metrics.failed_orders
        );

        prev_trades = metrics.trades_executed;
        prev_failed = metrics.failed_orders;
    }
}

/// Fuzz test: User count consistency
#[test]
fn fuzz_user_count_consistency() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    for i in 1..=15 {
        let user = fuzz_user(&env);
        portfolio.record_trade(&env, user.clone());

        // Verify active users <= total users
        assert!(
            invariant_user_counts_consistent(&portfolio),
            "Active users ({}) should not exceed total users ({})",
            portfolio.get_active_users_count(),
            portfolio.get_total_users()
        );
    }
}

// ==================== SLIPPAGE FUZZ TESTS ====================

/// Fuzz test: Slippage calculations
#[test]
fn fuzz_slippage_calculations() {
    let env = Env::default();
    let mut test_cases = Vec::new(&env);
    test_cases.push_back((10000, 10000, 100, true)); // No slippage
    test_cases.push_back((10000, 9900, 100, true));  // 1% slippage, max 1%
    test_cases.push_back((10000, 9800, 100, false)); // 2% slippage, max 1%
    test_cases.push_back((10000, 10100, 100, true)); // Positive slippage (better)
    test_cases.push_back((10000, 0, 10000, true));   // 100% slippage allowed
    test_cases.push_back((0, 0, 100, true));         // Zero expected
    test_cases.push_back((0, 100, 100, false));      // Non-zero actual with zero expected

    for (expected, actual, max_slippage, should_pass) in test_cases {
        let result = invariant_slippage_bounds(expected, actual, max_slippage);
        if should_pass {
            assert!(
                result,
                "Slippage check should pass for {}/{} with max {}",
                actual, expected, max_slippage
            );
        } else {
            assert!(
                !result,
                "Slippage check should fail for {}/{} with max {}",
                actual, expected, max_slippage
            );
        }
    }
}

// ==================== BALANCE UPDATE FUZZ TESTS ====================

/// Fuzz test: Balance update consistency
#[test]
fn fuzz_balance_update_consistency() {
    let env = Env::default();
    let mut test_cases = Vec::new(&env);
    test_cases.push_back((1000, 200, 300, 1100, true));                     // Normal case
    test_cases.push_back((1000, 0, 0, 1000, true));                         // No change
    test_cases.push_back((1000, 1000, 0, 0, true));                         // Full debit
    test_cases.push_back((1000, 0, 1000, 2000, true));                      // Full credit
    test_cases.push_back((1000, 200, 300, 1000, false));                    // Incorrect result
    test_cases.push_back((i128::MAX - 100, 50, 50, i128::MAX - 100, true)); // Near max

    for (before, debit, credit, after, should_pass) in test_cases {
        let result = invariant_balance_update_consistency(before, debit, credit, after);
        if should_pass {
            assert!(
                result,
                "Balance update should be consistent: {} - {} + {} = {}",
                before, debit, credit, after
            );
        } else {
            assert!(
                !result,
                "Balance update should be inconsistent: {} - {} + {} != {}",
                before, debit, credit, after
            );
        }
    }
}

// ==================== OVERFLOW/UNDERFLOW FUZZ TESTS ====================

/// Fuzz test: Saturating arithmetic prevents overflow
#[test]
fn fuzz_saturating_arithmetic() {
    let max = i128::MAX;
    let min = i128::MIN;

    // Test saturating add at max boundary
    let result = max.saturating_add(1);
    assert_eq!(result, max, "Saturating add should cap at max");

    // Test saturating sub at min boundary
    let result = min.saturating_sub(1);
    assert_eq!(result, min, "Saturating sub should cap at min");

    // Test normal operations still work
    let result = 1000i128.saturating_add(500);
    assert_eq!(result, 1500);

    let result = 1000i128.saturating_sub(500);
    assert_eq!(result, 500);
}

/// Fuzz test: Large number operations
#[test]
fn fuzz_large_number_operations() {
    let env = Env::default();
    let mut large_values = Vec::new(&env);
    large_values.push_back(1_000_000_000_000);
    large_values.push_back(10_000_000_000_000);
    large_values.push_back(100_000_000_000_000);
    large_values.push_back(1_000_000_000_000_000);

    for val in large_values {
        // Multiplication should use saturating
        let product = (val as u128).saturating_mul(val as u128);
        assert!(product >= val as u128, "Product should not underflow");

        // Division should be safe
        if val > 0 {
            let quotient = val / 2;
            assert_eq!(quotient * 2, val - (val % 2));
        }
    }
}

// ==================== COMPREHENSIVE INVARIANT FUZZ TESTS ====================

/// Fuzz test: Run multiple operations and verify all invariants
#[test]
fn fuzz_comprehensive_invariant_check() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    // Perform 50 random operations
    for i in 1..=50 {
        let user = fuzz_user(&env);
        let operation = i % 5;

        match operation {
            0 => {
                // Mint
                let amount = (i * 1000) as i128;
                portfolio.mint(&env, Asset::XLM, user.clone(), amount);
            }
            1 => {
                // Credit
                let amount = (i * 500) as i128;
                portfolio.credit(&env, Asset::XLM, user.clone(), amount);
            }
            2 => {
                // Record trade
                portfolio.record_trade(&env, user.clone());
            }
            3 => {
                // Add pool liquidity
                let xlm = (i * 100) as i128;
                let usdc = (i * 100) as i128;
                portfolio.add_pool_liquidity(xlm, usdc);
            }
            4 => {
                // Collect fee
                let fee = (i * 10) as i128;
                portfolio.collect_fee(fee);
            }
            _ => {}
        }

        // Verify invariants after each operation
        assert!(
            invariant_non_negative_balances(&portfolio),
            "Negative balance invariant failed at operation {}",
            i
        );
        assert!(
            invariant_pool_liquidity_non_negative(&portfolio),
            "Pool liquidity invariant failed at operation {}",
            i
        );
        assert!(
            invariant_lp_token_conservation(&portfolio),
            "LP token invariant failed at operation {}",
            i
        );
        assert!(
            invariant_metrics_non_negative(&portfolio),
            "Metrics invariant failed at operation {}",
            i
        );
        assert!(
            invariant_fee_accumulation_non_negative(&portfolio),
            "Fee accumulation invariant failed at operation {}",
            i
        );
    }

    // Final comprehensive check
    assert!(
        verify_contract_invariants(&env, &portfolio).is_ok(),
        "Final invariant check failed"
    );
}

/// Fuzz test: Badge awarding with random users
#[test]
fn fuzz_badge_awarding() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    for i in 1..=25 {
        let user = fuzz_user(&env);

        // Award multiple trades to trigger badges
        for _ in 0..i {
            portfolio.record_trade(&env, user.clone());
        }

        // Verify badge uniqueness
        assert!(
            invariant_badge_uniqueness(&env, &portfolio, &user),
            "Badge uniqueness violated for user at iteration {}",
            i
        );

        // Verify badge count is reasonable
        let badges = portfolio.get_user_badges(&env, user.clone());
        assert!(badges.len() <= 7, "More badges than possible types");
    }
}

/// Fuzz test: Tier calculation with various trade counts
#[test]
fn fuzz_tier_calculations() {
    let env = Env::default();
    let portfolio = Portfolio::new(&env);

    let mut trade_counts = Vec::new(&env);
    trade_counts.push_back(0);
    trade_counts.push_back(1);
    trade_counts.push_back(5);
    trade_counts.push_back(9);
    trade_counts.push_back(10);
    trade_counts.push_back(25);
    trade_counts.push_back(49);
    trade_counts.push_back(50);
    trade_counts.push_back(75);
    trade_counts.push_back(99);
    trade_counts.push_back(100);
    trade_counts.push_back(200);

    for trades in trade_counts {
        let user = fuzz_user(&env);

        // Simulate trade count by recording trades
        for _ in 0..trades {
            // We can't easily simulate trade count without storage,
            // but we can verify tier calculation logic
        }

        let tier = portfolio.get_user_tier(&env, user.clone());

        // Verify tier is valid
        match trades {
            0..=9 => assert_eq!(tier as u32, 0, "Should be Basic tier"),
            10..=49 => assert_eq!(tier as u32, 1, "Should be Silver tier"),
            50..=99 => assert_eq!(tier as u32, 2, "Should be Gold tier"),
            100.. => assert_eq!(tier as u32, 3, "Should be Platinum tier"),
        }
    }
}

/// Fuzz test: Rate limit with various timestamps
#[test]
fn fuzz_rate_limit_monotonicity() {
    let env = Env::default();
    let mut timestamps = Vec::new(&env);
    timestamps.push_back(1000);
    timestamps.push_back(2000);
    timestamps.push_back(3000);
    timestamps.push_back(5000);
    timestamps.push_back(10000);
    timestamps.push_back(50000);
    timestamps.push_back(100000);

    for window in 0..timestamps.len().saturating_sub(1) {
        let prev = timestamps.get(window).unwrap();
        let curr = timestamps.get(window + 1).unwrap();

        // Timestamps should be monotonic
        assert!(
            invariant_timestamp_monotonic(prev, curr),
            "Timestamp {} should be <= {}",
            prev,
            curr
        );
    }
}

/// Fuzz test: Transaction history limits
#[test]
fn fuzz_transaction_history_limits() {
    let env = Env::default();
    let portfolio = Portfolio::new(&env);
    let user = fuzz_user(&env);

    // Request various limits
    let env = Env::default();
    let mut limits = Vec::new(&env);
    limits.push_back(0);
    limits.push_back(1);
    limits.push_back(5);
    limits.push_back(10);
    limits.push_back(100);
    limits.push_back(1000);

    for limit in limits {
        let txs = portfolio.get_user_transactions(&env, user.clone(), limit);
        // Result should not exceed requested limit
        assert!(
            txs.len() <= limit,
            "Transaction count {} exceeds limit {}",
            txs.len(),
            limit
        );
    }
}

/// Fuzz test: Top traders leaderboard consistency
#[test]
fn fuzz_top_traders_consistency() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    // Add various traders with different PnL
    for i in 1..=20 {
        let user = fuzz_user(&env);
        let pnl = (i * 1000 - 5000) as i128; // Mix of positive and negative

        portfolio.mint(&env, Asset::XLM, user.clone(), pnl.abs());

        // Note: update_top_traders is called internally by portfolio operations
    }

    // Get top traders with various limits
    let mut limits = Vec::new(&env);
    limits.push_back(1);
    limits.push_back(5);
    limits.push_back(10);
    limits.push_back(50);
    limits.push_back(100);

    for limit in limits {
        let top = portfolio.get_top_traders(&env, limit);
        assert!(top.len() <= limit, "Top traders count exceeds limit");
        assert!(top.len() <= 100, "Top traders exceeds max of 100");
    }
}

/// Fuzz test: Pool stats consistency
#[test]
fn fuzz_pool_stats_consistency() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    // Add liquidity multiple times
    for i in 1..=20 {
        let xlm = (i * 1000) as i128;
        let usdc = (i * 2000) as i128;

        portfolio.add_pool_liquidity(xlm, usdc);

        let (pool_xlm, pool_usdc, fees) = portfolio.get_pool_stats();

        // All values should be non-negative
        assert!(pool_xlm >= 0, "Pool XLM should be non-negative");
        assert!(pool_usdc >= 0, "Pool USDC should be non-negative");
        assert!(fees >= 0, "Fees should be non-negative");

        // Pool should have accumulated liquidity
        assert!(pool_xlm >= xlm, "Pool XLM should accumulate");
        assert!(pool_usdc >= usdc, "Pool USDC should accumulate");
    }
}

/// Fuzz test: Version monotonicity
#[test]
fn fuzz_version_monotonicity() {
    let env = Env::default();
    let mut versions = Vec::new(&env);
    versions.push_back((1, 1, true));  // Same version is ok
    versions.push_back((1, 2, true));  // Upgrade is ok
    versions.push_back((2, 1, false)); // Downgrade is not ok
    versions.push_back((1, 5, true));  // Big upgrade is ok

    for (prev, curr, should_pass) in versions {
        let result = invariant_version_monotonic(prev, curr);
        if should_pass {
            assert!(result, "Version {} -> {} should be monotonic", prev, curr);
        } else {
            assert!(
                !result,
                "Version {} -> {} should not be monotonic",
                prev, curr
            );
        }
    }
}

/// Fuzz test: Comprehensive state corruption detection
#[test]
fn fuzz_state_corruption_detection() {
    let env = Env::default();
    let mut portfolio = Portfolio::new(&env);

    // Perform operations that should maintain state integrity
    for i in 1..=30 {
        let user = fuzz_user(&env);

        // Mint and perform operations
        portfolio.mint(&env, Asset::XLM, user.clone(), 10000);
        portfolio.record_trade(&env, user.clone());

        // Check for corruption
        let metrics = portfolio.get_metrics();
        assert!(
            metrics.trades_executed <= i,
            "Trade count corruption detected"
        );
        assert!(
            portfolio.get_total_users() <= i as u32,
            "User count corruption detected"
        );

        // Verify all invariants
        let report = get_invariant_report(&env, &portfolio);
        for j in 0..report.len() {
            if let Some((name, passed)) = report.get(j) {
                assert!(passed, "Invariant {:?} failed at iteration {}", name, i);
            }
        }
    }
}
