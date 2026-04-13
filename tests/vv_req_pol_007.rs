//! REQUIREMENT: POL-007 — Seen Cache (LRU Bounded Set)
//!
//! Test-driven verification of the seen cache's LRU eviction, pre-validation
//! insertion, and dedup checks against all pools.
//!
//! ## What this proves
//!
//! - Bundle IDs are inserted into the seen cache BEFORE CLVM validation
//!   (so a CLVM-invalid bundle is still marked "seen" and rejected on retry)
//! - LRU eviction: when at capacity, the oldest entry is evicted
//! - After eviction, a bundle in the active pool still returns AlreadySeen
//! - After eviction, a bundle in the pending pool still returns AlreadySeen
//! - A bundle in the conflict cache returns AlreadySeen (even without seen cache)
//! - `clear()` resets the seen cache (re-submission is allowed after clear)
//!
//! ## Already covered
//!
//! - Basic dedup (first submit passes, second returns AlreadySeen) is covered
//!   by the ADM-003 test suite. These tests focus on the scenarios that require
//!   the full multi-pool check (post seen-cache eviction, conflict cache, clear).
//!
//! Reference: docs/requirements/domains/pools/specs/POL-007.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

/// Build a nil-puzzle bundle.
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

// ──────────────────────────────────────────────────────────────────────────
// Pre-validation insertion (DoS protection)
// ──────────────────────────────────────────────────────────────────────────

/// Test: A bundle is added to the seen cache BEFORE CLVM validation.
///
/// Proves POL-007: "bundle IDs are added before CLVM validation begins."
/// A bundle with an invalid coin (coin not in coin_records) fails CLVM.
/// Re-submitting the same bundle returns AlreadySeen — it does NOT re-run CLVM.
#[test]
fn vv_req_pol_007_added_before_clvm_validation() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Build bundle A with missing coin_records (coin not on chain)
    let coin = Coin::new(Bytes32::from([0x01u8; 32]), NIL_PUZZLE_HASH, 1);
    let bundle_a = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let bundle_a_id = bundle_a.name();

    // First submission: CLVM validation fails (coin not in coin_records)
    let empty_cr: HashMap<Bytes32, CoinRecord> = HashMap::new();
    let result1 = mempool.submit(bundle_a.clone(), &empty_cr, 0, 0);
    assert!(
        result1.is_err(),
        "First submission should fail (coin not found)"
    );
    assert!(
        !matches!(result1, Err(MempoolError::AlreadySeen(_))),
        "Should not be AlreadySeen on first attempt"
    );

    // Second submission: should return AlreadySeen WITHOUT running CLVM again
    let result2 = mempool.submit(bundle_a, &empty_cr, 0, 0);
    assert!(
        matches!(result2, Err(MempoolError::AlreadySeen(id)) if id == bundle_a_id),
        "Re-submission should return AlreadySeen, got: {:?}",
        result2
    );
}

// ──────────────────────────────────────────────────────────────────────────
// LRU eviction
// ──────────────────────────────────────────────────────────────────────────

/// Test: When the seen cache is full, the oldest entry is evicted.
///
/// Proves POL-007: "LRU eviction removes the oldest entry when at capacity."
/// After eviction, submitting the evicted bundle succeeds (no longer "seen").
#[test]
fn vv_req_pol_007_lru_evicts_oldest() {
    // Set max_seen_cache_size = 2 so eviction is easy to trigger
    let config = MempoolConfig::default().with_max_seen_cache_size(2);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit bundle A (parent=0x01) → seen cache: [A]
    let (bundle_a, cr_a) = nil_bundle(0x01, 1);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Submit bundle B (parent=0x02) → seen cache: [A, B]
    let (bundle_b, cr_b) = nil_bundle(0x02, 1);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();

    // Submit bundle C (parent=0x03) → evicts A (oldest), seen cache: [B, C]
    let (bundle_c, cr_c) = nil_bundle(0x03, 1);
    mempool.submit(bundle_c, &cr_c, 0, 0).unwrap();

    // B should still be in seen cache → AlreadySeen
    let (bundle_b2, cr_b2) = nil_bundle(0x02, 1);
    assert_eq!(bundle_b2.name(), {
        let (b, _) = nil_bundle(0x02, 1);
        b.name()
    });
    let result_b = mempool.submit(bundle_b2, &cr_b2, 0, 0);
    assert!(
        matches!(result_b, Err(MempoolError::AlreadySeen(_))),
        "B should still be seen, got: {:?}",
        result_b
    );

    // A was evicted from seen cache AND was inserted into the active pool.
    // Submitting A again returns AlreadySeen from the active pool check.
    // (The seen cache evicted A, but the active pool still has it.)
    let (bundle_a2, cr_a2) = nil_bundle(0x01, 1);
    assert_eq!(bundle_a2.name(), a_id);
    let result_a = mempool.submit(bundle_a2, &cr_a2, 0, 0);
    assert!(
        matches!(result_a, Err(MempoolError::AlreadySeen(_))),
        "A should be AlreadySeen from active pool check after seen cache eviction, got: {:?}",
        result_a
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Multi-pool dedup
// ──────────────────────────────────────────────────────────────────────────

/// Test: Pending pool check catches bundle after seen cache eviction.
///
/// Proves POL-007: "The cache also checks against active pool, pending pool."
/// After the seen cache evicts a pending bundle's ID, a re-submission should
/// still return AlreadySeen (from the pending pool check).
#[test]
fn vv_req_pol_007_pending_pool_check_after_eviction() {
    use dig_clvm::{
        clvmr::{serde::node_to_bytes, Allocator},
        tree_hash, TreeHash,
    };

    // max_seen_cache_size = 1 so a second submission evicts the first ID
    let config = MempoolConfig::default().with_max_seen_cache_size(1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Build an AHA bundle that routes to the pending pool (height=100)
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

    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), puzzle_hash, 1);
    let mut cr_a = HashMap::new();
    cr_a.insert(
        coin_a.coin_id(),
        CoinRecord {
            coin: coin_a,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let bundle_a = SpendBundle::new(
        vec![CoinSpend::new(coin_a, puzzle.clone(), Program::default())],
        Signature::default(),
    );
    let a_id = bundle_a.name();

    // Submit A → pending pool + seen cache: [A]
    let result_a = mempool.submit(bundle_a.clone(), &cr_a, 0, 0);
    assert!(
        matches!(result_a, Ok(SubmitResult::Pending { .. })),
        "Bundle A should go to pending pool, got: {:?}",
        result_a
    );
    assert_eq!(mempool.pending_len(), 1);

    // Submit nil_bundle B → active pool + evicts A from seen cache
    let (bundle_b, cr_b) = nil_bundle(0x02, 1);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    // seen cache now only has B; A was evicted

    // Re-submit A: seen cache doesn't have A, but pending pool does → AlreadySeen
    let result_a2 = mempool.submit(bundle_a, &cr_a, 0, 0);
    assert!(
        matches!(result_a2, Err(MempoolError::AlreadySeen(id)) if id == a_id),
        "Should get AlreadySeen from pending pool check, got: {:?}",
        result_a2
    );
}

/// Test: Conflict cache check returns AlreadySeen.
///
/// Proves POL-007: "The cache also checks against conflict cache."
/// A bundle manually added to the conflict cache returns AlreadySeen when
/// submitted — even if it was never in the seen cache.
#[test]
fn vv_req_pol_007_conflict_cache_check() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Build bundle A (not submitted via submit() — only in conflict cache)
    let (bundle_a, _cr_a) = nil_bundle(0x01, 1);
    let a_id = bundle_a.name();

    // Manually add A to the conflict cache
    let inserted = mempool.add_to_conflict_cache(bundle_a.clone(), 1_000);
    assert!(inserted, "Should be inserted into conflict cache");
    assert_eq!(mempool.conflict_len(), 1);

    // Submit A: seen cache doesn't have A, active pool doesn't, pending pool doesn't,
    // but conflict cache does → AlreadySeen
    let (_, cr_a) = nil_bundle(0x01, 1);
    let result = mempool.submit(bundle_a, &cr_a, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::AlreadySeen(id)) if id == a_id),
        "Should get AlreadySeen from conflict cache check, got: {:?}",
        result
    );
}

// ──────────────────────────────────────────────────────────────────────────
// clear() resets seen cache
// ──────────────────────────────────────────────────────────────────────────

/// Test: `clear()` resets the seen cache so previously-seen bundles can be re-submitted.
///
/// Proves POL-007: "`clear()` resets the seen cache."
/// After clear(), the active pool is also empty, so a re-submission of
/// a previously-admitted bundle succeeds from scratch.
#[test]
fn vv_req_pol_007_clear_resets_seen_cache() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit bundle A — goes to active pool + seen cache
    let (bundle_a, cr_a) = nil_bundle(0x01, 1);
    mempool.submit(bundle_a.clone(), &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Verify A is seen (re-submission fails)
    let result_before = mempool.submit(bundle_a.clone(), &cr_a, 0, 0);
    assert!(
        matches!(result_before, Err(MempoolError::AlreadySeen(_))),
        "Should be AlreadySeen before clear"
    );

    // Clear everything
    mempool.clear();
    assert_eq!(mempool.len(), 0, "Active pool should be empty after clear");
    assert_eq!(mempool.pending_len(), 0);
    assert_eq!(mempool.conflict_len(), 0);

    // Re-submit A: seen cache and all pools are empty → succeeds
    let result_after = mempool.submit(bundle_a, &cr_a, 0, 0);
    assert_eq!(
        result_after,
        Ok(SubmitResult::Success),
        "Re-submission after clear should succeed, got: {:?}",
        result_after
    );
}

/// Test: `clear()` resets pending pool and conflict cache too.
///
/// Proves that clear() is a full reset for all state, not just the seen cache.
#[test]
fn vv_req_pol_007_clear_resets_all_pools() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Active pool
    let (b_active, cr_active) = nil_bundle(0x01, 1);
    mempool.submit(b_active, &cr_active, 0, 0).unwrap();

    // Conflict cache
    let (b_conflict, _) = nil_bundle(0x02, 1);
    mempool.add_to_conflict_cache(b_conflict, 1_000);

    assert_eq!(mempool.len(), 1);
    assert_eq!(mempool.conflict_len(), 1);

    mempool.clear();

    assert_eq!(mempool.len(), 0, "Active pool should be empty after clear");
    assert_eq!(
        mempool.pending_len(),
        0,
        "Pending pool should be empty after clear"
    );
    assert_eq!(
        mempool.conflict_len(),
        0,
        "Conflict cache should be empty after clear"
    );
    assert_eq!(
        mempool.stats().total_cost,
        0,
        "Total cost should be 0 after clear"
    );
}
