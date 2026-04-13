//! REQUIREMENT: CFR-002 — RBF Superset Rule
//!
//! The incoming bundle's removal set must be a strict superset of every
//! conflicting bundle's removal set.
//!
//! - Superset passes (new bundle spends the conflicted coin plus extra)
//! - Missing removal → RbfNotSuperset
//! - Multiple conflicts — new bundle must cover ALL removals of ALL conflicts
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-002.md

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

/// Superset passes: new bundle spends the conflicting coin plus extra coins.
///
/// Proves CFR-002: "Superset rule passes when new bundle's removals ⊇ every
/// conflict's removals."
#[test]
fn vv_req_cfr_002_superset_passes() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: {coin_X, coin_boost(127)} — strict superset with higher FPC
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Superset with higher fee should succeed, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "A replaced by B");
}

/// Missing removal: conflict's coin not in new bundle → RbfNotSuperset.
///
/// Proves CFR-002: "If any conflicting bundle has a removal not in the new
/// bundle, RbfNotSuperset is returned."
#[test]
fn vv_req_cfr_002_missing_removal_fails() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: spends both coin_X and coin_Y
    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);
    let (bundle_a, cr_a) = two_coin_bundle(coin_x, coin_y);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: spends only coin_X — misses coin_Y → RbfNotSuperset
    let (bundle_b, cr_b) = nil_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "Missing removal should return RbfNotSuperset, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool unchanged after superset failure");
}

/// Multiple conflicts — new bundle must cover ALL removals of ALL conflicts.
///
/// Proves CFR-002: "Aggregation across all conflicting bundles."
///
/// Bundle A spends {coin_X, coin_Y}. Bundle B spends {coin_Z}. Bundle C
/// spends {coin_X, coin_Z, coin_boost}: conflicts with A (via X) and B (via Z).
/// C misses coin_Y from A → RbfNotSuperset.
#[test]
fn vv_req_cfr_002_multiple_conflicts_all_must_be_covered() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);
    let coin_z = make_coin(0x03, 1);
    let coin_boost = make_coin(0x0F, 127);

    let (bundle_a, cr_a) = two_coin_bundle(coin_x, coin_y);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let (bundle_b, cr_b) = nil_bundle(coin_z);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // C misses coin_Y from A's removals → RbfNotSuperset
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_z, coin_boost);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "C misses coin_Y from A's removals; got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 2, "Pool unchanged after superset failure");
}
