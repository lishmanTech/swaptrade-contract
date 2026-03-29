use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone)]
pub struct BadgeEvent {
    pub user: Address,
    pub badge: crate::portfolio::Badge,
    pub timestamp: i64,
}

const EVENT_BUFFER_KEY: Symbol = Symbol::short("evt_buf");

pub struct Events;

impl Events {
    pub fn swap_executed(
        env: &Env,
        from_token: Symbol,
        to_token: Symbol,
        from_amount: i128,
        to_amount: i128,
        user: Address,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "SwapExecuted"), user, from_token, to_token),
            (from_amount, to_amount, timestamp),
        );
    }

    pub fn liquidity_added(
        env: &Env,
        xlm_amount: i128,
        usdc_amount: i128,
        lp_tokens_minted: i128,
        user: Address,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "LiquidityAdded"), user),
            (xlm_amount, usdc_amount, lp_tokens_minted, timestamp),
        );
    }

    pub fn liquidity_removed(
        env: &Env,
        xlm_amount: i128,
        usdc_amount: i128,
        lp_tokens_burned: i128,
        user: Address,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "LiquidityRemoved"), user),
            (xlm_amount, usdc_amount, lp_tokens_burned, timestamp),
        );
    }

    pub fn badge_awarded(env: &Env, user: Address, badge: crate::portfolio::Badge, timestamp: i64) {
        let mut buffer: Vec<BadgeEvent> = env
            .storage()
            .temporary()
            .get(&EVENT_BUFFER_KEY)
            .unwrap_or_else(|| Vec::new(env));
        buffer.push_back(BadgeEvent {
            user,
            badge,
            timestamp,
        });
        env.storage().temporary().set(&EVENT_BUFFER_KEY, &buffer);
    }

    pub fn flush_badge_events(env: &Env) {
        let buffer: Option<Vec<BadgeEvent>> = env.storage().temporary().get(&EVENT_BUFFER_KEY);
        if let Some(events) = buffer {
            if !events.is_empty() {
                env.events()
                    .publish((Symbol::new(env, "BadgesAwarded"),), events);
                env.storage().temporary().remove(&EVENT_BUFFER_KEY);
            }
        }
    }

    pub fn user_tier_changed(
        env: &Env,
        user: Address,
        old_tier: crate::tiers::UserTier,
        new_tier: crate::tiers::UserTier,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "UserTierChanged"), user),
            (old_tier, new_tier, timestamp),
        );
    }

    pub fn admin_paused(env: &Env, admin: Address, timestamp: i64) {
        env.events()
            .publish((Symbol::new(env, "AdminPaused"), admin), (timestamp,));
    }

    pub fn admin_resumed(env: &Env, admin: Address, timestamp: i64) {
        env.events()
            .publish((Symbol::new(env, "AdminResumed"), admin), (timestamp,));
    }
}

/// Emitted whenever an alert fires. Carries enough metadata for an
/// off-chain indexer to route a push notification or webhook call.
///
/// Topic  : ("AlertTriggered", owner_address, alert_id)
/// Payload: (alert_kind, notification_method, timestamp)
///
/// NOTE: This event is also emitted directly inside `alerts.rs` via
/// `emit_alert_triggered`. This stub documents the schema for the audit
/// trail and can be called from `events.rs` if you prefer to centralise
/// event emission in future.
pub fn alert_triggered(
    env: &Env,
    owner: Address,
    alert_id: u64,
    // Using Symbol here keeps the payload ABI-stable regardless of the
    // internal AlertKind enum layout across contract upgrades.
    kind_tag: Symbol,
    notification_method_tag: Symbol,
    timestamp: u64,
) {
    env.events().publish(
        (Symbol::new(env, "AlertTriggered"), owner, alert_id),
        (kind_tag, notification_method_tag, timestamp),
        );
    }

    /// Emitted when an alert is created so indexers can track the full
    /// lifecycle (create → trigger → cleanup) without polling storage.
    ///
    /// Topic  : ("AlertCreated", owner_address, alert_id)
    /// Payload: (kind_tag, expires_at)
    pub fn alert_created(
        env: &Env,
        owner: Address,
        alert_id: u64,
        kind_tag: Symbol,
        expires_at: u64,
    ) {
        env.events().publish(
            (Symbol::new(env, "AlertCreated"), owner, alert_id),
            (kind_tag, expires_at),
        );
    }

/// Emitted when performance metrics are calculated for a user.
/// Used for tracking portfolio performance analytics.
///
/// Topic  : ("PerformanceMetricsCalculated", user_address)
/// Payload: (time_window, sharpe_ratio, max_drawdown, timestamp)
pub fn performance_metrics_calculated(
        env: &Env,
        user: Address,
        time_window: crate::analytics::TimeWindow,
        sharpe_ratio: u128,
        max_drawdown: u128,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "PerformanceMetricsCalculated"), user),
            (time_window, sharpe_ratio, max_drawdown, timestamp),
        );
    }

/// Emitted when asset allocation analysis is completed.
/// Used for portfolio diversification tracking.
///
/// Topic  : ("AssetAllocationAnalyzed", user_address)
/// Payload: (total_assets, diversification_score, timestamp)
pub fn asset_allocation_analyzed(
        env: &Env,
        user: Address,
        total_assets: u32,
        diversification_score: u128,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "AssetAllocationAnalyzed"), user),
            (total_assets, diversification_score, timestamp),
        );
    }

/// Emitted when benchmark comparison is calculated.
/// Used for performance relative to market benchmarks.
///
/// Topic  : ("BenchmarkComparisonCalculated", user_address, benchmark_id)
/// Payload: (alpha, beta, timestamp)
pub fn benchmark_comparison_calculated(
        env: &Env,
        user: Address,
        benchmark_id: Symbol,
        alpha: i128,
        beta: u128,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "BenchmarkComparisonCalculated"), user, benchmark_id),
            (alpha, beta, timestamp),
        );
    }

/// Emitted when period returns are calculated.
/// Used for tracking returns over specific time periods.
///
/// Topic  : ("PeriodReturnsCalculated", user_address)
/// Payload: (start_timestamp, end_timestamp, time_weighted_return, timestamp)
pub fn period_returns_calculated(
        env: &Env,
        user: Address,
        start_timestamp: u64,
        end_timestamp: u64,
        time_weighted_return: i128,
        timestamp: i64,
    ) {
        env.events().publish(
            (Symbol::new(env, "PeriodReturnsCalculated"), user),
            (start_timestamp, end_timestamp, time_weighted_return, timestamp),
        );
    }
