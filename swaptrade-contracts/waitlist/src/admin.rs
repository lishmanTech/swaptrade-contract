use soroban_sdk::{Env, Address};
use crate::storage::DataKey;

pub fn require_admin(env: &Env, admin: &Address) {
    admin.require_auth();

    let stored_admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .unwrap();

    if &stored_admin != admin {
        panic!("Not authorized");
    }
}