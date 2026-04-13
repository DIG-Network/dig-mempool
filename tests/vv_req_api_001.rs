//! REQUIREMENT: API-001 — Mempool Constructors
//!
//! TDD tests for `Mempool::new()` and `Mempool::with_config()`.
//! Written BEFORE implementation per TDD workflow.

use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig};

#[test]
fn vv_req_api_001_new_compiles_and_returns_mempool() {
    let _mempool = Mempool::new(DIG_TESTNET);
}

#[test]
fn vv_req_api_001_with_config_compiles_and_returns_mempool() {
    let config = MempoolConfig::default();
    let _mempool = Mempool::with_config(DIG_TESTNET, config);
}

#[test]
fn vv_req_api_001_default_config_max_cost() {
    let mempool = Mempool::new(DIG_TESTNET);
    let stats = mempool.stats();
    // L2_MAX_COST_PER_BLOCK (550B) * MEMPOOL_BLOCK_BUFFER (15) = 8_250_000_000_000
    assert_eq!(stats.max_cost, 8_250_000_000_000);
}

#[test]
fn vv_req_api_001_custom_config_applied() {
    let config = MempoolConfig::default().with_max_total_cost(1_000_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    let stats = mempool.stats();
    assert_eq!(stats.max_cost, 1_000_000);
}

#[test]
fn vv_req_api_001_empty_on_construction() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.len(), 0);
    assert!(mempool.is_empty());
}

#[test]
fn vv_req_api_001_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Mempool>();
}

#[test]
fn vv_req_api_001_multiple_instances_independent() {
    let m1 = Mempool::new(DIG_TESTNET);
    let m2 = Mempool::new(DIG_TESTNET);
    assert_eq!(m1.len(), 0);
    assert_eq!(m2.len(), 0);
    // Both are independent — no shared state
}
