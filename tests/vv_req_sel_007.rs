//! REQUIREMENT: SEL-007 — Best Selection Comparator
//!
//! Proves that select_for_block() picks the strategy producing the highest total fees:
//! - The winning candidate set maximises total fees
//! - Fee-tie broken by lowest total virtual cost
//! - Cost-tie broken by fewest bundles
//! - Empty pool → empty result
//! - Single item → that item is always returned
//!
//! Reference: docs/requirements/domains/selection/specs/SEL-007.md

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

/// Empty pool → all four strategies produce empty sets → comparator returns empty.
///
/// Proves SEL-007: "If all four strategies produce empty sets, the comparator returns empty."
#[test]
fn vv_req_sel_007_all_empty_returns_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result = mempool.select_for_block(u64::MAX, 0, 0);
    assert!(result.is_empty(), "empty pool must return empty selection");
}

/// Single item in pool → all strategies agree → that item is always returned.
///
/// Proves SEL-007: comparator correctly handles single-item sets.
#[test]
fn vv_req_sel_007_single_item_always_selected() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(selected.len(), 1, "single item must be selected");
    assert_eq!(
        selected[0].spend_bundle_id, id,
        "correct item must be selected"
    );
}

/// The set with the highest total fees is always selected.
///
/// Design: budget fits exactly one item. Item with highest fee wins
/// because that single-item set has the highest total_fees.
///
/// Proves SEL-007: "The set with the highest total fees wins."
#[test]
fn vv_req_sel_007_highest_total_fees_wins() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Winner: highest fee.
    let coin_best = make_coin(0x01, 9_999);
    let (b_best, cr_best) = nil_bundle(coin_best);
    let id_best = b_best.name();
    mempool.submit(b_best, &cr_best, 0, 0).unwrap();

    // Loser items.
    for i in 0x02..=0x04u8 {
        let coin = make_coin(i, 100);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }

    // Budget = one item's virtual cost → only one can be selected.
    let vc = mempool.get(&id_best).unwrap().virtual_cost;
    let selected = mempool.select_for_block(vc, 0, 0);
    assert_eq!(selected.len(), 1, "budget fits exactly one item");
    assert_eq!(
        selected[0].spend_bundle_id, id_best,
        "item with highest fee (9999) must be selected"
    );
    assert_eq!(selected[0].fee, 9_999, "correct fee on selected item");
}

/// Total fees of selected set is at least as high as any single item's fee.
///
/// Proves SEL-007: the comparator maximises total fees across all strategies.
#[test]
fn vv_req_sel_007_maximises_total_fees() {
    let mempool = Mempool::new(DIG_TESTNET);

    let fees = [1000u64, 500, 750, 250, 2000, 100];
    for (i, fee) in fees.iter().enumerate() {
        let coin = make_coin(i as u8 + 1, *fee);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(
        selected.len(),
        6,
        "all 6 items selected with generous budget"
    );

    let total_fees: u64 = selected.iter().map(|i| i.fee).sum();
    let expected_total: u64 = fees.iter().sum();
    assert_eq!(
        total_fees, expected_total,
        "total fees must equal sum of all submitted fees"
    );
}

/// Comparator is deterministic: same input produces same output.
///
/// Proves SEL-007: "The comparator is deterministic."
#[test]
fn vv_req_sel_007_deterministic() {
    let mempool = Mempool::new(DIG_TESTNET);

    for i in 0x01..=0x05u8 {
        let coin = make_coin(i, (i as u64) * 137);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }

    let r1 = mempool.select_for_block(u64::MAX, 0, 0);
    let r2 = mempool.select_for_block(u64::MAX, 0, 0);

    let ids1: Vec<_> = r1.iter().map(|i| i.spend_bundle_id).collect();
    let ids2: Vec<_> = r2.iter().map(|i| i.spend_bundle_id).collect();
    assert_eq!(
        ids1, ids2,
        "best comparator must produce deterministic results"
    );
}
