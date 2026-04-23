#![no_std]

mod storage;
mod admin;
mod waitlist;
mod approval;
mod events;

use soroban_sdk::{contract, contractimpl, Address, Env};
use storage::DataKey;

#[contract]
pub struct WaitlistContract;

#[contractimpl]
impl WaitlistContract {

    pub fn initialize(env: Env, admin: Address, max_users: u32) {
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MaxUsers, &max_users);
        env.storage().instance().set(&DataKey::ApprovedCount, &0u32);
    }

    pub fn join_waitlist(env: Env, user: Address) {
        waitlist::join(&env, user.clone());
        events::emit_join(&env, user);
    }

    pub fn approve_user(env: Env, admin: Address, user: Address) {
        admin::require_admin(&env, &admin);
        approval::approve(&env, user.clone());
        events::emit_approved(&env, user);
    }

    pub fn is_approved(env: Env, user: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Approved(user))
            .unwrap_or(false)
    }
}