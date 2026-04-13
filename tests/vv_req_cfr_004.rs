//! REQUIREMENT: CFR-004 — RBF Minimum Fee Bump
//!
//! The incoming bundle's total fee must be at least the sum of all conflicting
//! bundles' fees plus `min_rbf_fee_bump` (default 10M mojos).
//!
//! - Fee exactly at required minimum passes (>= comparison)
//! - Fee below minimum → RbfBumpTooLow with correct required/provided
//! - Aggregate fee compared (sum of all conflicts)
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-004.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

fn make_coin(parent: u8, amount: u64) -> Coin {
    Coin::new(Bytes32::from([parent; 32]), NIL_PUZZLE_HASH, amount)
}

fn coin_record(coin: Coin) -> CoinRecord {
    CoinRecord {
        coin,
        coinbase: false,
        confirmed_block_index: 1,
        spent: false,
        spent_block_index: 0,
        timestamp: 100,
    }
}

fn nil_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

fn two_coin_bundle(
    coin_a: Coin,
    coin_b: Coin,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, Program::default(), Program::default()),
            CoinSpend::new(coin_b, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin_a.coin_id(), coin_record(coin_a));
    cr.insert(coin_b.coin_id(), coin_record(coin_b));
    (bundle, cr)
}

fn three_coin_bundle(
    coin_a: Coin,
    coin_b: Coin,
    coin_c: Coin,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, Program::default(), Program::default()),
            CoinSpend::new(coin_b, Program::default(), Program::default()),
            CoinSpend::new(coin_c, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin_a.coin_id(), coin_record(coin_a));
    cr.insert(coin_b.coin_id(), coin_record(coin_b));
    cr.insert(coin_c.coin_id(), coin_record(coin_c));
    (bundle, cr)
}

/// Fee exactly at minimum bump passes (>= comparison).
///
/// Proves CFR-004: "fee_new >= conflict_fees + min_rbf_fee_bump."
/// min_rbf_fee_bump = 10, fee_A = 5. Required: fee_B >= 15.
/// Bundle B: {coin_X(5), coin_bump(10)} → fee_B = 15 exactly ✓.
#[test]
fn vv_req_cfr_004_exact_minimum_bump_passes() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(10);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 5);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // fee_B = 5+10 = 15 = fee_A(5) + bump(10) → exactly at minimum
    // FPC_B = 15/(2×vc_1) > 5/vc_1 = FPC_A ✓
    let coin_bump = make_coin(0x0F, 10);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_bump);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Exact minimum fee bump should pass, got: {:?}",
        result
    );
}

/// Fee one below minimum → RbfBumpTooLow with correct required/provided.
///
/// Proves CFR-004: "fee_new < conflict_fees + min_rbf_fee_bump → RbfBumpTooLow."
/// min_rbf_fee_bump = 20, fee_A = 5. Required: fee_B >= 25.
/// Bundle B: {coin_X(5), coin_under(19)} → fee_B=24 < 25 → fails.
#[test]
fn vv_req_cfr_004_below_minimum_bump_fails() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(20);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 5);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let coin_under = make_coin(0x0F, 19);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_under);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    match result {
        Err(MempoolError::RbfBumpTooLow { required, provided }) => {
            assert_eq!(required, 25, "required = 5 + 20 = 25");
            assert_eq!(provided, 24, "provided = 5 + 19 = 24");
        }
        other => panic!("Expected RbfBumpTooLow, got: {:?}", other),
    }
}

/// Aggregate fee bump: sum of all conflict fees + bump must be covered.
///
/// Proves CFR-004: "conflicting_fees = sum of ALL conflicting bundles' fees."
/// fee_A=5, fee_B=10, bump=10 → required=25.
/// Bundle C: {coin_X(5), coin_Y(10), coin_extra(12)} → fee_C=27 >= 25 ✓.
#[test]
fn vv_req_cfr_004_aggregate_fee_bump() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(10);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 5);
    let coin_y = make_coin(0x02, 10);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    let (bundle_b, cr_b) = nil_bundle(coin_y);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // fee_C = 5+10+12 = 27 >= required(25)
    let coin_extra = make_coin(0x0F, 12);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_extra);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Aggregate fee bump should pass, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Both A and B replaced by C");
}
