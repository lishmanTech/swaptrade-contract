use soroban_sdk::{
    contracttype, symbol_short, Address, Env, Map, Symbol, Vec,
};

// Data Types

/// Direction a price alert should fire.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum PriceDirection {
    Above,
    Below,
}

/// What kind of portfolio event triggers the alert.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum PortfolioTrigger {
    /// Portfolio value has changed by `threshold` percent (basis points, e.g. 500 = 5%).
    ValueChangeBps,
    /// User's collateral ratio drops below `threshold` (in basis points).
    LiquidationRisk,
}

/// Market-level signal types.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum MarketSignal {
    TrendReversal,
    VolatilitySpike,
}

/// How the user wants to be notified (on-chain event vs. indexed webhook).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum NotificationMethod {
    Event,
    Webhook,
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum AlertKind {
    Price(Symbol, i128, PriceDirection),
    Portfolio(PortfolioTrigger, i128),
    Market(Symbol, MarketSignal),
}

/// A single alert record.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Alert {
    pub id: u64,
    pub owner: Address,
    pub kind: AlertKind,
    pub notification_method: NotificationMethod,
    pub expires_at: u64,
    pub active: bool,
    pub last_triggered_at: u64,
}

// Storage Keys

const ALERT_COUNTER_KEY: Symbol = symbol_short!("alrt_cnt");

const ALERT_MAP_KEY: Symbol = symbol_short!("alrt_map");

// Registry helpers

fn load_map(env: &Env) -> Map<Address, Vec<Alert>> {
    env.storage()
        .persistent()
        .get(&ALERT_MAP_KEY)
        .unwrap_or_else(|| Map::new(env))
}

fn save_map(env: &Env, map: &Map<Address, Vec<Alert>>) {
    env.storage().persistent().set(&ALERT_MAP_KEY, map);
}

fn next_id(env: &Env) -> u64 {
    let counter: u64 = env
        .storage()
        .persistent()
        .get(&ALERT_COUNTER_KEY)
        .unwrap_or(0u64);
    let new_counter = counter.saturating_add(1);
    env.storage()
        .persistent()
        .set(&ALERT_COUNTER_KEY, &new_counter);
    new_counter
}

// Public API

pub fn create_price_alert(
    env: &Env,
    owner: Address,
    token: Symbol,
    target_price: i128,
    direction: PriceDirection,
    expires_at: u64,
    notification_method: NotificationMethod,
) -> u64 {
    let id = next_id(env);
    let alert = Alert {
        id,
        owner: owner.clone(),
        kind: AlertKind::Price(token, target_price, direction),
        notification_method,
        expires_at,
        active: true,
        last_triggered_at: 0,
    };
    push_alert(env, owner, alert);
    id
}

pub fn create_portfolio_alert(
    env: &Env,
    owner: Address,
    trigger_type: PortfolioTrigger,
    threshold_bps: i128,
    expires_at: u64,
    notification_method: NotificationMethod,
) -> u64 {
    let id = next_id(env);
    let alert = Alert {
        id,
        owner: owner.clone(),
        kind: AlertKind::Portfolio(trigger_type, threshold_bps),
        notification_method,
        expires_at,
        active: true,
        last_triggered_at: 0,
    };
    push_alert(env, owner, alert);
    id
}

/// Create a market-level alert (trend reversal, volatility spike).
/// Returns the new `alert_id`.
pub fn create_market_alert(
    env: &Env,
    owner: Address,
    market_id: Symbol,
    signal_type: MarketSignal,
    expires_at: u64,
    notification_method: NotificationMethod,
) -> u64 {
    let id = next_id(env);
    let alert = Alert {
        id,
        owner: owner.clone(),
        kind: AlertKind::Market(market_id, signal_type),
        notification_method,
        expires_at,
        active: true,
        last_triggered_at: 0,
    };
    push_alert(env, owner, alert);
    id
}

/// Subscribe (activate) a set of existing alert IDs for a user.
/// Also updates the notification method on those alerts.
pub fn subscribe_alerts(
    env: &Env,
    user: Address,
    alert_ids: Vec<u64>,
    notification_method: NotificationMethod,
) {
    let mut map = load_map(env);
    let mut user_alerts: Vec<Alert> = map.get(user.clone()).unwrap_or_else(|| Vec::new(env));

    let len = user_alerts.len();
    for i in 0..len {
        let mut alert = user_alerts.get(i).unwrap();
        let alert_id = alert.id;
        let ids_len = alert_ids.len();
        for j in 0..ids_len {
            if alert_ids.get(j).unwrap() == alert_id {
                alert.active = true;
                alert.notification_method = notification_method.clone();
                break;
            }
        }
        user_alerts.set(i, alert);
    }

    map.set(user, user_alerts);
    save_map(env, &map);
}

/// Returns all active (non-expired) alerts for a user.
pub fn get_active_alerts(env: &Env, user: Address) -> Vec<Alert> {
    let now = env.ledger().timestamp();
    let map = load_map(env);
    let user_alerts: Vec<Alert> = map.get(user).unwrap_or_else(|| Vec::new(env));

    let mut active = Vec::new(env);
    let len = user_alerts.len();
    for i in 0..len {
        let alert = user_alerts.get(i).unwrap();
        let not_expired = alert.expires_at == 0 || alert.expires_at > now;
        if alert.active && not_expired {
            active.push_back(alert);
        }
    }
    active
}

// Trigger Checks (called from trading / LP operations)

/// Check all price alerts for `token` against `current_price`.
/// Fires any that match and emits the appropriate event.
pub fn check_price_alerts(env: &Env, token: &Symbol, current_price: i128) {
    let now = env.ledger().timestamp();
    let mut map = load_map(env);
    let keys = map.keys();
    let keys_len = keys.len();

    for k in 0..keys_len {
        let user = keys.get(k).unwrap();
        let mut user_alerts: Vec<Alert> = map.get(user.clone()).unwrap_or_else(|| Vec::new(env));
        let mut changed = false;

        let len = user_alerts.len();
        for i in 0..len {
            let mut alert = user_alerts.get(i).unwrap();
            if !alert.active {
                continue;
            }
            // Expire stale alerts
            if alert.expires_at != 0 && alert.expires_at <= now {
                alert.active = false;
                user_alerts.set(i, alert);
                changed = true;
                continue;
            }

            if let AlertKind::Price(ref alert_token, target_price, ref direction) = alert.kind.clone() {
                if alert_token == token {
                    let fired = match direction {
                        PriceDirection::Above => current_price >= target_price,
                        PriceDirection::Below => current_price <= target_price,
                    };
                    if fired {
                        alert.last_triggered_at = now;
                        // Deactivate one-shot style – keep persistent alerts active
                        if alert.expires_at != 0 {
                            alert.active = false;
                        }
                        user_alerts.set(i, alert.clone());
                        changed = true;
                        emit_alert_triggered(env, &alert, now);
                    }
                }
            }
        }

        if changed {
            map.set(user, user_alerts);
        }
    }

    save_map(env, &map);
}

/// Check all portfolio alerts for `user` against a current portfolio value and
/// the value recorded at alert creation time (passed in as `reference_value`).
pub fn check_portfolio_alerts(
    env: &Env,
    user: &Address,
    current_value: i128,
    reference_value: i128,
) {
    let now = env.ledger().timestamp();
    let mut map = load_map(env);
    let mut user_alerts: Vec<Alert> = map
        .get(user.clone())
        .unwrap_or_else(|| Vec::new(env));
    let mut changed = false;

    let len = user_alerts.len();
    for i in 0..len {
        let mut alert = user_alerts.get(i).unwrap();
        if !alert.active {
            continue;
        }
        if alert.expires_at != 0 && alert.expires_at <= now {
            alert.active = false;
            user_alerts.set(i, alert);
            changed = true;
            continue;
        }

        if let AlertKind::Portfolio(ref trigger_type, threshold_bps) = alert.kind.clone() {
            let fired = match trigger_type {
                PortfolioTrigger::ValueChangeBps => {
                    if reference_value == 0 {
                        false
                    } else {
                        let change_bps = ((current_value - reference_value).abs() * 10_000)
                            / reference_value;
                        change_bps >= threshold_bps
                    }
                }
                PortfolioTrigger::LiquidationRisk => {
                    // current_value here is treated as collateral ratio in bps
                    current_value <= threshold_bps
                }
            };

            if fired {
                alert.last_triggered_at = now;
                if alert.expires_at != 0 {
                    alert.active = false;
                }
                user_alerts.set(i, alert.clone());
                changed = true;
                emit_alert_triggered(env, &alert, now);
            }
        }
    }

    if changed {
        map.set(user.clone(), user_alerts);
        save_map(env, &map);
    }
}

/// Check market alerts for a given `market_id` and `signal_type`.
pub fn check_market_alerts(env: &Env, market_id: &Symbol, signal_type: &MarketSignal) {
    let now = env.ledger().timestamp();
    let mut map = load_map(env);

    let keys = map.keys();
    let keys_len = keys.len();

    for k in 0..keys_len {
        let user = keys.get(k).unwrap();
        let mut user_alerts: Vec<Alert> = map.get(user.clone()).unwrap_or_else(|| Vec::new(env));
        let mut changed = false;

        let len = user_alerts.len();
        for i in 0..len {
            let mut alert = user_alerts.get(i).unwrap();
            if !alert.active {
                continue;
            }
            if alert.expires_at != 0 && alert.expires_at <= now {
                alert.active = false;
                user_alerts.set(i, alert);
                changed = true;
                continue;
            }

            if let AlertKind::Market(ref alert_market, ref alert_signal) = alert.kind.clone() {
                if alert_market == market_id && alert_signal == signal_type {
                    alert.last_triggered_at = now;
                    if alert.expires_at != 0 {
                        alert.active = false;
                    }
                    user_alerts.set(i, alert.clone());
                    changed = true;
                    emit_alert_triggered(env, &alert, now);
                }
            }
        }

        if changed {
            map.set(user, user_alerts);
        }
    }

    save_map(env, &map);
}

// ─── Cleanup ─────────────────────────────────────────────────────────────────

/// Remove all expired / inactive alerts for a user to prevent accumulation.
pub fn cleanup_alerts(env: &Env, user: Address) {
    let now = env.ledger().timestamp();
    let mut map = load_map(env);
    let user_alerts: Vec<Alert> = map.get(user.clone()).unwrap_or_else(|| Vec::new(env));

    let mut retained = Vec::new(env);
    let len = user_alerts.len();
    for i in 0..len {
        let alert = user_alerts.get(i).unwrap();
        let not_expired = alert.expires_at == 0 || alert.expires_at > now;
        if alert.active && not_expired {
            retained.push_back(alert);
        }
    }

    map.set(user, retained);
    save_map(env, &map);
}

// Internal helpers

fn push_alert(env: &Env, owner: Address, alert: Alert) {
    let mut map = load_map(env);
    let mut user_alerts: Vec<Alert> = map.get(owner.clone()).unwrap_or_else(|| Vec::new(env));
    user_alerts.push_back(alert);
    map.set(owner, user_alerts);
    save_map(env, &map);
}

/// Emit a structured `AlertTriggered` event that any off-chain indexer or
/// webhook relay can subscribe to.
fn emit_alert_triggered(env: &Env, alert: &Alert, timestamp: u64) {
    // The topic contains the alert id and owner so indexers can filter cheaply.
    // The data payload carries the full alert kind for rich notification content.
    env.events().publish(
        (
            Symbol::new(env, "AlertTriggered"),
            alert.owner.clone(),
            alert.id,
        ),
        (alert.kind.clone(), alert.notification_method.clone(), timestamp),
    );
}