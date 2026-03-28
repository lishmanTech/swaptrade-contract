use soroban_sdk::{Env, Symbol};

pub fn set_limit(env: &Env, user: Symbol, limit: i32) {
    env.storage().set(&format!("limit_{}", user), &limit);
}

pub fn get_limit(env: &Env, user: Symbol) -> i32 {
    env.storage().get_unchecked(&format!("limit_{}", user)).unwrap_or(0)
}