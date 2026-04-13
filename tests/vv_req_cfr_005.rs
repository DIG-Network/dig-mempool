//! REQUIREMENT: CFR-005 — Conflict Cache on RBF Failure
//!
//! When any RBF check fails, the rejected bundle is added to the conflict
//! cache so it can be retried after the conflicting item is evicted.
//!
//! - Superset failure → bundle added to conflict cache
//! - FPC failure → bundle added to conflict cache
//! - Minimum fee bump failure → bundle added to conflict cache
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-005.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError};
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

/// Failed superset check → bundle added to conflict cache; error returned.
///
/// Proves CFR-005: "On RBF failure, the bundle is added to conflict cache."
#[test]
fn vv_req_cfr_005_superset_failure_caches_bundle() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);
    let (bundle_a, cr_a) = two_coin_bundle(coin_x, coin_y);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B spends {coin_X} only → not superset → RbfNotSuperset + cached
    let (bundle_b, cr_b) = nil_bundle(coin_x);
    let b_id = bundle_b.name();
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "Expected RbfNotSuperset, got: {:?}",
        result
    );
    assert_eq!(mempool.conflict_len(), 1, "Failed bundle should be in conflict cache");

    let cached = mempool.drain_conflict();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].name(), b_id, "Cached bundle ID should match");
}

/// Failed FPC check → bundle added to conflict cache; error returned.
///
/// Proves CFR-005: "On RBF failure (FPC), bundle is added to conflict cache."
#[test]
fn vv_req_cfr_005_fpc_failure_caches_bundle() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 100);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: lower FPC → RbfFpcNotHigher + cached
    let coin_cheap = make_coin(0x0E, 1);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_cheap);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Expected RbfFpcNotHigher, got: {:?}",
        result
    );
    assert_eq!(mempool.conflict_len(), 1, "Bundle should be in conflict cache");
}

/// Failed fee bump → bundle added to conflict cache; error returned.
///
/// Proves CFR-005: "On RBF failure (bump too low), bundle cached."
#[test]
fn vv_req_cfr_005_bump_failure_caches_bundle() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(1_000_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // fee=101; required = 1 + 1_000_000 = 1_000_001; FPC passes but bump fails
    let coin_extra = make_coin(0x0F, 100);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_extra);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfBumpTooLow { .. })),
        "Expected RbfBumpTooLow, got: {:?}",
        result
    );
    assert_eq!(
        mempool.conflict_len(),
        1,
        "Bundle should be in conflict cache after bump failure"
    );
}
