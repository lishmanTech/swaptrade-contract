#[cfg(test)]
mod analytics_tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, Symbol, symbol_short, Vec};
    use crate::portfolio::{Asset, Portfolio};
    use crate::analytics::{PortfolioAnalytics, TimeWindow, PerformanceMetrics};

    #[test]
    fn test_get_performance_metrics_empty_portfolio() {
        let env = Env::default();
        let portfolio = Portfolio::new(&env);
        let user = Address::generate(&env);

        let metrics = PortfolioAnalytics::get_performance_metrics(
            &env,
            &portfolio,
            user,
            TimeWindow::Day7,
        );

        // Should return zero metrics for empty portfolio
        assert_eq!(metrics.sharpe_ratio, 0);
        assert_eq!(metrics.sortino_ratio, 0);
        assert_eq!(metrics.max_drawdown, 0);
        assert_eq!(metrics.volatility, 0);
        assert_eq!(metrics.total_return, 0);
        assert_eq!(metrics.win_rate, 0);
    }

    #[test]
    fn test_get_asset_allocation() {
        let env = Env::default();
        let mut portfolio = Portfolio::new(&env);
        let user = Address::generate(&env);

        // Mint some assets
        portfolio.mint(&env, Asset::XLM, user.clone(), 1000);
        portfolio.mint(&env, Asset::Custom(symbol_short!("USDCSIM")), user.clone(), 500);

        let allocation = PortfolioAnalytics::get_asset_allocation(&env, &portfolio, user);

        // Should have 2 assets
        assert_eq!(allocation.assets.len(), 2);

        // Check allocations (simplified - both should be non-zero)
        let xlm_allocation = allocation.assets.get(0).unwrap().1;
        let usdc_allocation = allocation.assets.get(1).unwrap().1;

        assert!(xlm_allocation > 0);
        assert!(usdc_allocation > 0);
        assert_eq!(xlm_allocation + usdc_allocation, 2_000_000_000); // 2.0 in fixed-point
    }

    #[test]
    fn test_get_benchmark_comparison() {
        let env = Env::default();
        let portfolio = Portfolio::new(&env);
        let user = Address::generate(&env);
        let benchmark_id = symbol_short!("SPX");

        let comparison = PortfolioAnalytics::get_benchmark_comparison(
            &env,
            &portfolio,
            user,
            benchmark_id,
            TimeWindow::Day30,
        );

        // Should return placeholder values
        assert_eq!(comparison.alpha, 0);
        assert_eq!(comparison.beta, 10_000_000); // 1.0 in fixed-point
        assert_eq!(comparison.tracking_error, 0);
        assert_eq!(comparison.information_ratio, 0);
    }

    #[test]
    fn test_get_period_returns() {
        let env = Env::default();
        let portfolio = Portfolio::new(&env);
        let user = Address::generate(&env);

        let returns = PortfolioAnalytics::get_period_returns(
            &env,
            &portfolio,
            user,
            1000000, // start timestamp
            2000000, // end timestamp
        );

        // Should return zero returns for empty portfolio
        assert_eq!(returns.time_weighted_return, 0);
        assert_eq!(returns.arithmetic_return, 0);
        assert_eq!(returns.geometric_return, 0);
        assert_eq!(returns.start_value, 0);
        assert_eq!(returns.end_value, 0);
        assert_eq!(returns.period_days, 0);
    }

    #[test]
    fn test_calculate_daily_returns() {
        let env = Env::default();
        let mut values = Vec::new(&env);
        values.push_back(100);
        values.push_back(110);
        values.push_back(95);

        let returns = PortfolioAnalytics::calculate_daily_returns(&values);

        assert_eq!(returns.len(), 2);
        // (110-100)/100 * FIXED_POINT = 10/100 * 10^7 = 10^6
        assert_eq!(returns.get(0).unwrap(), 1_000_000);
        // (95-110)/110 * FIXED_POINT ≈ -13.636... * 10^7 ≈ -136363636
        let expected_return_2 = ((95i128 - 110i128) * 10_000_000i128) / 110i128;
        assert_eq!(returns.get(1).unwrap(), expected_return_2);
    }

    #[test]
    fn test_calculate_volatility() {
        let env = Env::default();
        let mut returns = Vec::new(&env);
        returns.push_back(1_000_000);  // 0.1 in fixed-point
        returns.push_back(-500_000);  // -0.05 in fixed-point
        returns.push_back(2_000_000);  // 0.2 in fixed-point

        let volatility = PortfolioAnalytics::calculate_volatility(&returns);

        // Should be non-zero
        assert!(volatility > 0);
    }

    #[test]
    fn test_calculate_max_drawdown() {
        let env = Env::default();
        let mut values = Vec::new(&env);
        values.push_back(100);
        values.push_back(120);
        values.push_back(90);
        values.push_back(110);

        let max_drawdown = PortfolioAnalytics::calculate_max_drawdown(&values);

        // Max drawdown should be (120-90)/120 = 0.25 = 25% = 2_500_000 in fixed-point
        let expected_drawdown = (30u128 * 10_000_000u128) / 120u128;
        assert_eq!(max_drawdown, expected_drawdown);
    }

    #[test]
    fn test_calculate_win_rate() {
        let env = Env::default();
        let mut returns = Vec::new(&env);
        returns.push_back(1_000_000);   // win
        returns.push_back(-500_000);   // loss
        returns.push_back(2_000_000);   // win
        returns.push_back(-1_000_000);  // loss

        let win_rate = PortfolioAnalytics::calculate_win_rate(&returns);

        // 2 wins out of 4 = 50% = 5_000_000 in fixed-point
        assert_eq!(win_rate, 5_000_000);
    }

    #[test]
    fn test_calculate_diversification_score() {
        let env = Env::default();
        let mut assets: Vec<(Asset, u128)> = Vec::new(&env);

        // Equal allocation between 2 assets
        assets.push_back((Asset::XLM, 5_000_000)); // 0.5
        assets.push_back((Asset::Custom(symbol_short!("USDCSIM")), 5_000_000)); // 0.5

        let score = PortfolioAnalytics::calculate_diversification_score(&assets);

        // Should be high diversification score (close to 1.0)
        assert!(score > 8_000_000); // > 0.8
    }

    #[test]
    fn test_portfolio_record_daily_value() {
        let env = Env::default();
        let mut portfolio = Portfolio::new(&env);
        let user = Address::generate(&env);

        // Mint some assets
        portfolio.mint(&env, Asset::XLM, user.clone(), 1000);

        // Record portfolio value
        let timestamp = 1000000;
        portfolio.record_daily_portfolio_value(&env, user.clone(), timestamp);

        // Check that value was recorded
        let recorded_value = portfolio.get_last_portfolio_value(&env, user);
        assert_eq!(recorded_value, Some(1000));
    }

    #[test]
    fn test_portfolio_values_in_range() {
        let env = Env::default();
        let mut portfolio = Portfolio::new(&env);
        let user = Address::generate(&env);

        // Record values for multiple days
        portfolio.record_daily_portfolio_value(&env, user.clone(), 86400); // Day 1
        portfolio.record_daily_portfolio_value(&env, user.clone(), 172800); // Day 2
        portfolio.record_daily_portfolio_value(&env, user.clone(), 259200); // Day 3

        // Get values for range
        let values = portfolio.get_portfolio_values_in_range(&env, user, 1, 2);

        assert_eq!(values.len(), 2);
    }
}