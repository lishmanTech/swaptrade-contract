use soroban_sdk::{Env, Address};
use crate::storage::{DataKey, ReportStatus};

pub fn review(
    env: &Env,
    admin: Address,
    report_id: u32,
    approve: bool,
    reward: i128,
) {
    admin.require_auth();

    let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();

    if admin != stored_admin {
        panic!("Unauthorized");
    }

    if approve {
        env.storage().instance().set(&DataKey::Status(report_id), &ReportStatus::Approved);
        env.storage().instance().set(&DataKey::Reward(report_id), &reward);
    } else {
        env.storage().instance().set(&DataKey::Status(report_id), &ReportStatus::Rejected);
    }
}