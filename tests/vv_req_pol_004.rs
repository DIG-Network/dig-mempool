//! REQUIREMENT: POL-004 — Pending Pool
//!
//! Test-driven verification that submit() routes future-timelocked items to a
//! separate pending pool and that the pool enforces count and cost limits.
//!
//! ## What this proves
//!
//! - Items with `assert_height > current_height` get `SubmitResult::Pending`
//! - Pending items are NOT in the active pool (`contains()` returns false)
//! - `pending_len()` tracks the pending item count
//! - `stats().pending_count` matches `pending_len()`
//! - `pending_bundle_ids()` enumerates pending items
//! - Count limit `max_pending_count` is enforced (→ `PendingPoolFull`)
//! - Cost limit `max_pending_cost` is enforced (→ `PendingPoolFull`)
//! - `drain_pending(height, timestamp)` promotes ready items
//! - Items not yet mature are NOT promoted by drain_pending
//! - After promotion, items are removed from the pending pool
//!
//! ## Chia L1 Correspondence
//!
//! Corresponds to Chia's `PendingTxCache` at:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/pending_tx_cache.py#L50
//!
//! Reference: docs/requirements/domains/pools/specs/POL-004.md

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
#[allow(dead_code)]
const NIL_PUZZLE_HASH: [u8; 32] =
    hex!("4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a");

/// ASSERT_HEIGHT_ABSOLUTE opcode = 83 (0x53).
const ASSERT_HEIGHT_ABSOLUTE: u8 = 83;

/// Encode a positive integer as a minimum-length big-endian CLVM atom.
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

/// Build a CLVM puzzle that returns `((ASSERT_HEIGHT_ABSOLUTE height))`.
///
/// The program is `(q . ((83 . (height . ()))))` — a quoted constant that
/// produces a single condition when executed with any solution.
fn make_aha_puzzle(height: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();

    let height_atom = a.new_atom(&encode_uint(height)).unwrap();
    let inner = a.new_pair(height_atom, nil).unwrap(); // (height . ())
    let opcode = a.new_atom(&[ASSERT_HEIGHT_ABSOLUTE]).unwrap();
    let cond = a.new_pair(opcode, inner).unwrap(); // (83 . (height . ()))
    let cond_list = a.new_pair(cond, nil).unwrap(); // ((83 height))
    let q = a.new_atom(&[1u8]).unwrap(); // quote opcode
    let prog = a.new_pair(q, cond_list).unwrap(); // (q . ((83 height)))

    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());

    (puzzle, puzzle_hash)
}

/// Create a bundle with `ASSERT_HEIGHT_ABSOLUTE height` condition.
fn aha_bundle(
    parent_prefix: u8,
    amount: u64,
    assert_height: u64,
    coin_records: &mut HashMap<Bytes32, CoinRecord>,
) -> SpendBundle {
    let (puzzle, puzzle_hash) = make_aha_puzzle(assert_height);
    let coin = Coin::new(Bytes32::from([parent_prefix; 32]), puzzle_hash, amount);
    let coin_id = coin.coin_id();
    coin_records.insert(
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
    SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    )
}

/// Create a plain nil-puzzle bundle (no timelock).
#[allow(dead_code)]
fn nil_bundle(
    parent_prefix: u8,
    amount: u64,
    coin_records: &mut HashMap<Bytes32, CoinRecord>,
) -> SpendBundle {
    let coin = Coin::new(
        Bytes32::from([parent_prefix; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        amount,
    );
    let coin_id = coin.coin_id();
    coin_records.insert(
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
    SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    )
}

/// Measure the virtual_cost of a 1-spend aha bundle with height=100 (1-byte atom).
///
/// Heights 1–127 all encode as 1-byte CLVM atoms, giving identical virtual_cost.
fn probe_aha_virtual_cost() -> u64 {
    let probe = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    // Submit at current_height=0 with assert_height=100 → goes to pending
    let b = aha_bundle(0xff, 1, 100, &mut cr);
    let result = probe.submit(b, &cr, 0, 0);
    assert!(
        matches!(result, Ok(SubmitResult::Pending { .. })),
        "probe bundle should go to pending, got: {:?}",
        result
    );
    probe.stats().pending_cost
}

// ──────────────────────────────────────────────────────────────────────────
// Routing: timelocked items go to pending
// ──────────────────────────────────────────────────────────────────────────

/// Test: Timelocked item is routed to pending pool.
///
/// Proves POL-004: items with `assert_height > current_height` return
/// `SubmitResult::Pending { assert_height }`.
#[test]
fn vv_req_pol_004_timelocked_item_goes_to_pending() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    // assert_height=100, current_height=0 → should go to pending
    let b = aha_bundle(0x01, 1, 100, &mut cr);

    let result = mempool.submit(b, &cr, 0, 0);
    assert!(
        matches!(result, Ok(SubmitResult::Pending { assert_height: 100 })),
        "Expected Pending {{ assert_height: 100 }}, got: {:?}",
        result
    );
}

/// Test: Pending item is not in the active pool.
///
/// Proves POL-004: "pending pool is separate from the active pool."
#[test]
fn vv_req_pol_004_pending_not_in_active_pool() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    let b = aha_bundle(0x01, 1, 100, &mut cr);
    let b_id = b.name();

    mempool.submit(b, &cr, 0, 0).unwrap();

    assert_eq!(mempool.len(), 0, "Active pool should be empty");
    assert!(
        mempool.is_empty(),
        "Mempool should report empty active pool"
    );
    assert!(
        !mempool.contains(&b_id),
        "contains() should not find pending items"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Query methods
// ──────────────────────────────────────────────────────────────────────────

/// Test: pending_len() increments when pending items are added.
///
/// Proves POL-004: the pending pool tracks its item count.
#[test]
fn vv_req_pol_004_pending_len_increments() {
    let mempool = Mempool::new(DIG_TESTNET);

    assert_eq!(mempool.pending_len(), 0);

    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    let mut cr2 = HashMap::new();
    let b2 = aha_bundle(0x02, 2, 200, &mut cr2);
    mempool.submit(b2, &cr2, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 2);
}

/// Test: stats().pending_count matches pending_len().
///
/// Proves POL-004: MempoolStats.pending_count reflects the pending pool size.
#[test]
fn vv_req_pol_004_stats_pending_count() {
    let mempool = Mempool::new(DIG_TESTNET);

    assert_eq!(mempool.stats().pending_count, 0);

    let mut cr = HashMap::new();
    let b = aha_bundle(0x01, 1, 100, &mut cr);
    mempool.submit(b, &cr, 0, 0).unwrap();

    assert_eq!(mempool.stats().pending_count, 1);
    assert_eq!(mempool.stats().pending_count, mempool.pending_len());
}

/// Test: pending_bundle_ids() returns IDs of all pending items.
///
/// Proves POL-004: pending pool is enumerable.
#[test]
fn vv_req_pol_004_pending_bundle_ids() {
    let mempool = Mempool::new(DIG_TESTNET);

    assert!(mempool.pending_bundle_ids().is_empty());

    let mut cr = HashMap::new();
    let b = aha_bundle(0x01, 1, 100, &mut cr);
    let b_id = b.name();
    mempool.submit(b, &cr, 0, 0).unwrap();

    let ids = mempool.pending_bundle_ids();
    assert_eq!(ids.len(), 1);
    assert!(
        ids.contains(&b_id),
        "pending_bundle_ids() must contain the submitted bundle"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Capacity limits
// ──────────────────────────────────────────────────────────────────────────

/// Test: Count limit max_pending_count is enforced.
///
/// Proves POL-004: "count limit max_pending_count (default 3000) is enforced."
#[test]
fn vv_req_pol_004_count_limit_enforced() {
    let config = MempoolConfig::default().with_max_pending_count(2);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Fill to max_pending_count
    for i in 0u8..2 {
        let mut cr = HashMap::new();
        let b = aha_bundle(i + 1, 1, 100, &mut cr);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }
    assert_eq!(mempool.pending_len(), 2);

    // One more should exceed the limit
    let mut cr = HashMap::new();
    let b = aha_bundle(0x10, 1, 100, &mut cr);
    let result = mempool.submit(b, &cr, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::PendingPoolFull)),
        "Should get PendingPoolFull when count limit exceeded, got: {:?}",
        result
    );
    assert_eq!(mempool.pending_len(), 2, "Pool should still have 2 items");
}

/// Test: Cost limit max_pending_cost is enforced.
///
/// Proves POL-004: cost limit is enforced with PendingPoolFull.
#[test]
fn vv_req_pol_004_cost_limit_enforced() {
    let aha_vc = probe_aha_virtual_cost();

    // Capacity for exactly 1 aha bundle
    let config = MempoolConfig::default().with_max_pending_cost(aha_vc + 1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // First bundle fits
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // Second bundle exceeds cost limit
    let mut cr2 = HashMap::new();
    let b2 = aha_bundle(0x02, 1, 100, &mut cr2);
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::PendingPoolFull)),
        "Should get PendingPoolFull when cost limit exceeded, got: {:?}",
        result
    );
    assert_eq!(mempool.pending_len(), 1);
}

// ──────────────────────────────────────────────────────────────────────────
// Promotion: drain_pending
// ──────────────────────────────────────────────────────────────────────────

/// Test: drain_pending promotes items whose timelocks are satisfied.
///
/// Proves POL-004: "on_new_block extracts promotable items."
#[test]
fn vv_req_pol_004_promotion_on_new_block() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    let b = aha_bundle(0x01, 1, 100, &mut cr);
    let b_id = b.name();
    mempool.submit(b, &cr, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // At height=100, assert_height=100 is satisfied
    let promoted = mempool.drain_pending(100, 0);
    assert_eq!(promoted.len(), 1, "One bundle should be promoted");
    assert_eq!(
        promoted[0].name(),
        b_id,
        "Promoted bundle should match the submitted bundle"
    );
}

/// Test: Items are not promoted when their timelock is not yet satisfied.
///
/// Proves POL-004: items remain in pending if assert_height > current height.
#[test]
fn vv_req_pol_004_not_promoted_too_early() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    let b = aha_bundle(0x01, 1, 100, &mut cr);
    mempool.submit(b, &cr, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // At height=99, assert_height=100 is NOT satisfied
    let promoted = mempool.drain_pending(99, 0);
    assert!(promoted.is_empty(), "No bundles should be promoted at h=99");
    assert_eq!(mempool.pending_len(), 1, "Item should still be in pending");
}

/// Test: After promotion, item is removed from the pending pool.
///
/// Proves POL-004: drain_pending removes items from pending storage.
#[test]
fn vv_req_pol_004_removal_cleans_up() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    let b = aha_bundle(0x01, 1, 100, &mut cr);
    let b_id = b.name();
    mempool.submit(b, &cr, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // Promote
    let promoted = mempool.drain_pending(100, 0);
    assert_eq!(promoted.len(), 1);

    // Item should be gone from pending
    assert_eq!(
        mempool.pending_len(),
        0,
        "Pending pool should be empty after promotion"
    );
    assert!(
        !mempool.pending_bundle_ids().contains(&b_id),
        "Promoted item should not be in pending_bundle_ids()"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Pending cost tracking
// ──────────────────────────────────────────────────────────────────────────

/// Test: stats().pending_cost reflects the sum of pending items' virtual costs.
///
/// Proves POL-004: "pending_cost accumulator."
#[test]
fn vv_req_pol_004_pending_cost_tracked() {
    let aha_vc = probe_aha_virtual_cost();
    let mempool = Mempool::new(DIG_TESTNET);

    assert_eq!(mempool.stats().pending_cost, 0);

    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.stats().pending_cost, aha_vc);

    let mut cr2 = HashMap::new();
    let b2 = aha_bundle(0x02, 1, 110, &mut cr2);
    mempool.submit(b2, &cr2, 0, 0).unwrap();
    assert_eq!(mempool.stats().pending_cost, aha_vc * 2);
}
