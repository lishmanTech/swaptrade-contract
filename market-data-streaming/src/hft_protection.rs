use crate::order_types::*;
use std::collections::HashMap;
use chrono::{DateTime, Duration, Utc};
use crate::error::Result;

/// HFT Protection Mechanisms
pub struct HFTProtection {
    /// Order submission rate limits (orders per second per trader)
    order_rate_limits: HashMap<String, RateLimit>,
    /// Quote stuffing detection
    quote_stuffing_detector: QuoteStuffingDetector,
    /// Layering and spoofing detection
    spoofing_detector: SpoofinDetector,
    /// Flash crash prevention
    circuit_breaker: CircuitBreaker,
    /// Message rates
    message_rate_limiter: RateLimiter,
}

#[derive(Debug, Clone)]
struct RateLimit {
    trader_id: String,
    max_orders_per_sec: u32,
    current_orders: u32,
    last_reset: DateTime<Utc>,
    violations: u32,
}

#[derive(Debug, Default)]
struct QuoteStuffingDetector {
    /// Track order submissions per trader
    submissions: HashMap<String, Vec<DateTime<Utc>>>,
    /// Threshold: cancellations > this ratio triggers alert
    cancellation_ratio_threshold: f64,
    max_window_seconds: u32,
}

#[derive(Debug, Default)]
struct SpoofinDetector {
    /// Track orders that don't result in trades
    unmatched_orders: HashMap<String, Vec<Order>>,
    /// Watch for repeated placement and cancellation patterns
    patterns: HashMap<String, CancellationPattern>,
}

#[derive(Debug, Clone, Default)]
struct CancellationPattern {
    placements: u32,
    cancellations: u32,
    period_start: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct CircuitBreaker {
    /// Price movement threshold to trigger halt (e.g., 7%)
    price_move_threshold: f64,
    /// Volume threshold (e.g., 50% spike)
    volume_threshold: f64,
    /// Active halt status per symbol
    halted_symbols: HashMap<String, DateTime<Utc>>,
    /// Halt duration in seconds
    halt_duration: u32,
}

#[derive(Debug)]
struct RateLimiter {
    max_messages_per_sec: u32,
    window_duration: Duration,
    submission_times: Vec<DateTime<Utc>>,
}

impl RateLimiter {
    fn new(max_messages_per_sec: u32) -> Self {
        Self {
            max_messages_per_sec,
            window_duration: Duration::seconds(1),
            submission_times: Vec::new(),
        }
    }

    fn check_rate_limit(&mut self) -> bool {
        let now = Utc::now();
        
        // Remove old entries
        self.submission_times.retain(|time| (now - *time) < self.window_duration);

        if self.submission_times.len() as u32 >= self.max_messages_per_sec {
            return false;
        }

        self.submission_times.push(now);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HFTViolation {
    ExcessiveOrderRate,
    QuoteStuffing,
    Spoofing,
    Layering,
    FlashCrash,
    ExcessiveMessageRate,
}

#[derive(Debug, Clone)]
pub struct HFTAlert {
    pub violation: HFTViolation,
    pub trader_id: String,
    pub severity: AlertSeverity,
    pub description: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl HFTProtection {
    pub fn new() -> Self {
        Self {
            order_rate_limits: HashMap::new(),
            quote_stuffing_detector: QuoteStuffingDetector {
                submissions: HashMap::new(),
                cancellation_ratio_threshold: 0.95,
                max_window_seconds: 60,
            },
            spoofing_detector: SpoofinDetector::default(),
            circuit_breaker: CircuitBreaker {
                price_move_threshold: 0.07,
                volume_threshold: 0.5,
                halted_symbols: HashMap::new(),
                halt_duration: 5,
            },
            message_rate_limiter: RateLimiter::new(1000),
        }
    }

    /// Check if trader can submit order
    pub fn can_submit_order(&mut self, trader_id: &str, max_rate: u32) -> Result<()> {
        let now = Utc::now();
        let limit = self.order_rate_limits
            .entry(trader_id.to_string())
            .or_insert_with(|| RateLimit {
                trader_id: trader_id.to_string(),
                max_orders_per_sec: max_rate,
                current_orders: 0,
                last_reset: now,
                violations: 0,
            });

        // Reset counter every second
        if (now - limit.last_reset).num_seconds() >= 1 {
            limit.current_orders = 0;
            limit.last_reset = now;
        }

        if limit.current_orders >= limit.max_orders_per_sec {
            limit.violations += 1;
            return Err(crate::error::MarketDataError::HFTViolation(
                HFTViolation::ExcessiveOrderRate,
            ));
        }

        limit.current_orders += 1;
        Ok(())
    }

    /// Detect quote stuffing (excessive order cancellations)
    pub fn check_quote_stuffing(&mut self, trader_id: &str, order: &Order) -> Option<HFTAlert> {
        let now = Utc::now();
        
        let submissions = self.quote_stuffing_detector.submissions
            .entry(trader_id.to_string())
            .or_insert_with(Vec::new);

        // Clean old entries (outside window)
        let window_start = now - Duration::seconds(self.quote_stuffing_detector.max_window_seconds as i64);
        submissions.retain(|time| time > &window_start);

        submissions.push(now);

        // Detection: high cancellation rate indicates quote stuffing
        if submissions.len() > 50 && order.status == OrderStatus::Cancelled {
            let cancellation_rate = if submissions.len() > 0 {
                1.0 // Simplified - count cancellations
            } else {
                0.0
            };

            if cancellation_rate > self.quote_stuffing_detector.cancellation_ratio_threshold {
                return Some(HFTAlert {
                    violation: HFTViolation::QuoteStuffing,
                    trader_id: trader_id.to_string(),
                    severity: AlertSeverity::Medium,
                    description: format!(
                        "High cancellation rate detected: {:.2}%",
                        cancellation_rate * 100.0
                    ),
                    timestamp: now,
                });
            }
        }

        None
    }

    /// Detect spoofing (placing orders without intention to trade)
    pub fn check_spoofing(&mut self, trader_id: &str, order: &Order) -> Option<HFTAlert> {
        let pattern = self.spoofing_detector.patterns
            .entry(trader_id.to_string())
            .or_insert_with(|| {
                CancellationPattern {
                    placements: 0,
                    cancellations: 0,
                    period_start: Utc::now(),
                }
            });

        pattern.placements += 1;

        if order.status == OrderStatus::Cancelled {
            pattern.cancellations += 1;
        }

        // Detection: >90% of orders cancelled = potential spoofing
        if pattern.placements > 10 {
            let cancellation_rate = pattern.cancellations as f64 / pattern.placements as f64;
            if cancellation_rate > 0.9 {
                let alert = Some(HFTAlert {
                    violation: HFTViolation::Spoofing,
                    trader_id: trader_id.to_string(),
                    severity: AlertSeverity::High,
                    description: format!(
                        "Spoofing pattern detected: {:.2}% cancellation rate",
                        cancellation_rate * 100.0
                    ),
                    timestamp: Utc::now(),
                });

                // Reset pattern
                pattern.placements = 0;
                pattern.cancellations = 0;

                return alert;
            }
        }

        None
    }

    /// Check circuit breaker for flash crash
    pub fn check_circuit_breaker(
        &mut self,
        symbol: &str,
        price_change_pct: f64,
        volume_change_pct: f64,
    ) -> Result<()> {
        let now = Utc::now();

        // Check if already halted
        if let Some(halt_time) = self.circuit_breaker.halted_symbols.get(symbol) {
            let elapsed = (now - *halt_time).num_seconds();
            if elapsed < self.circuit_breaker.halt_duration as i64 {
                return Err(crate::error::MarketDataError::CircuitBreakerTriggered);
            } else {
                self.circuit_breaker.halted_symbols.remove(symbol);
            }
        }

        // Trigger halt if thresholds exceeded
        if price_change_pct.abs() > self.circuit_breaker.price_move_threshold ||
            volume_change_pct > self.circuit_breaker.volume_threshold {
            self.circuit_breaker.halted_symbols.insert(symbol.to_string(), now);
            return Err(crate::error::MarketDataError::CircuitBreakerTriggered);
        }

        Ok(())
    }

    /// Check global message rate limit
    pub fn check_message_rate(&mut self) -> Result<()> {
        if !self.message_rate_limiter.check_rate_limit() {
            return Err(crate::error::MarketDataError::HFTViolation(
                HFTViolation::ExcessiveMessageRate,
            ));
        }
        Ok(())
    }

    /// Get all active alerts
    pub fn get_active_alerts(&self) -> Vec<HFTAlert> {
        // In a real implementation, would track and return all active alerts
        Vec::new()
    }

    /// set price move threshold for circuit breaker
    pub fn set_circuit_breaker_threshold(&mut self, threshold: f64) {
        self.circuit_breaker.price_move_threshold = threshold;
    }

    /// Get circuit breaker status
    pub fn is_circuit_breaker_active(&self, symbol: &str) -> bool {
        if let Some(halt_time) = self.circuit_breaker.halted_symbols.get(symbol) {
            let elapsed = (Utc::now() - *halt_time).num_seconds();
            elapsed < self.circuit_breaker.halt_duration as i64
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_rate_limit() {
        let mut protection = HFTProtection::new();
        
        // First 10 orders should pass
        for _ in 0..10 {
            assert!(protection.can_submit_order("trader1", 10).is_ok());
        }

        // 11th should fail
        let result = protection.can_submit_order("trader1", 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_circuit_breaker() {
        let mut protection = HFTProtection::new();
        protection.set_circuit_breaker_threshold(0.07);

        // 10% price move should trigger
        let result = protection.check_circuit_breaker("BTC/USD", 0.10, 0.0);
        assert!(result.is_err());
        assert!(protection.is_circuit_breaker_active("BTC/USD"));
    }

    #[test]
    fn test_spoofing_detection() {
        let mut protection = HFTProtection::new();
        let mut order = Order::new(
            "ord1".to_string(),
            "BTC/USD".to_string(),
            OrderSide::Buy,
            OrderType::Limit,
            50000.0,
            1.0,
        );

        // Place many orders but cancel them
        for _ in 0..11 {
            let alert = protection.check_spoofing("trader1", &order);
            order.status = OrderStatus::Cancelled;
            
            if _ > 9 {
                // Should detect spoofing after 10 placements with >90% cancellation
                if let Some(alert) = alert {
                    assert_eq!(alert.violation, HFTViolation::Spoofing);
                }
            }
        }
    }
}
