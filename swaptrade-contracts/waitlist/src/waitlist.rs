use soroban_sdk::{Env, Address};
use crate::storage::DataKey;

pub fn join(env: &Env, user: Address) {
    user.require_auth();

    let key = DataKey::Waitlist(user.clone());

    if env.storage().instance().has(&key) {
        panic!("Already joined");
    }

    env.storage().instance().set(&key, &true);
}