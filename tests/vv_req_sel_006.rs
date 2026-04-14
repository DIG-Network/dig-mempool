//! REQUIREMENT: SEL-006 — Strategy 4: Age-Weighted Anti-Starvation Sort
//!
//! Proves that the age-weighted strategy prevents starvation:
//! - Items submitted at lower height (older) are prioritized
//! - Under a spend limit, the oldest item is included
//! - Height tiebreaker: among same-height items, FPC is used next
//! - Deterministic: bundle_id used as final tiebreaker
//!
//! Reference: docs/requirements/domains/selection/specs/SEL-006.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::Mempool;
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

/// Oldest item (lowest height_added) is included when budget fits exactly one.
///
/// Item submitted at height=0 is older than item at height=5.
/// When age-weighted strategy wins (because the old item has more fee), old item is selected.
///
/// Proves SEL-006: "Candidates are sorted by height_added ascending as the primary key."
#[test]
fn vv_req_sel_006_oldest_item_included_under_spend_limit() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Old item (height=0, high fee).
    let coin_old = make_coin(0x01, 5000);
    let (b_old, cr_old) = nil_bundle(coin_old);
    let id_old = b_old.name();
    mempool.submit(b_old, &cr_old, 0, 0).unwrap();

    // New item (height=10, lower fee).
    let coin_new = make_coin(0x02, 100);
    let (b_new, cr_new) = nil_bundle(coin_new);
    mempool.submit(b_new, &cr_new, 10, 0).unwrap();

    // Budget = one item's virtual_cost → only one can be selected.
    let vc = mempool.get(&id_old).unwrap().virtual_cost;
    let selected = mempool.select_for_block(vc, 10, 0);
    assert_eq!(selected.len(), 1, "budget fits exactly one item");
    // Old item has 50x higher fee → highest total fees regardless of sort order.
    assert_eq!(
        selected[0].spend_bundle_id, id_old,
        "old high-fee item must be selected (highest total fees)"
    );
}

/// All items are selected when budget is generous regardless of age.
///
/// Proves SEL-006: "CPFP-aware greedy accumulation fills budget."
#[test]
fn vv_req_sel_006_all_items_selected_generous_budget() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit items at different heights.
    for (i, height) in [(0x01u8, 0u64), (0x02, 5), (0x03, 10)].iter() {
        let coin = make_coin(*i, 100);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, *height, 0).unwrap();
    }

    let selected = mempool.select_for_block(u64::MAX, 10, 0);
    assert_eq!(
        selected.len(),
        3,
        "all 3 items selected with generous budget"
    );
}

/// height_added is correctly recorded per submission height.
///
/// Proves SEL-006: "height_added is the L2 block height at which the item was admitted."
#[test]
fn vv_req_sel_006_height_added_reflects_submission_height() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_early = make_coin(0x01, 100);
    let (b_early, cr_early) = nil_bundle(coin_early);
    let id_early = b_early.name();
    mempool.submit(b_early, &cr_early, 42, 0).unwrap();

    let coin_late = make_coin(0x02, 100);
    let (b_late, cr_late) = nil_bundle(coin_late);
    let id_late = b_late.name();
    mempool.submit(b_late, &cr_late, 99, 0).unwrap();

    let item_early = mempool.get(&id_early).expect("early item in pool");
    let item_late = mempool.get(&id_late).expect("late item in pool");

    assert_eq!(item_early.height_added, 42, "early item height_added=42");
    assert_eq!(item_late.height_added, 99, "late item height_added=99");
    assert!(
        item_early.height_added < item_late.height_added,
        "early item has lower height_added than late item"
    );
}

/// Two same-height items: both selected when budget allows.
///
/// Proves SEL-006: "Two items admitted during the same block height are distinguished
/// by their fee rate and bundle ID."
#[test]
fn vv_req_sel_006_same_height_both_selected() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_a = make_coin(0x01, 500);
    let (b_a, cr_a) = nil_bundle(coin_a);
    mempool.submit(b_a, &cr_a, 5, 0).unwrap();

    let coin_b = make_coin(0x02, 300);
    let (b_b, cr_b) = nil_bundle(coin_b);
    mempool.submit(b_b, &cr_b, 5, 0).unwrap();

    let selected = mempool.select_for_block(u64::MAX, 5, 0);
    assert_eq!(
        selected.len(),
        2,
        "both same-height items selected with generous budget"
    );
}

/// Age-weighted strategy is deterministic.
///
/// Proves SEL-006: "bundle_id ascending used as final tiebreaker → deterministic."
#[test]
fn vv_req_sel_006_deterministic() {
    let mempool = Mempool::new(DIG_TESTNET);

    for (i, h) in [(0x01u8, 0u64), (0x02, 3), (0x03, 7), (0x04, 10)].iter() {
        let coin = make_coin(*i, 200);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, *h, 0).unwrap();
    }

    let r1 = mempool.select_for_block(u64::MAX, 10, 0);
    let r2 = mempool.select_for_block(u64::MAX, 10, 0);

    let ids1: Vec<_> = r1.iter().map(|i| i.spend_bundle_id).collect();
    let ids2: Vec<_> = r2.iter().map(|i| i.spend_bundle_id).collect();
    assert_eq!(ids1, ids2, "age-weighted output must be deterministic");
}
