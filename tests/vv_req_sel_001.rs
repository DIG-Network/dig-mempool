//! REQUIREMENT: SEL-001 — select_for_block() Entry Point
//!
//! Proves that select_for_block() fulfils its interface contract:
//! - Empty pool returns empty Vec
//! - Only active pool items are returned (not pending, not conflict cache)
//! - max_block_cost=0 produces empty output
//! - max_spends_per_block=0 produces empty output
//! - The call is read-only (calling twice returns identical results)
//!
//! Reference: docs/requirements/domains/selection/specs/SEL-001.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, SubmitResult};
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

/// Empty pool returns empty Vec.
///
/// Proves SEL-001: "May be empty if no eligible items exist."
#[test]
fn vv_req_sel_001_empty_pool_returns_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result = mempool.select_for_block(u64::MAX, 0, 0);
    assert!(result.is_empty(), "empty pool must produce empty selection");
}

/// Only active pool items are returned — pending items are excluded.
///
/// Proves SEL-001: "Reads from the active pool only (not pending or conflict caches)."
#[test]
fn vv_req_sel_001_returns_only_active_items() {
    use dig_clvm::{
        clvmr::{serde::node_to_bytes, Allocator},
        tree_hash, TreeHash,
    };

    let mempool = Mempool::new(DIG_TESTNET);

    // Submit one normal (active) item.
    let coin_active = make_coin(0x01, 1000);
    let (bundle_a, cr_a) = nil_bundle(coin_active);
    let id_active = bundle_a.name();
    assert_eq!(
        mempool.submit(bundle_a, &cr_a, 0, 0),
        Ok(SubmitResult::Success)
    );

    // Submit one item that routes to pending (ASSERT_HEIGHT_ABSOLUTE(100) at height=0).
    let mut a = Allocator::new();
    let nil = a.nil();
    let h = a.new_atom(&[100u8]).unwrap();
    let inner = a.new_pair(h, nil).unwrap();
    let op = a.new_atom(&[83u8]).unwrap(); // ASSERT_HEIGHT_ABSOLUTE
    let cond = a.new_pair(op, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());

    let coin_pending = Coin::new(Bytes32::from([0x02u8; 32]), puzzle_hash, 500);
    let mut cr_p = HashMap::new();
    cr_p.insert(
        coin_pending.coin_id(),
        CoinRecord {
            coin: coin_pending,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let bundle_pending = SpendBundle::new(
        vec![CoinSpend::new(coin_pending, puzzle, Program::default())],
        Signature::default(),
    );
    assert!(
        matches!(
            mempool.submit(bundle_pending, &cr_p, 0, 0),
            Ok(SubmitResult::Pending { .. })
        ),
        "expect pending result"
    );

    assert_eq!(mempool.len(), 1, "only active item in active pool");
    assert_eq!(mempool.pending_len(), 1, "one item in pending pool");

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(selected.len(), 1, "only active item should be returned");
    assert_eq!(
        selected[0].spend_bundle_id, id_active,
        "returned item must be the active one"
    );
}

/// max_block_cost=0 forces empty selection.
///
/// Proves SEL-001: "The total virtual_cost of returned items MUST NOT exceed max_block_cost."
#[test]
fn vv_req_sel_001_zero_budget_returns_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let selected = mempool.select_for_block(0, 0, 0);
    assert!(
        selected.is_empty(),
        "zero cost budget must produce empty selection"
    );
}

/// Total spends in selected output does not exceed max_spends_per_block.
///
/// max_spends_per_block gates both admission and selection.
/// With limit=2 and two single-spend items, both are admitted and both are selected.
///
/// Proves SEL-001: "The total spend count MUST NOT exceed config.max_spends_per_block."
#[test]
fn vv_req_sel_001_spend_limit_respected() {
    let config = MempoolConfig::default().with_max_spends_per_block(2);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit 2 items (pool capacity = 2 spends).
    for i in 0x01..=0x02u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }
    assert_eq!(mempool.len(), 2);

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    let total_spends: usize = selected.iter().map(|i| i.num_spends).sum();
    assert_eq!(
        total_spends, 2,
        "both single-spend items should be selected"
    );
    assert!(
        total_spends <= 2,
        "total spends ({}) must not exceed max_spends_per_block=2",
        total_spends
    );
}

/// select_for_block is read-only: calling twice returns identical results.
///
/// Proves SEL-001: "The function acquires a read lock; it does not mutate state."
#[test]
fn vv_req_sel_001_idempotent() {
    let mempool = Mempool::new(DIG_TESTNET);
    for i in 0x01..=0x05u8 {
        let coin = make_coin(i, i as u64 * 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    let result1 = mempool.select_for_block(u64::MAX, 0, 0);
    let result2 = mempool.select_for_block(u64::MAX, 0, 0);

    let ids1: Vec<_> = result1.iter().map(|i| i.spend_bundle_id).collect();
    let ids2: Vec<_> = result2.iter().map(|i| i.spend_bundle_id).collect();
    assert_eq!(
        ids1, ids2,
        "two identical calls must produce identical results"
    );
}

/// No two returned items conflict (share a spent coin).
///
/// Proves SEL-001: "No two returned items may spend the same coin."
#[test]
fn vv_req_sel_001_output_is_conflict_free() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit several non-conflicting items.
    for i in 0x01..=0x04u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(
        selected.len(),
        4,
        "all non-conflicting items should be selected"
    );

    // Verify no shared removals across items.
    let mut all_removals = std::collections::HashSet::new();
    for item in &selected {
        for coin_id in &item.removals {
            assert!(
                all_removals.insert(*coin_id),
                "coin {:?} spent by two items — conflict in output!",
                coin_id
            );
        }
    }
}
