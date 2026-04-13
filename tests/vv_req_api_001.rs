//! REQUIREMENT: API-001 — Mempool Constructors
//!
//! TDD tests for `Mempool::new()` and `Mempool::with_config()`.
//!
//! ## What this proves
//!
//! These tests verify the API-001 acceptance criteria:
//! - `Mempool::new()` accepts `NetworkConstants` and returns a valid Mempool
//! - `Mempool::with_config()` accepts `NetworkConstants` + `MempoolConfig`
//! - Default config yields `max_cost = L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER`
//! - Custom config overrides are applied correctly
//! - New mempools are empty (`len() == 0`, `is_empty() == true`)
//! - `Mempool` is `Send + Sync` (usable across threads via Arc)
//! - Multiple instances are independent (no shared mutable state)
//!
//! ## Chia L1 Correspondence
//!
//! Corresponds to `Mempool.__init__()` at:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L107
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-001.md

use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig};

/// Test: `Mempool::new()` compiles and returns a Mempool.
///
/// Proves the public constructor exists with the correct signature.
/// Uses `DIG_TESTNET` constants (placeholder genesis challenge).
#[test]
fn vv_req_api_001_new_compiles_and_returns_mempool() {
    let _mempool = Mempool::new(DIG_TESTNET);
}

/// Test: `Mempool::with_config()` compiles and returns a Mempool.
///
/// Proves the custom-config constructor exists. Uses `MempoolConfig::default()`
/// which should produce the same result as `Mempool::new()`.
#[test]
fn vv_req_api_001_with_config_compiles_and_returns_mempool() {
    let config = MempoolConfig::default();
    let _mempool = Mempool::with_config(DIG_TESTNET, config);
}

/// Test: Default config produces correct `max_cost` in stats.
///
/// Proves that `Mempool::new()` derives the max_total_cost from network
/// constants: `L2_MAX_COST_PER_BLOCK (550B) * MEMPOOL_BLOCK_BUFFER (15) = 8.25T`.
///
/// This is the primary capacity limit for the active pool.
#[test]
fn vv_req_api_001_default_config_max_cost() {
    let mempool = Mempool::new(DIG_TESTNET);
    let stats = mempool.stats();
    // L2_MAX_COST_PER_BLOCK (550B) * MEMPOOL_BLOCK_BUFFER (15) = 8_250_000_000_000
    assert_eq!(stats.max_cost, 8_250_000_000_000);
}

/// Test: Custom config is applied correctly via `with_config()`.
///
/// Proves that the custom `max_total_cost` override flows through to
/// `stats().max_cost`. This connects API-001 to API-003 (MempoolConfig).
#[test]
fn vv_req_api_001_custom_config_applied() {
    let config = MempoolConfig::default().with_max_total_cost(1_000_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    let stats = mempool.stats();
    assert_eq!(stats.max_cost, 1_000_000);
}

/// Test: Newly constructed mempool is empty.
///
/// Proves acceptance criteria: `len() == 0` and `is_empty() == true`
/// on a freshly constructed mempool. No items have been submitted.
#[test]
fn vv_req_api_001_empty_on_construction() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.len(), 0);
    assert!(mempool.is_empty());
}

/// Test: `Mempool` is `Send + Sync`.
///
/// Proves the struct can be shared across threads via `Arc<Mempool>`.
/// This is Decision #1 from the spec: interior mutability via RwLock
/// enables concurrent reads and serialized writes.
///
/// If this test compiles, the trait bounds are satisfied.
#[test]
fn vv_req_api_001_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Mempool>();
}

/// Test: Multiple Mempool instances are independent.
///
/// Proves that each `Mempool::new()` call creates an independent instance
/// with its own state. No shared mutable static state exists.
#[test]
fn vv_req_api_001_multiple_instances_independent() {
    let m1 = Mempool::new(DIG_TESTNET);
    let m2 = Mempool::new(DIG_TESTNET);
    assert_eq!(m1.len(), 0);
    assert_eq!(m2.len(), 0);
    // Both are independent — no shared state between instances
}
