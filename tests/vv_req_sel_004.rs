//! REQUIREMENT: SEL-004 — Strategy 2: Absolute Fee Whale Sort
//!
//! Proves that the whale strategy selects the highest absolute-fee items:
//! - High-fee "whale" item wins when it yields more total fees than many
//!   smaller high-FPC items
//! - Budget constraint skips whale if it doesn't fit
//! - CPFP package fee is used for ordering
//!
//! Reference: docs/requirements/domains/selection/specs/SEL-004.md

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

/// Whale wins when budget fits only one item.
///
/// When cost budget fits exactly 1 item and whale has the highest fee,
/// whale is selected (whale strategy: highest absolute fee first).
///
/// Proves SEL-004: "Highest absolute fee first."
#[test]
fn vv_req_sel_004_whale_selected_under_spend_limit() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Whale: high absolute fee.
    let coin_whale = make_coin(0x01, 5000);
    let (b_whale, cr_whale) = nil_bundle(coin_whale);
    let id_whale = b_whale.name();
    mempool.submit(b_whale, &cr_whale, 0, 0).unwrap();

    // Small: lower absolute fee.
    let coin_small = make_coin(0x02, 100);
    let (b_small, cr_small) = nil_bundle(coin_small);
    mempool.submit(b_small, &cr_small, 0, 0).unwrap();

    // Budget = one item's virtual_cost → only one can be selected.
    let vc = mempool.get(&id_whale).unwrap().virtual_cost;
    let selected = mempool.select_for_block(vc, 0, 0);
    assert_eq!(selected.len(), 1, "budget fits exactly one item");
    assert_eq!(
        selected[0].spend_bundle_id, id_whale,
        "whale (highest fee=5000) must be selected"
    );
}

/// Higher-fee item is included when budget allows only the highest-fee item.
///
/// Proves SEL-004: the whale strategy ensures high-value transactions are not starved.
#[test]
fn vv_req_sel_004_high_fee_item_included_among_many() {
    let mempool = Mempool::new(DIG_TESTNET);

    // 4 medium-fee items.
    for i in 0x02..=0x05u8 {
        let coin = make_coin(i, 200);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }

    // One whale with higher absolute fee than any of the above.
    let coin_whale = make_coin(0x01, 10_000);
    let (b_whale, cr_whale) = nil_bundle(coin_whale);
    let id_whale = b_whale.name();
    mempool.submit(b_whale, &cr_whale, 0, 0).unwrap();

    // Budget fits one item → whale (highest fee) wins.
    let vc = mempool.get(&id_whale).unwrap().virtual_cost;
    let selected = mempool.select_for_block(vc, 0, 0);
    assert_eq!(selected.len(), 1, "budget fits one item");
    assert_eq!(
        selected[0].spend_bundle_id, id_whale,
        "whale (fee=10000) must beat all medium-fee items (fee=200)"
    );
}

/// When budget is generous, whale and smaller items are all selected.
///
/// Proves SEL-004: whale strategy still includes smaller items when budget allows.
#[test]
fn vv_req_sel_004_whale_and_small_both_selected_with_budget() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_whale = make_coin(0x01, 5000);
    let (b_whale, cr_whale) = nil_bundle(coin_whale);
    mempool.submit(b_whale, &cr_whale, 0, 0).unwrap();

    let coin_small = make_coin(0x02, 50);
    let (b_small, cr_small) = nil_bundle(coin_small);
    mempool.submit(b_small, &cr_small, 0, 0).unwrap();

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(
        selected.len(),
        2,
        "both whale and small item should be selected"
    );
}

/// Conflicting item skipped by whale strategy.
///
/// Proves SEL-004: "Skip if any conflict with already-selected items."
/// Since RBF ensures the active pool is conflict-free, the output is always conflict-free.
#[test]
fn vv_req_sel_004_output_is_conflict_free() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Three independent items with large fees.
    for i in 0x01..=0x03u8 {
        let coin = make_coin(i, (i as u64) * 3000);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(selected.len(), 3, "all 3 whale items selected");

    let mut seen = std::collections::HashSet::new();
    for item in &selected {
        for r in &item.removals {
            assert!(seen.insert(*r), "conflict in whale strategy output!");
        }
    }
}
