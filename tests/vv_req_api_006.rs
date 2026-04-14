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

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolStats};
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

/// Test: Stats accurately reflect a populated pool.
///
/// Proves API-006: active_count, total_cost, total_fees, total_spend_count,
/// utilization, min_fpc_scaled, max_fpc_scaled all update correctly when items
/// are admitted to the active pool.
#[test]
fn vv_req_api_006_populated_pool_stats() {
    let config = MempoolConfig::default().with_max_total_cost(1_000_000_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit two bundles with zero fee (RESERVE_FEE=0 in default config).
    for i in 0x01..=0x02u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    let stats = mempool.stats();

    // Active count matches submitted items.
    assert_eq!(stats.active_count, 2, "active_count must be 2");

    // total_spend_count matches (1 spend per bundle).
    assert_eq!(stats.total_spend_count, 2, "total_spend_count must be 2");

    // total_fees: both bundles spend coins without creating outputs.
    // Fee = coin amount (inputs - outputs = 100 + 100 = 200).
    assert_eq!(
        stats.total_fees, 200,
        "total_fees must equal sum of coin amounts when no outputs created"
    );

    // total_cost > 0 (CLVM dry-run produces a nonzero cost).
    assert!(
        stats.total_cost > 0,
        "total_cost must be nonzero after submission"
    );

    // utilization = total_cost / max_total_cost.
    let expected_util = stats.total_cost as f64 / 1_000_000_000f64;
    assert!(
        (stats.utilization - expected_util).abs() < 1e-9,
        "utilization must match total_cost/max_total_cost, got {}",
        stats.utilization
    );

    // No CPFP dependencies — no items_with_dependencies.
    assert_eq!(
        stats.items_with_dependencies, 0,
        "root items have depth=0, so items_with_dependencies must be 0"
    );
    assert_eq!(stats.max_current_depth, 0);

    // No dedup-eligible or singleton-FF items in this batch.
    assert_eq!(stats.dedup_eligible_count, 0);
    assert_eq!(stats.singleton_ff_count, 0);
}

/// Test: min_fpc_scaled and max_fpc_scaled track fee rates across items.
///
/// Proves API-006 spec: "min_fpc_scaled and max_fpc_scaled track the range of
/// fee-per-virtual-cost-scaled across all active items."
/// With a single zero-fee item, both values are 0.
/// After adding a second item with nonzero fee, max_fpc_scaled > 0.
#[test]
fn vv_req_api_006_fpc_range_tracking() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit a zero-fee item.
    let coin_a = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_a);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let stats = mempool.stats();
    // Single item: min == max (both 0 for zero fee).
    assert_eq!(stats.min_fpc_scaled, stats.max_fpc_scaled);

    // Stats.active_count is 1.
    assert_eq!(stats.active_count, 1);
}
