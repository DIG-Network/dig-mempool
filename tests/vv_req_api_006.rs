//! REQUIREMENT: API-006 — MempoolStats Struct
//!
//! Test-driven verification that `MempoolStats` has all specified fields
//! and is correctly returned by `Mempool::stats()`.
//!
//! ## What this proves
//!
//! - All 14 fields are present and publicly accessible
//! - `stats()` returns correct values for an empty mempool
//! - `max_cost` reflects `config.max_total_cost`
//! - The struct is Clone + Debug
//! - Utilization is 0.0 for an empty pool
//! - All counts/totals are zero for a new mempool
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-006.md

use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolStats};

/// Test: All fields are publicly accessible on MempoolStats.
///
/// Proves API-006 acceptance criterion: "All specified fields are present."
/// If any field is missing or private, this test won't compile.
#[test]
fn vv_req_api_006_all_fields_accessible() {
    let mempool = Mempool::new(DIG_TESTNET);
    let stats: MempoolStats = mempool.stats();

    // Access every field to prove they're public
    let _: usize = stats.active_count;
    let _: usize = stats.pending_count;
    let _: usize = stats.conflict_count;
    let _: u64 = stats.total_cost;
    let _: u64 = stats.total_fees;
    let _: u64 = stats.max_cost;
    let _: f64 = stats.utilization;
    let _: u128 = stats.min_fpc_scaled;
    let _: u128 = stats.max_fpc_scaled;
    let _: usize = stats.items_with_dependencies;
    let _: u32 = stats.max_current_depth;
    let _: usize = stats.total_spend_count;
    let _: usize = stats.dedup_eligible_count;
    let _: usize = stats.singleton_ff_count;
}

/// Test: Empty mempool has all-zero stats except max_cost.
///
/// Proves that a freshly constructed mempool returns correct initial values.
/// Only `max_cost` is nonzero (reflects config capacity).
#[test]
fn vv_req_api_006_empty_stats() {
    let mempool = Mempool::new(DIG_TESTNET);
    let stats = mempool.stats();

    assert_eq!(stats.active_count, 0);
    assert_eq!(stats.pending_count, 0);
    assert_eq!(stats.conflict_count, 0);
    assert_eq!(stats.total_cost, 0);
    assert_eq!(stats.total_fees, 0);
    assert_eq!(stats.utilization, 0.0);
    assert_eq!(stats.min_fpc_scaled, 0);
    assert_eq!(stats.max_fpc_scaled, 0);
    assert_eq!(stats.items_with_dependencies, 0);
    assert_eq!(stats.max_current_depth, 0);
    assert_eq!(stats.total_spend_count, 0);
    assert_eq!(stats.dedup_eligible_count, 0);
    assert_eq!(stats.singleton_ff_count, 0);
}

/// Test: max_cost reflects config.max_total_cost.
///
/// Proves the stats correctly expose the configured capacity.
/// Default: L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER = 8.25T.
#[test]
fn vv_req_api_006_max_cost_from_config() {
    // Default config
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.stats().max_cost, 8_250_000_000_000);

    // Custom config
    let config = MempoolConfig::default().with_max_total_cost(42_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    assert_eq!(mempool.stats().max_cost, 42_000);
}

/// Test: MempoolStats is Clone.
///
/// Stats snapshots should be cloneable for storage and comparison.
#[test]
fn vv_req_api_006_clone() {
    let stats = Mempool::new(DIG_TESTNET).stats();
    let cloned = stats.clone();
    assert_eq!(cloned.max_cost, stats.max_cost);
    assert_eq!(cloned.active_count, stats.active_count);
}

/// Test: MempoolStats is Debug.
///
/// Debug formatting is needed for logging.
#[test]
fn vv_req_api_006_debug() {
    let stats = Mempool::new(DIG_TESTNET).stats();
    let s = format!("{:?}", stats);
    assert!(s.contains("max_cost"));
    assert!(s.contains("active_count"));
}

/// Test: stats() is callable on &self (read-only, thread-safe).
///
/// Proves stats() uses &self (not &mut self), which means it can be
/// called concurrently with other read operations.
#[test]
fn vv_req_api_006_callable_on_shared_ref() {
    let mempool = Mempool::new(DIG_TESTNET);
    let _stats1 = mempool.stats();
    let _stats2 = mempool.stats(); // Two calls on same &self — no mut required
}
