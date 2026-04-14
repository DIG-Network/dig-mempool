//! REQUIREMENT: CFR-003 — RBF FPC Strictly Higher
//!
//! The incoming bundle's fee-per-virtual-cost must be strictly greater than
//! the aggregate fee-per-virtual-cost of all conflicting bundles.
//!
//! - Higher FPC passes (strict superset with large fee boost)
//! - Equal FPC → RbfFpcNotHigher
//! - Lower FPC → RbfFpcNotHigher
//! - Aggregate FPC compared (not per-conflict)
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-003.md

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

fn alt_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let alt_sol = Program::new(vec![0x01].into());
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), alt_sol)],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

fn two_coin_bundle(coin_a: Coin, coin_b: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
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

/// Higher FPC passes: replacement has higher fee per virtual cost.
///
/// Proves CFR-003: "New bundle FPC strictly > aggregate conflict FPC."
#[test]
fn vv_req_cfr_003_higher_fpc_passes() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // FPC_B = 128/(2×vc_1) = 64/vc_1 >> 1/vc_1 = FPC_A ✓
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success), "Higher FPC should pass");
    assert_eq!(mempool.len(), 1);
}

/// Equal FPC: conflict and new bundle have identical FPC → rejected.
///
/// Proves CFR-003: "equal FPC is NOT strictly higher → RbfFpcNotHigher."
#[test]
fn vv_req_cfr_003_equal_fpc_rejected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 10);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // alt solution → different hash, same coin → same fee → same FPC
    let (bundle_b, cr_b) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Equal FPC should fail, got: {:?}",
        result
    );
}

/// Lower FPC: new bundle's FPC is lower than existing → rejected.
///
/// Proves CFR-003: "lower FPC → RbfFpcNotHigher."
/// Bundle A: 1 spend of coin_X(100) → FPC_A = 100/vc_1.
/// Bundle B: {coin_X(100), coin_cheap(1)} → FPC_B ≈ 50.5/vc_1 < FPC_A.
#[test]
fn vv_req_cfr_003_lower_fpc_rejected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 100);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let coin_cheap = make_coin(0x0E, 1);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_cheap);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Lower FPC should fail, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool unchanged");
}

/// Aggregate FPC compared across all conflicts.
///
/// Proves CFR-003: "FPC comparison is against the aggregate of all conflicts."
/// A: fee=1, B: fee=1. C: fee=129 with 3 spends. FPC_C >> FPC_agg.
#[test]
fn vv_req_cfr_003_aggregate_fpc_compared() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let (bundle_b, cr_b) = nil_bundle(coin_y);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Higher aggregate FPC should succeed, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Both A and B replaced by C");
}
