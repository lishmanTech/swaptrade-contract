use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Waitlist(Address),
    Approved(Address),
    MaxUsers,
    ApprovedCount,
}