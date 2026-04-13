//! REQUIREMENT: CFR-001 — Conflict Detection via coin_index
//!
//! Proves that the active pool's coin_index correctly identifies conflicts:
//! - No conflict on empty pool → direct admission
//! - Single conflict detected via coin_index
//! - Both conflicting bundles detected when C touches coins from A and B
//! - Pending pool items do NOT appear in conflict detection
//! - Conflict cache items do NOT appear in conflict detection
//! - coin_index cleaned on removal — no stale entries
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-001.md

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

/// alt_bundle uses a non-nil solution so bundle hash differs from nil_bundle.
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

/// No conflict on empty pool → direct admission.
///
/// Proves CFR-001: "An empty conflict set results in direct admission."
#[test]
fn vv_req_cfr_001_no_conflict_empty_pool() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 1);
    let (bundle, cr) = nil_bundle(coin);
    assert_eq!(mempool.submit(bundle, &cr, 0, 0), Ok(SubmitResult::Success));
    assert_eq!(mempool.len(), 1);
}

/// Single conflict detected: coin_index returns the existing bundle.
///
/// Proves CFR-001: "For each removal, look up coin_index. If found, add to conflict set."
#[test]
fn vv_req_cfr_001_single_conflict_detected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 10);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Bundle B: alt solution → different bundle hash, same coin_x, same fee=10 → equal FPC
    let (bundle_b, cr_b) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Equal-FPC conflicting bundle should fail with RbfFpcNotHigher, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool unchanged after RBF failure");
}

/// Multiple conflicts: bundle C touches coins from two separate active items.
///
/// Proves CFR-001: "All unique conflicting bundle IDs are collected."
#[test]
fn vv_req_cfr_001_multiple_conflicts_detected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let coin_y = make_coin(0x02, 1);
    let (bundle_b, cr_b) = nil_bundle(coin_y);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Bundle C should replace both A and B, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "A and B both removed; C inserted");
}

/// Pending pool items do NOT appear in conflict detection.
///
/// Proves CFR-001: "Pending pool items: NOT checked."
#[test]
fn vv_req_cfr_001_pending_not_conflict() {
    use dig_clvm::{
        clvmr::{serde::node_to_bytes, Allocator},
        tree_hash, TreeHash,
    };

    let mempool = Mempool::new(DIG_TESTNET);

    // Build a puzzle with ASSERT_HEIGHT_ABSOLUTE(100) → routes to pending pool
    let mut a = Allocator::new();
    let nil = a.nil();
    let h = a.new_atom(&[100u8]).unwrap();
    let inner = a.new_pair(h, nil).unwrap();
    let op = a.new_atom(&[83u8]).unwrap();
    let cond = a.new_pair(op, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());

    let coin_p = Coin::new(Bytes32::from([0x10u8; 32]), puzzle_hash, 1);
    let mut cr_p = HashMap::new();
    cr_p.insert(coin_p.coin_id(), coin_record(coin_p));
    let bundle_pending = SpendBundle::new(
        vec![CoinSpend::new(coin_p, puzzle, Program::default())],
        Signature::default(),
    );
    let r = mempool.submit(bundle_pending, &cr_p, 0, 0);
    assert!(
        matches!(r, Ok(SubmitResult::Pending { .. })),
        "Should go to pending pool, got: {:?}",
        r
    );
    assert_eq!(mempool.pending_len(), 1);

    // Coin with NIL_PUZZLE_HASH having same parent byte → different coin_id
    let coin_active = make_coin(0x10, 1);
    let (bundle_active, cr_active) = nil_bundle(coin_active);
    let result = mempool.submit(bundle_active, &cr_active, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Active bundle should be admitted with no conflict against pending item"
    );
    assert_eq!(mempool.len(), 1);
}

/// Conflict cache items do NOT appear in conflict detection.
///
/// Proves CFR-001: "Conflict cache items: NOT checked via coin_index."
#[test]
fn vv_req_cfr_001_conflict_cache_not_indexed() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_x = make_coin(0x01, 1);

    let (bundle_cached, _) = nil_bundle(coin_x);
    mempool.add_to_conflict_cache(bundle_cached, 1_000);
    assert_eq!(mempool.conflict_len(), 1);

    // A DIFFERENT bundle spending the same coin — not blocked by conflict cache
    let (bundle_new, cr_new) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_new, &cr_new, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Conflict cache items should not block via coin_index, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1);
}

/// coin_index cleaned on removal — no stale entries block re-submission.
///
/// Proves CFR-001: "After removal, coin_index is empty for that coin."
#[test]
fn vv_req_cfr_001_coin_index_cleaned_on_removal() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_x = make_coin(0x01, 1);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    assert!(mempool.remove(&a_id), "remove() should succeed");
    assert_eq!(mempool.len(), 0);

    // Same coin, different bundle hash — no stale conflict entry
    let (bundle_b, cr_b) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "After removal, same coin should be spendable without conflict"
    );
    assert_eq!(mempool.len(), 1);
}
