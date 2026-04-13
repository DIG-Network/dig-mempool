//! REQUIREMENT: LCY-002 — RetryBundles Struct
//!
//! Proves the RetryBundles struct:
//! - Is publicly exported from the crate
//! - Has conflict_retries: Vec<SpendBundle>
//! - Has pending_promotions: Vec<SpendBundle>
//! - Has cascade_evicted: Vec<Bytes32>
//! - Always returned by on_new_block(), even when all vectors are empty
//!
//! Reference: docs/requirements/domains/lifecycle/specs/LCY-002.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, RetryBundles};
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

/// RetryBundles is publicly exported and can be constructed.
///
/// Proves LCY-002: "RetryBundles is publicly exported from the crate."
#[test]
fn vv_req_lcy_002_struct_is_public() {
    let r = RetryBundles {
        conflict_retries: vec![],
        pending_promotions: vec![],
        cascade_evicted: vec![],
    };
    assert!(r.conflict_retries.is_empty());
    assert!(r.pending_promotions.is_empty());
    assert!(r.cascade_evicted.is_empty());
}

/// conflict_retries field accepts Vec<SpendBundle>.
///
/// Proves LCY-002: "Contains conflict_retries: Vec<SpendBundle>."
#[test]
fn vv_req_lcy_002_conflict_retries_field() {
    let coin = make_coin(0x01, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let r = RetryBundles {
        conflict_retries: vec![bundle],
        pending_promotions: vec![],
        cascade_evicted: vec![],
    };
    assert_eq!(r.conflict_retries.len(), 1);
}

/// pending_promotions field accepts Vec<SpendBundle>.
///
/// Proves LCY-002: "Contains pending_promotions: Vec<SpendBundle>."
#[test]
fn vv_req_lcy_002_pending_promotions_field() {
    let coin = make_coin(0x02, 500);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let r = RetryBundles {
        conflict_retries: vec![],
        pending_promotions: vec![bundle],
        cascade_evicted: vec![],
    };
    assert_eq!(r.pending_promotions.len(), 1);
}

/// cascade_evicted field accepts Vec<Bytes32>.
///
/// Proves LCY-002: "Contains cascade_evicted: Vec<Bytes32>."
#[test]
fn vv_req_lcy_002_cascade_evicted_field() {
    let id = Bytes32::from([0xAB; 32]);
    let r = RetryBundles {
        conflict_retries: vec![],
        pending_promotions: vec![],
        cascade_evicted: vec![id],
    };
    assert_eq!(r.cascade_evicted.len(), 1);
    assert_eq!(r.cascade_evicted[0], id);
}

/// on_new_block() returns RetryBundles with all-empty vectors when no coins match.
///
/// Proves LCY-002: "The struct is always returned, even if all fields are empty."
#[test]
fn vv_req_lcy_002_empty_on_noop_block() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    // Confirm a coin that is NOT in any submitted bundle.
    let unrelated_coin_id = Bytes32::from([0xFF; 32]);
    let retry = mempool.on_new_block(1, 100, &[unrelated_coin_id], &[]);
    assert!(
        retry.conflict_retries.is_empty(),
        "no conflict retries expected"
    );
    assert!(
        retry.pending_promotions.is_empty(),
        "no pending promotions expected"
    );
    assert!(
        retry.cascade_evicted.is_empty(),
        "no cascade evictions expected"
    );
}

/// on_new_block() removes confirmed items from the active pool.
///
/// Proves LCY-002: "All items in cascade_evicted have been removed from the active pool."
#[test]
fn vv_req_lcy_002_confirmed_item_removed_from_pool() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    assert!(
        mempool.contains(&bundle_id),
        "item must be in pool before confirmation"
    );

    // Confirm the coin spent by this bundle.
    mempool.on_new_block(1, 100, &[coin.coin_id()], &[]);

    assert!(
        !mempool.contains(&bundle_id),
        "confirmed item must be removed from pool"
    );
}
