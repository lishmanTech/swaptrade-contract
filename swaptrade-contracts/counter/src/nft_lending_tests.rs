#![cfg(test)]

use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, String};

use crate::nft::{
    create_collection, mint_nft, set_nft_valuation, get_nft_valuation, get_collection_floor_price,
};
use crate::nft_lending::{
    request_loan, fund_loan, calculate_loan_ltv, is_loan_undercollateralized,
    monitor_and_queue_liquidations, process_liquidation_queue, place_liquidation_bid,
    get_loan, get_collateral_value
};
use crate::nft_types::NFTStandard;
use crate::nft_errors::NFTError;

fn setup_env() -> (Env, Address, Address) {
    let env = Env::default();
    let borrower = Address::generate(&env);
    let lender = Address::generate(&env);
    (env, borrower, lender)
}

#[test]
fn test_undercollateralized_loan_is_queued_and_partial_liquidated() {
    let (env, borrower, lender) = setup_env();

    // Deploy a collection and mint NFT
    let collection_id = create_collection(
        &env,
        borrower.clone(),
        String::from_slice(&env, "COLL"),
        String::from_slice(&env, "C"),
        String::from_slice(&env, "desc"),
        String::from_slice(&env, "uri"),
        0,
        0,
        borrower.clone(),
    )
    .unwrap();

    let token_id = mint_nft(
        &env,
        borrower.clone(),
        collection_id,
        String::from_slice(&env, "uri"),
        NFTStandard::ERC721,
        1,
    )
    .unwrap();

    // Set valuation to 120, so 100 loan is undercollateralized but not deeply
    set_nft_valuation(&env, collection_id, token_id, 120, crate::nft_types::ValuationMethod::Manual).unwrap();

    let loan_id = crate::nft_lending::request_loan(
        &env,
        borrower.clone(),
        collection_id,
        token_id,
        100,
        100, // 1% daily
        86400,
    )
    .unwrap();

    crate::nft_lending::fund_loan(&env, lender.clone(), loan_id).unwrap();

    let ltv = calculate_loan_ltv(&env, loan_id).unwrap();
    assert!(ltv > 7000);
    assert!(is_loan_undercollateralized(&env, loan_id).unwrap());

    let queued = monitor_and_queue_liquidations(&env);
    assert_eq!(queued, 1);

    let processed = process_liquidation_queue(&env, 1).unwrap();
    assert_eq!(processed, 1);

    let updated_loan = get_loan(&env, loan_id).unwrap();
    assert!(!updated_loan.is_liquidated, "partial liquidation should not mark liquidated");
    assert!(updated_loan.repayment_amount < 120, "remaining due should be reduced");
}

#[test]
fn test_full_liquidation_without_bids_transfers_to_lender() {
    let (env, borrower, lender) = setup_env();

    let collection_id = create_collection(
        &env,
        borrower.clone(),
        String::from_slice(&env, "COLL2"),
        String::from_slice(&env, "C2"),
        String::from_slice(&env, "desc2"),
        String::from_slice(&env, "uri2"),
        0,
        0,
        borrower.clone(),
    )
    .unwrap();

    let token_id = mint_nft(
        &env,
        borrower.clone(),
        collection_id,
        String::from_slice(&env, "uri2"),
        NFTStandard::ERC721,
        1,
    )
    .unwrap();

    set_nft_valuation(&env, collection_id, token_id, 80, crate::nft_types::ValuationMethod::Manual).unwrap();

    let loan_id = request_loan(&env, borrower.clone(), collection_id, token_id, 100, 1, 86400).unwrap();
    fund_loan(&env, lender.clone(), loan_id).unwrap();

    assert!(is_loan_undercollateralized(&env, loan_id).unwrap());

    monitor_and_queue_liquidations(&env);
    let processed = process_liquidation_queue(&env, 1).unwrap();
    assert_eq!(processed, 1);

    let updated_loan = get_loan(&env, loan_id).unwrap();
    assert!(updated_loan.is_liquidated);

    let owner = crate::nft_minting::get_nft(&env, collection_id, token_id).unwrap().owner;
    assert_eq!(owner, lender);
}
