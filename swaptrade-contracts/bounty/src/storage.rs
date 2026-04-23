use soroban_sdk::{contracttype, Address, BytesN};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Report(u32),          // report_id => report
    ReportCount,
    Reward(u32),          // report_id => reward amount
    Status(u32),          // report_id => status
    Reporter(u32),        // report_id => user
}

#[contracttype]
#[derive(Clone)]
pub enum ReportStatus {
    Submitted,
    UnderReview,
    Approved,
    Rejected,
    Paid,
}

#[contracttype]
#[derive(Clone)]
pub struct Report {
    pub hash: BytesN<32>,   // hash of off-chain report (IPFS or doc)
}