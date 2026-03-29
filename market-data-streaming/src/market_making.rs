use crate::order_types::*;
use crate::order_book_depth::OrderBookDepth;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use crate::error::Result;

/// Market making algorithm variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketMakingStrategy {
    SimpleSpread,
    OptimalSpread,
    PriceImprovement,
    VolatilityAdapted,
    InventoryAware,
}

/// Market maker configuration
#[derive(Debug, Clone)]
pub struct MarketMakerConfig {
    pub strategy: MarketMakingStrategy,
    pub base_spread_bps: f64,      // Basis points
    pub position_limit: f64,        // Max inventory
    pub max_order_size: f64,
    pub min_order_size: f64,
    pub inventory_target: f64,
    pub volatility_multiplier: f64,
}

/// Market maker state and orders
pub struct MarketMaker {
    pub trader_id: String,
    pub config: MarketMakerConfig,
    pub current_inventory: f64,
    pub active_orders: HashMap<String, Order>,
    pub pnl: f64,
    pub stats: MarketMakerStats,
}

#[derive(Debug, Clone, Default)]
pub struct MarketMakerStats {
    pub trades_count: u64,
    pub total_filled: f64,
    pub maker_rebates: f64,
    pub roi: f64,
    pub sharpe_ratio: f64,
    pub avg_execution_price: f64,
}

/// Price level with spread information
#[derive(Debug, Clone)]
pub struct SpreadLevel {
    pub price: f64,
    pub bid_price: f64,
    pub ask_price: f64,
    pub spread: f64,
    pub bid_quantity: f64,
    pub ask_quantity: f64,
}

impl MarketMaker {
    pub fn new(trader_id: String, config: MarketMakerConfig) -> Self {
        Self {
            trader_id,
            config,
            current_inventory: 0.0,
            active_orders: HashMap::new(),
            pnl: 0.0,
            stats: MarketMakerStats::default(),
        }
    }

    /// Generate market-making quotes based on strategy
    pub fn generate_quotes(
        &self,
        market_price: f64,
        book_depth: &OrderBookDepth,
    ) -> Result<(SpreadLevel, Vec<Order>)> {
        let spread_level = match self.config.strategy {
            MarketMakingStrategy::SimpleSpread => {
                self.simple_spread_quotes(market_price)
            }
            MarketMakingStrategy::OptimalSpread => {
                self.optimal_spread_quotes(market_price, book_depth)
            }
            MarketMakingStrategy::PriceImprovement => {
                self.price_improvement_quotes(market_price, book_depth)
            }
            MarketMakingStrategy::VolatilityAdapted => {
                self.volatility_adapted_quotes(market_price)
            }
            MarketMakingStrategy::InventoryAware => {
                self.inventory_aware_quotes(market_price)
            }
        };

        let orders = self.create_orders_from_spread(&spread_level)?;
        Ok((spread_level, orders))
    }

    /// Simple spread: fixed spread around market price
    fn simple_spread_quotes(&self, market_price: f64) -> SpreadLevel {
        let spread_amount = market_price * self.config.base_spread_bps / 10000.0;
        
        SpreadLevel {
            price: market_price,
            bid_price: market_price - spread_amount / 2.0,
            ask_price: market_price + spread_amount / 2.0,
            spread: spread_amount,
            bid_quantity: self.config.max_order_size,
            ask_quantity: self.config.max_order_size,
        }
    }

    /// Optimal spread: dynamic based on order book depth
    fn optimal_spread_quotes(&self, market_price: f64, book_depth: &OrderBookDepth) -> SpreadLevel {
        let bid_vol = book_depth.get_side_volume(OrderSide::Buy);
        let ask_vol = book_depth.get_side_volume(OrderSide::Sell);
        
        // Higher spread when inventory imbalanced
        let imbalance_ratio = if bid_vol + ask_vol > 0.0 {
            (bid_vol - ask_vol).abs() / (bid_vol + ask_vol)
        } else {
            0.0
        };

        let adjusted_spread = self.config.base_spread_bps * (1.0 + imbalance_ratio);
        let spread_amount = market_price * adjusted_spread / 10000.0;

        SpreadLevel {
            price: market_price,
            bid_price: market_price - spread_amount / 2.0,
            ask_price: market_price + spread_amount / 2.0,
            spread: spread_amount,
            bid_quantity: self.config.max_order_size * (1.0 - imbalance_ratio).min(1.0),
            ask_quantity: self.config.max_order_size * (1.0 - imbalance_ratio).min(1.0),
        }
    }

    /// Price improvement: tighten spread to compete
    fn price_improvement_quotes(&self, market_price: f64, book_depth: &OrderBookDepth) -> SpreadLevel {
        let current_spread = book_depth.get_spread().unwrap_or(market_price * 0.0001);
        let improved_spread = current_spread * 0.5; // Quote half the current spread

        SpreadLevel {
            price: market_price,
            bid_price: market_price - improved_spread / 2.0,
            ask_price: market_price + improved_spread / 2.0,
            spread: improved_spread,
            bid_quantity: self.config.max_order_size * 0.5,
            ask_quantity: self.config.max_order_size * 0.5,
        }
    }

    /// Volatility-adapted: wider spread in high volatility
    fn volatility_adapted_quotes(&self, market_price: f64) -> SpreadLevel {
        let volatility_adjustment = 1.0 + (self.config.volatility_multiplier * 0.1); // Simplified
        let spread_amount = market_price * self.config.base_spread_bps * volatility_adjustment / 10000.0;

        SpreadLevel {
            price: market_price,
            bid_price: market_price - spread_amount / 2.0,
            ask_price: market_price + spread_amount / 2.0,
            spread: spread_amount,
            bid_quantity: self.config.max_order_size / volatility_adjustment,
            ask_quantity: self.config.max_order_size / volatility_adjustment,
        }
    }

    /// Inventory-aware: adjust quotes to manage inventory
    fn inventory_aware_quotes(&self, market_price: f64) -> SpreadLevel {
        let inventory_excess = self.current_inventory - self.config.inventory_target;
        let inventory_adjustment = (inventory_excess / self.config.position_limit).clamp(-0.1, 0.1);

        let base_spread = market_price * self.config.base_spread_bps / 10000.0;
        
        // Increase ask if over-inventoried (to sell more)
        // Decrease bid if over-inventoried (to avoid buying more)
        let bid_price = market_price - base_spread / 2.0 - inventory_adjustment * market_price * 0.001;
        let ask_price = market_price + base_spread / 2.0 + inventory_adjustment * market_price * 0.001;

        let qty_adjustment = 1.0 - inventory_excess.abs() / self.config.position_limit;
        
        SpreadLevel {
            price: market_price,
            bid_price,
            ask_price,
            spread: ask_price - bid_price,
            bid_quantity: self.config.max_order_size * qty_adjustment.max(0.1),
            ask_quantity: self.config.max_order_size * qty_adjustment.max(0.1),
        }
    }

    /// Create limit orders from spread quotes
    fn create_orders_from_spread(&self, spread_level: &SpreadLevel) -> Result<Vec<Order>> {
        let mut orders = Vec::new();

        // Buy order
        let mut buy_order = Order::new(
            uuid::Uuid::new_v4().to_string(),
            "BTC/USD".to_string(),
            OrderSide::Buy,
            OrderType::Limit,
            spread_level.bid_price,
            spread_level.bid_quantity,
        );
        buy_order.client_id = Some(format!("{}_buy", self.trader_id));
        orders.push(buy_order);

        // Sell order
        let mut sell_order = Order::new(
            uuid::Uuid::new_v4().to_string(),
            "BTC/USD".to_string(),
            OrderSide::Sell,
            OrderType::Limit,
            spread_level.ask_price,
            spread_level.ask_quantity,
        );
        sell_order.client_id = Some(format!("{}_sell", self.trader_id));
        orders.push(sell_order);

        Ok(orders)
    }

    /// Update inventory after trade
    pub fn update_inventory(&mut self, side: OrderSide, quantity: f64) {
        match side {
            OrderSide::Buy => self.current_inventory += quantity,
            OrderSide::Sell => self.current_inventory -= quantity,
        }
    }

    /// Calculate realized PnL from trade
    pub fn record_trade(
        &mut self,
        price: f64,
        quantity: f64,
        side: OrderSide,
        fee: f64,
    ) {
        let cost = price * quantity + fee;
        match side {
            OrderSide::Buy => {
                self.pnl -= cost;
            }
            OrderSide::Sell => {
                self.pnl += cost;
            }
        }

        self.stats.trades_count += 1;
        self.stats.total_filled += quantity;
        self.stats.maker_rebates += fee;
    }

    /// Check if can place order based on limits
    pub fn can_place_order(&self, side: OrderSide, quantity: f64, price: f64) -> bool {
        // Check position limit
        let projected_inventory = match side {
            OrderSide::Buy => self.current_inventory + quantity,
            OrderSide::Sell => self.current_inventory - quantity,
        };

        if projected_inventory.abs() > self.config.position_limit {
            return false;
        }

        // Check order size limits
        if quantity < self.config.min_order_size || quantity > self.config.max_order_size {
            return false;
        }

        true
    }

    /// Get current market-making efficiency
    pub fn get_efficiency(&self) -> f64 {
        if self.stats.total_filled > 0.0 {
            self.pnl / self.stats.total_filled
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_spread() {
        let config = MarketMakerConfig {
            strategy: MarketMakingStrategy::SimpleSpread,
            base_spread_bps: 10.0,
            position_limit: 10.0,
            max_order_size: 1.0,
            min_order_size: 0.01,
            inventory_target: 0.0,
            volatility_multiplier: 1.0,
        };

        let mm = MarketMaker::new("mm1".to_string(), config);
        let spread = mm.simple_spread_quotes(50000.0);

        assert!(spread.bid_price < spread.price);
        assert!(spread.ask_price > spread.price);
        assert!((spread.spread - 50.0).abs() < 1.0); // ~50 bps spread
    }

    #[test]
    fn test_inventory_tracking() {
        let config = MarketMakerConfig {
            strategy: MarketMakingStrategy::SimpleSpread,
            base_spread_bps: 10.0,
            position_limit: 10.0,
            max_order_size: 1.0,
            min_order_size: 0.01,
            inventory_target: 0.0,
            volatility_multiplier: 1.0,
        };

        let mut mm = MarketMaker::new("mm1".to_string(), config);
        
        mm.update_inventory(OrderSide::Buy, 1.5);
        assert_eq!(mm.current_inventory, 1.5);

        mm.update_inventory(OrderSide::Sell, 0.5);
        assert_eq!(mm.current_inventory, 1.0);
    }
}
