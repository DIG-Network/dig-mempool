//! REQUIREMENT: CFR-006 — Remove Conflicting Items on Successful RBF
//!
//! When RBF succeeds, all conflicting items are removed from the active pool
//! and coin_index is updated to point to the replacement bundle.
//!
//! - Conflicting item removed; replacement inserted; pool count stable
//! - Both conflicting items removed when new bundle beats both
//! - coin_index entries cleaned after removal; new bundle's entries installed
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-006.md

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

/// Successful RBF: conflicting item removed, replacement inserted.
///
/// Proves CFR-006: "Remove conflicting items on successful RBF."
#[test]
fn vv_req_cfr_006_conflicting_item_removed() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);
    assert!(mempool.contains(&a_id));

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    let b_id = bundle_b.name();
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success), "RBF should succeed");

    assert!(!mempool.contains(&a_id), "Old bundle A should be removed");
    assert!(mempool.contains(&b_id), "New bundle B should be inserted");
    assert_eq!(mempool.len(), 1, "Pool has 1 item: the replacement");
}

/// Successful RBF replacing two conflicting items.
///
/// Proves CFR-006: "Both conflicting items removed when new bundle beats both."
#[test]
fn vv_req_cfr_006_two_conflicts_both_removed() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let (bundle_b, cr_b) = nil_bundle(coin_y);
    let b_id = bundle_b.name();
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let c_id = bundle_c.name();
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success), "RBF with two conflicts should succeed");

    assert!(!mempool.contains(&a_id), "Bundle A should be removed");
    assert!(!mempool.contains(&b_id), "Bundle B should be removed");
    assert!(mempool.contains(&c_id), "Bundle C should be inserted");
    assert_eq!(mempool.len(), 1, "Only the replacement remains");
}

/// After successful RBF, coin_index points to replacement, not the evicted bundle.
///
/// Proves CFR-006: "coin_index updated after replacement."
/// After B replaces A, C (spending same coin_X, not superset) detects conflict
/// against B — proving the index was updated.
#[test]
fn vv_req_cfr_006_coin_index_updated_after_rbf() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);
    assert!(!mempool.contains(&a_id), "A should be gone");

    // C: only spends coin_X — conflicts with B, but B also spends coin_boost → not superset
    let (bundle_c, cr_c) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "coin_X should now be indexed to B; C fails superset, got: {:?}",
        result
    );
}
