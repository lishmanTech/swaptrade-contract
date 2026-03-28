use soroban_sdk::{Env, Symbol, Map};

pub fn calculate_risk(env: &Env, user: Symbol) -> i32 {
    // Simplified VaR calculation example
    let positions: Map<Symbol, i32> = env.storage().get_unchecked(&user).unwrap_or_default();
    let mut risk = 0;
    for (_, value) in positions.iter() {
        risk += (*value * 10) / 100; // Example: 10% risk per position
    }
    risk
}