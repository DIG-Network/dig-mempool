//! REQUIREMENT: POL-006 — Conflict Cache
//!
//! Test-driven verification that the conflict cache stores bundles that fail
//! active-pool RBF, with count and cost limits.
//!
//! ## What this proves
//!
//! - Conflict cache is initially empty
//! - `add_to_conflict_cache()` inserts a bundle and increments conflict_len()
//! - `stats().conflict_count` reflects the cache size
//! - Count limit (`max_conflict_count`) is enforced (silently dropped when full)
//! - Cost limit (`max_conflict_cost`) is enforced (silently dropped when full)
//! - `drain_conflict()` returns all cached bundles and clears the cache
//! - After drain, conflict_len() == 0
//! - Duplicate bundle IDs are not double-inserted
//! - Non-conflict rejections from submit() do NOT add to the conflict cache
//!
//! ## Note on full RBF integration
//!
//! The test "failed active-pool RBF stores bundle" is exercised in CFR-005,
//! once active-pool conflict detection (CFR-001) is wired up. This file
//! verifies the ConflictCache data structure and capacity limits in isolation.
//!
//! Reference: docs/requirements/domains/pools/specs/POL-006.md

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError};

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex_literal::hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

/// ASSERT_HEIGHT_ABSOLUTE opcode = 83.
const ASSERT_HEIGHT_ABSOLUTE: u8 = 83;

fn encode_uint(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![];
    }
    let be = v.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(7);
    let bytes = &be[start..];
    if bytes[0] >= 0x80 {
        let mut result = vec![0x00];
        result.extend_from_slice(bytes);
        result
    } else {
        bytes.to_vec()
    }
}

/// Build the nil-puzzle bundle (no timelocks, routes to active pool).
fn nil_bundle(parent_prefix: u8, amount: u64) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let coin = Coin::new(Bytes32::from([parent_prefix; 32]), NIL_PUZZLE_HASH, amount);
    let coin_id = coin.coin_id();
    let mut cr = HashMap::new();
    cr.insert(
        coin_id,
        CoinRecord {
            coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    (bundle, cr)
}

/// Build an AHA bundle (ASSERT_HEIGHT_ABSOLUTE, routes to pending pool).
fn aha_bundle(
    parent_prefix: u8,
    amount: u64,
    height: u64,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let height_atom = a.new_atom(&encode_uint(height)).unwrap();
    let inner = a.new_pair(height_atom, nil).unwrap();
    let opcode = a.new_atom(&[ASSERT_HEIGHT_ABSOLUTE]).unwrap();
    let cond = a.new_pair(opcode, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());

    let coin = Coin::new(Bytes32::from([parent_prefix; 32]), puzzle_hash, amount);
    let coin_id = coin.coin_id();
    let mut cr = HashMap::new();
    cr.insert(
        coin_id,
        CoinRecord {
            coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    (bundle, cr)
}

// ──────────────────────────────────────────────────────────────────────────
// Initial state
// ──────────────────────────────────────────────────────────────────────────

/// Test: Conflict cache starts empty.
///
/// Proves POL-006: "conflict_cache is initially empty."
#[test]
fn vv_req_pol_006_initial_state() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(
        mempool.conflict_len(),
        0,
        "Conflict cache should start empty"
    );
    assert_eq!(
        mempool.stats().conflict_count,
        0,
        "stats().conflict_count should start at 0"
    );
    let drained = mempool.drain_conflict();
    assert!(
        drained.is_empty(),
        "drain_conflict() on empty cache should return []"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Insert / count
// ──────────────────────────────────────────────────────────────────────────

/// Test: Adding a bundle increments conflict_len and stats.conflict_count.
///
/// Proves POL-006: "conflict_cache is a HashMap<Bytes32, SpendBundle>."
#[test]
fn vv_req_pol_006_add_increments_count() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, _) = nil_bundle(0x01, 1);
    let inserted = mempool.add_to_conflict_cache(bundle, 1_000_000);
    assert!(inserted, "First insertion should succeed");
    assert_eq!(mempool.conflict_len(), 1);
    assert_eq!(mempool.stats().conflict_count, 1);
}

// ──────────────────────────────────────────────────────────────────────────
// Count limit
// ──────────────────────────────────────────────────────────────────────────

/// Test: Count limit `max_conflict_count` is enforced — silently dropped when full.
///
/// Proves POL-006: "Count limit max_conflict_count (default 1000) is enforced."
/// Uses a tiny limit (3) for efficiency.
#[test]
fn vv_req_pol_006_count_limit_enforced() {
    let config = MempoolConfig::default().with_max_conflict_count(3);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Fill to the limit
    for i in 0u8..3 {
        let (bundle, _) = nil_bundle(i, 1);
        let inserted = mempool.add_to_conflict_cache(bundle, 1_000);
        assert!(inserted, "Insertion {} should succeed", i);
    }
    assert_eq!(mempool.conflict_len(), 3);

    // One more — should be silently dropped
    let (bundle, _) = nil_bundle(0xff, 1);
    let inserted = mempool.add_to_conflict_cache(bundle, 1_000);
    assert!(
        !inserted,
        "Insertion past count limit should be silently dropped"
    );
    assert_eq!(
        mempool.conflict_len(),
        3,
        "Cache size should not exceed max_conflict_count"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Cost limit
// ──────────────────────────────────────────────────────────────────────────

/// Test: Cost limit `max_conflict_cost` is enforced — silently dropped when full.
///
/// Proves POL-006: "Cost limit max_conflict_cost is enforced."
#[test]
fn vv_req_pol_006_cost_limit_enforced() {
    // Set max_conflict_cost = 5_000 and max_conflict_count = 1000
    let config = MempoolConfig::default()
        .with_max_conflict_count(1000)
        .with_max_conflict_cost(5_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Add bundles until cost limit is reached (cost 2_000 each)
    let (b1, _) = nil_bundle(0x01, 1);
    let (b2, _) = nil_bundle(0x02, 1);
    assert!(mempool.add_to_conflict_cache(b1, 2_000), "b1 should fit");
    assert!(
        mempool.add_to_conflict_cache(b2, 2_000),
        "b2 should fit (total=4_000 <= 5_000)"
    );
    assert_eq!(mempool.conflict_len(), 2);

    // Third bundle would push total to 6_000 > 5_000 — should be dropped
    let (b3, _) = nil_bundle(0x03, 1);
    let inserted = mempool.add_to_conflict_cache(b3, 2_000);
    assert!(
        !inserted,
        "Insertion past cost limit should be silently dropped"
    );
    assert_eq!(
        mempool.conflict_len(),
        2,
        "Cache size should not exceed cost limit"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Duplicate insertion
// ──────────────────────────────────────────────────────────────────────────

/// Test: Duplicate bundle IDs are not double-inserted.
///
/// Proves POL-006: "duplicate bundles are deduplicated."
#[test]
fn vv_req_pol_006_duplicate_not_inserted() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, _) = nil_bundle(0x01, 1);
    let bundle_copy = bundle.clone();
    assert!(mempool.add_to_conflict_cache(bundle, 1_000));
    // Insert the same bundle again — should return false (already present)
    let inserted = mempool.add_to_conflict_cache(bundle_copy, 1_000);
    assert!(!inserted, "Duplicate insertion should return false");
    assert_eq!(
        mempool.conflict_len(),
        1,
        "Duplicate should not change cache size"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Drain
// ──────────────────────────────────────────────────────────────────────────

/// Test: drain_conflict() returns all cached bundles.
///
/// Proves POL-006: "on_new_block() drains all entries."
#[test]
fn vv_req_pol_006_drain_returns_all() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (b1, _) = nil_bundle(0x01, 1);
    let (b2, _) = nil_bundle(0x02, 1);
    let (b3, _) = nil_bundle(0x03, 1);
    let id1 = b1.name();
    let id2 = b2.name();
    let id3 = b3.name();
    mempool.add_to_conflict_cache(b1, 1_000);
    mempool.add_to_conflict_cache(b2, 1_000);
    mempool.add_to_conflict_cache(b3, 1_000);
    assert_eq!(mempool.conflict_len(), 3);

    let drained = mempool.drain_conflict();
    assert_eq!(
        drained.len(),
        3,
        "drain_conflict() should return all 3 bundles"
    );

    let drained_ids: std::collections::HashSet<Bytes32> =
        drained.iter().map(|b| b.name()).collect();
    assert!(drained_ids.contains(&id1));
    assert!(drained_ids.contains(&id2));
    assert!(drained_ids.contains(&id3));
}

/// Test: drain_conflict() clears the cache.
///
/// Proves POL-006: "conflict cache is empty after drain."
#[test]
fn vv_req_pol_006_drain_clears_cache() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (b1, _) = nil_bundle(0x01, 1);
    mempool.add_to_conflict_cache(b1, 1_000);
    assert_eq!(mempool.conflict_len(), 1);

    mempool.drain_conflict();

    assert_eq!(
        mempool.conflict_len(),
        0,
        "Cache should be empty after drain"
    );
    assert_eq!(mempool.stats().conflict_count, 0);
    assert!(
        mempool.drain_conflict().is_empty(),
        "Second drain should return []"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Non-conflict rejections not cached
// ──────────────────────────────────────────────────────────────────────────

/// Test: Non-conflict submission failures do not add to the conflict cache.
///
/// Proves POL-006: "Items rejected for non-conflict reasons are NOT cached."
/// Any submit() that fails for a non-conflict reason (AlreadySeen, PendingPoolFull,
/// CostExceeded, etc.) must leave conflict_len() at 0.
#[test]
fn vv_req_pol_006_non_conflict_rejection_not_cached() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit a valid bundle
    let (bundle, cr) = nil_bundle(0x01, 1);
    mempool.submit(bundle.clone(), &cr, 0, 0).unwrap();

    // Re-submit the same bundle — AlreadySeen, not a conflict
    let result = mempool.submit(bundle, &cr, 0, 0);
    assert!(matches!(result, Err(MempoolError::AlreadySeen(_))));

    assert_eq!(
        mempool.conflict_len(),
        0,
        "AlreadySeen rejection must not add to conflict cache"
    );
}

/// Test: A pending RBF failure does not add to the conflict cache.
///
/// Proves POL-006: "Pending pool RBF failures do not go to the conflict cache."
/// (Covered also by POL-005 tests, reiterated here for completeness.)
#[test]
fn vv_req_pol_006_pending_rbf_not_cached() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit a pending item
    let (b1, cr1) = aha_bundle(0x01, 1, 100);
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // Attempt pending RBF with fee too low
    let (aha_puzzle, _aha_hash) = {
        let mut a = Allocator::new();
        let nil = a.nil();
        let height_atom = a.new_atom(&encode_uint(100)).unwrap();
        let inner = a.new_pair(height_atom, nil).unwrap();
        let opcode = a.new_atom(&[ASSERT_HEIGHT_ABSOLUTE]).unwrap();
        let cond = a.new_pair(opcode, inner).unwrap();
        let cond_list = a.new_pair(cond, nil).unwrap();
        let q = a.new_atom(&[1u8]).unwrap();
        let prog = a.new_pair(q, cond_list).unwrap();
        let hash: TreeHash = tree_hash(&a, prog);
        let puzzle_hash = Bytes32::from(hash);
        let bytes = node_to_bytes(&a, prog).unwrap();
        (Program::new(bytes.into()), puzzle_hash)
    };
    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), _aha_hash, 1);
    let coin_c = Coin::new(Bytes32::from([0x03u8; 32]), _aha_hash, 1);
    let mut cr2 = HashMap::new();
    for coin in [coin_a, coin_c] {
        cr2.insert(
            coin.coin_id(),
            CoinRecord {
                coin,
                coinbase: false,
                confirmed_block_index: 1,
                spent: false,
                spent_block_index: 0,
                timestamp: 100,
            },
        );
    }
    let b2 = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, aha_puzzle.clone(), Program::default()),
            CoinSpend::new(coin_c, aha_puzzle, Program::default()),
        ],
        Signature::default(),
    );
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(result.is_err(), "Pending RBF should fail");

    assert_eq!(
        mempool.conflict_len(),
        0,
        "Failed pending RBF must NOT add to conflict cache"
    );
}
