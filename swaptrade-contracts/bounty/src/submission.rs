use soroban_sdk::{Env, Address, BytesN};
use crate::storage::{DataKey, Report, ReportStatus};

pub fn submit(env: &Env, user: Address, hash: BytesN<32>) -> u32 {
    user.require_auth();

    let mut id: u32 = env.storage().instance().get(&DataKey::ReportCount).unwrap_or(0);

    env.storage().instance().set(&DataKey::Report(id), &Report { hash });
    env.storage().instance().set(&DataKey::Status(id), &ReportStatus::Submitted);
    env.storage().instance().set(&DataKey::Reporter(id), &user);

    env.storage().instance().set(&DataKey::ReportCount, &(id + 1));

    id
}