//! REQUIREMENT: API-008 — Query Methods
//!
//! Test-driven verification that all 14+ query methods exist on `Mempool`
//! with the correct signatures and return types.
//!
//! ## What this proves
//!
//! - All methods compile with correct parameter and return types
//! - Methods work on `&self` (read-only, thread-safe)
//! - Empty mempool returns sensible defaults (empty vecs, None, 0, true)
//! - CPFP coin queries have correct signatures
//!
//! ## Note on Scope
//!
//! These tests verify the API surface (signatures + types). Behavioral tests
//! for populated mempools will be added when the pool data structures are
//! implemented (Phase 2: POL-001 through POL-010).
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-008.md

use std::sync::Arc;

use dig_constants::DIG_TESTNET;
use dig_mempool::item::MempoolItem;
use dig_mempool::{Bytes32, CoinRecord, Mempool, MempoolStats};

/// Test: get() returns None for unknown bundle ID on empty mempool.
///
/// Proves `get()` exists with signature `(&self, &Bytes32) -> Option<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_get_returns_none() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: Option<Arc<MempoolItem>> = mempool.get(&Bytes32::default());
    assert!(result.is_none());
}

/// Test: contains() returns false for unknown bundle ID.
///
/// Proves `contains()` exists with signature `(&self, &Bytes32) -> bool`.
#[test]
fn vv_req_api_008_contains_returns_false() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: bool = mempool.contains(&Bytes32::default());
    assert!(!result);
}

/// Test: active_bundle_ids() returns empty vec on empty mempool.
///
/// Proves signature `(&self) -> Vec<Bytes32>`.
#[test]
fn vv_req_api_008_active_bundle_ids_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let ids: Vec<Bytes32> = mempool.active_bundle_ids();
    assert!(ids.is_empty());
}

/// Test: pending_bundle_ids() returns empty vec.
///
/// Proves signature `(&self) -> Vec<Bytes32>`.
#[test]
fn vv_req_api_008_pending_bundle_ids_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let ids: Vec<Bytes32> = mempool.pending_bundle_ids();
    assert!(ids.is_empty());
}

/// Test: active_items() returns empty vec.
///
/// Proves signature `(&self) -> Vec<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_active_items_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let items: Vec<Arc<MempoolItem>> = mempool.active_items();
    assert!(items.is_empty());
}

/// Test: dependents_of() returns empty vec for unknown bundle.
///
/// Proves signature `(&self, &Bytes32) -> Vec<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_dependents_of_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let deps: Vec<Arc<MempoolItem>> = mempool.dependents_of(&Bytes32::default());
    assert!(deps.is_empty());
}

/// Test: ancestors_of() returns empty vec for unknown bundle.
///
/// Proves signature `(&self, &Bytes32) -> Vec<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_ancestors_of_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let ancestors: Vec<Arc<MempoolItem>> = mempool.ancestors_of(&Bytes32::default());
    assert!(ancestors.is_empty());
}

/// Test: len(), pending_len(), conflict_len() return 0.
///
/// Proves count methods exist with `(&self) -> usize` signatures.
#[test]
fn vv_req_api_008_count_methods() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.len(), 0);
    assert_eq!(mempool.pending_len(), 0);
    assert_eq!(mempool.conflict_len(), 0);
}

/// Test: is_empty() returns true on empty mempool.
///
/// Proves signature `(&self) -> bool`.
#[test]
fn vv_req_api_008_is_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert!(mempool.is_empty());
}

/// Test: stats() returns MempoolStats.
///
/// Proves signature `(&self) -> MempoolStats`.
/// Already tested in API-006, but included here for API surface completeness.
#[test]
fn vv_req_api_008_stats() {
    let mempool = Mempool::new(DIG_TESTNET);
    let _stats: MempoolStats = mempool.stats();
}

/// Test: get_mempool_coin_record() returns None for unknown coin.
///
/// Proves signature `(&self, &Bytes32) -> Option<CoinRecord>`.
/// This method is for CPFP: look up coins created by active mempool items.
#[test]
fn vv_req_api_008_get_mempool_coin_record() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: Option<CoinRecord> = mempool.get_mempool_coin_record(&Bytes32::default());
    assert!(result.is_none());
}

/// Test: get_mempool_coin_creator() returns None for unknown coin.
///
/// Proves signature `(&self, &Bytes32) -> Option<Bytes32>`.
/// Returns the bundle ID that created a mempool coin (for CPFP dependency building).
#[test]
fn vv_req_api_008_get_mempool_coin_creator() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: Option<Bytes32> = mempool.get_mempool_coin_creator(&Bytes32::default());
    assert!(result.is_none());
}

/// Test: All query methods work on shared reference (&self).
///
/// Proves all queries use `&self` (not `&mut self`), enabling concurrent
/// reads. This is critical for the interior mutability design (Decision #1).
#[test]
fn vv_req_api_008_all_read_only() {
    let mempool = Mempool::new(DIG_TESTNET);
    // Call all methods on the same &self — no &mut needed
    let _ = mempool.get(&Bytes32::default());
    let _ = mempool.contains(&Bytes32::default());
    let _ = mempool.active_bundle_ids();
    let _ = mempool.pending_bundle_ids();
    let _ = mempool.active_items();
    let _ = mempool.dependents_of(&Bytes32::default());
    let _ = mempool.ancestors_of(&Bytes32::default());
    let _ = mempool.len();
    let _ = mempool.pending_len();
    let _ = mempool.conflict_len();
    let _ = mempool.is_empty();
    let _ = mempool.stats();
    let _ = mempool.get_mempool_coin_record(&Bytes32::default());
    let _ = mempool.get_mempool_coin_creator(&Bytes32::default());
}
