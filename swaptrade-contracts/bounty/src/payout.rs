use soroban_sdk::{Env, Address};
use crate::storage::{DataKey, ReportStatus};

pub fn claim(env: &Env, user: Address, report_id: u32) -> i128 {
    user.require_auth();

    let reporter: Address = env.storage().instance().get(&DataKey::Reporter(report_id)).unwrap();

    if user != reporter {
        panic!("Not report owner");
    }

    let status: ReportStatus = env.storage().instance().get(&DataKey::Status(report_id)).unwrap();

    match status {
        ReportStatus::Approved => {
            let reward: i128 = env.storage().instance().get(&DataKey::Reward(report_id)).unwrap();

            env.storage().instance().set(&DataKey::Status(report_id), &ReportStatus::Paid);

            // ⚠️ integrate token transfer here in real system
            reward
        }
        _ => panic!("Not eligible"),
    }
}