//! REQUIREMENT: API-003 — MempoolConfig with Builder Pattern and Defaults
//!
//! Test-driven verification that `MempoolConfig` has all specified fields,
//! correct default values matching Chia L1 where applicable, and a full
//! builder pattern with `with_*` methods for every field.
//!
//! ## What this proves
//!
//! These tests verify every acceptance criterion from the API-003 spec:
//! - All 17 fields are present and public
//! - `Default` implementation provides all values from the spec table
//! - Builder methods exist for all fields and return `Self` for chaining
//! - Builder overrides are applied correctly, leaving other fields at defaults
//! - The struct is `Clone`
//! - Default values match Chia L1 equivalents (documented with citations)
//!
//! ## Chia L1 Reference Values
//!
//! | Field | Default | Chia Source |
//! |-------|---------|-------------|
//! | max_total_cost | 8,250,000,000,000 | L2_MAX_COST_PER_BLOCK * 15 |
//! | max_bundle_cost | 11,000,000,000 | L1_MAX_COST_PER_SPEND |
//! | min_rbf_fee_bump | 10,000,000 | MEMPOOL_MIN_FEE_INCREASE (mempool_manager.py:52) |
//! | spend_penalty_cost | 500,000 | SPEND_PENALTY_COST (mempool_item.py:14) |
//! | max_pending_count | 3,000 | PendingTxCache._cache_max_size |
//! | max_conflict_count | 1,000 | ConflictTxCache._cache_max_size |
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-003.md

use dig_mempool::config::{
    DEFAULT_EXPIRY_PROTECTION_BLOCKS, DEFAULT_FEE_ESTIMATOR_BUCKETS, DEFAULT_FEE_ESTIMATOR_WINDOW,
    DEFAULT_MAX_CONFLICT_COUNT, DEFAULT_MAX_DEPENDENCY_DEPTH, DEFAULT_MAX_PENDING_COUNT,
    DEFAULT_MAX_SPENDS_PER_BLOCK, DEFAULT_SEEN_CACHE_SIZE, FULL_MEMPOOL_MIN_FPC_SCALED,
    MIN_RBF_FEE_BUMP, SPEND_PENALTY_COST,
};
use dig_mempool::{MempoolConfig, FPC_SCALE, MEMPOOL_BLOCK_BUFFER};

/// Test: All default values match the specification table exactly.
///
/// Proves API-003 acceptance criterion: "Default implementation provides
/// all specified default values."
///
/// Each assertion references the corresponding row in the API-003 spec's
/// "Default Values" table and the Chia L1 source where applicable.
///
/// Chia refs:
/// - max_total_cost: mempool.py:509-514 (MempoolInfo.max_size_in_cost)
/// - max_bundle_cost: mempool_manager.py:733 (max_tx_clvm_cost)
/// - min_rbf_fee_bump: mempool_manager.py:52 (MEMPOOL_MIN_FEE_INCREASE = 10M)
/// - spend_penalty_cost: mempool_item.py:14 (SPEND_PENALTY_COST = 500K)
/// - full_mempool_min_fpc_scaled: mempool_manager.py:746 (nonzero_fee_minimum_fpc = 5)
/// - max_pending_count: pending_tx_cache.py:53 (_cache_max_size = 3000)
/// - max_conflict_count: pending_tx_cache.py:15 (_cache_max_size = 1000)
#[test]
fn vv_req_api_003_default_values_correct() {
    let config = MempoolConfig::default();

    // Active pool limits
    // L2_MAX_COST_PER_BLOCK = 550_000_000_000, MEMPOOL_BLOCK_BUFFER = 15
    // 550B * 15 = 8_250_000_000_000
    assert_eq!(
        config.max_total_cost,
        550_000_000_000 * MEMPOOL_BLOCK_BUFFER
    );
    assert_eq!(config.max_total_cost, 8_250_000_000_000);

    // L1_MAX_COST_PER_SPEND = 11_000_000_000 (Chia: MAX_BLOCK_COST_CLVM)
    assert_eq!(config.max_bundle_cost, 11_000_000_000);

    // MAX_SPENDS_PER_BLOCK = 6_000
    assert_eq!(config.max_spends_per_block, DEFAULT_MAX_SPENDS_PER_BLOCK);
    assert_eq!(config.max_spends_per_block, 6_000);

    // Pending pool limits (Chia: PendingTxCache defaults)
    assert_eq!(config.max_pending_count, DEFAULT_MAX_PENDING_COUNT);
    assert_eq!(config.max_pending_count, 3_000);
    assert_eq!(config.max_pending_cost, 550_000_000_000); // 1x block cost

    // Conflict cache limits (Chia: ConflictTxCache defaults)
    assert_eq!(config.max_conflict_count, DEFAULT_MAX_CONFLICT_COUNT);
    assert_eq!(config.max_conflict_count, 1_000);
    assert_eq!(config.max_conflict_cost, 550_000_000_000); // 1x block cost

    // Fee / RBF (Chia: MEMPOOL_MIN_FEE_INCREASE = 10M mojos)
    assert_eq!(config.min_rbf_fee_bump, MIN_RBF_FEE_BUMP);
    assert_eq!(config.min_rbf_fee_bump, 10_000_000);

    // Full mempool minimum FPC (Chia: nonzero_fee_minimum_fpc = 5)
    assert_eq!(
        config.full_mempool_min_fpc_scaled,
        FULL_MEMPOOL_MIN_FPC_SCALED
    );
    assert_eq!(config.full_mempool_min_fpc_scaled, 5 * FPC_SCALE);

    // Virtual cost penalty (Chia: SPEND_PENALTY_COST = 500_000)
    assert_eq!(config.spend_penalty_cost, SPEND_PENALTY_COST);
    assert_eq!(config.spend_penalty_cost, 500_000);

    // Expiry protection (DIG uses 100 blocks; Chia uses 48 blocks/900s)
    assert_eq!(
        config.expiry_protection_blocks,
        DEFAULT_EXPIRY_PROTECTION_BLOCKS
    );
    assert_eq!(config.expiry_protection_blocks, 100);

    // CPFP depth limit (dig-mempool extension, not in Chia)
    assert_eq!(config.max_dependency_depth, DEFAULT_MAX_DEPENDENCY_DEPTH);
    assert_eq!(config.max_dependency_depth, 25);

    // Seen cache (dig-mempool extension)
    assert_eq!(config.max_seen_cache_size, DEFAULT_SEEN_CACHE_SIZE);
    assert_eq!(config.max_seen_cache_size, 10_000);

    // Fee estimation (dig-mempool extension)
    assert_eq!(config.fee_estimator_window, DEFAULT_FEE_ESTIMATOR_WINDOW);
    assert_eq!(config.fee_estimator_window, 100);
    assert_eq!(config.fee_estimator_buckets, DEFAULT_FEE_ESTIMATOR_BUCKETS);
    assert_eq!(config.fee_estimator_buckets, 50);

    // Feature flags (both enabled by default, matching Chia behavior)
    assert!(config.enable_singleton_ff);
    assert!(config.enable_identical_spend_dedup);
}

/// Test: Builder chaining works — multiple with_* calls on the same config.
///
/// Proves API-003 acceptance criterion: "Builder methods return Self for
/// chaining." The fluent API allows `Config::default().with_a(x).with_b(y)`.
#[test]
fn vv_req_api_003_builder_chaining() {
    let config = MempoolConfig::default()
        .with_max_total_cost(1_000_000)
        .with_min_rbf_fee_bump(5_000_000)
        .with_singleton_ff(false);

    assert_eq!(config.max_total_cost, 1_000_000);
    assert_eq!(config.min_rbf_fee_bump, 5_000_000);
    assert!(!config.enable_singleton_ff);
}

/// Test: Builder preserves fields that were not overridden.
///
/// Proves that calling one with_* method doesn't affect other fields.
/// This is critical for ergonomic usage where callers only override
/// the 1-2 parameters they care about.
#[test]
fn vv_req_api_003_builder_preserves_unset_fields() {
    let config = MempoolConfig::default().with_max_total_cost(999);

    // The overridden field has the new value
    assert_eq!(config.max_total_cost, 999);

    // All other fields retain their defaults
    assert_eq!(config.max_bundle_cost, 11_000_000_000);
    assert_eq!(config.max_pending_count, 3_000);
    assert_eq!(config.max_conflict_count, 1_000);
    assert_eq!(config.min_rbf_fee_bump, 10_000_000);
    assert_eq!(config.spend_penalty_cost, 500_000);
    assert_eq!(config.max_dependency_depth, 25);
    assert!(config.enable_singleton_ff);
    assert!(config.enable_identical_spend_dedup);
}

/// Test: All 17 with_* builder methods exist and compile.
///
/// Proves API-003 acceptance criterion: "Builder methods (with_*) are
/// available for all fields." Each method is called to verify it exists
/// and accepts the correct parameter type.
#[test]
fn vv_req_api_003_all_builder_methods_exist() {
    // Call every builder method to prove they exist and compile.
    // Each one consumes self and returns Self (fluent pattern).
    let _config = MempoolConfig::default()
        .with_max_total_cost(1)
        .with_max_bundle_cost(2)
        .with_max_spends_per_block(3)
        .with_max_pending_count(4)
        .with_max_pending_cost(5)
        .with_max_conflict_count(6)
        .with_max_conflict_cost(7)
        .with_min_rbf_fee_bump(8)
        .with_full_mempool_min_fpc_scaled(9)
        .with_spend_penalty_cost(10)
        .with_expiry_protection_blocks(11)
        .with_max_dependency_depth(12)
        .with_max_seen_cache_size(13)
        .with_fee_estimator_window(14)
        .with_fee_estimator_buckets(15)
        .with_singleton_ff(false)
        .with_identical_spend_dedup(false);
}

/// Test: Config integrates with Mempool::with_config().
///
/// Proves that a custom config is actually used by the Mempool.
/// The max_cost field in stats() should reflect the custom value.
/// This connects API-003 to API-001 (constructors use config).
#[test]
fn vv_req_api_003_config_used_by_mempool() {
    use dig_constants::DIG_TESTNET;
    use dig_mempool::Mempool;

    let config = MempoolConfig::default().with_max_total_cost(42_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    assert_eq!(mempool.stats().max_cost, 42_000);
}

/// Test: MempoolConfig is Clone.
///
/// Proves API-003 acceptance criterion: the struct derives Clone.
/// Cloned configs must have identical field values.
#[test]
fn vv_req_api_003_struct_is_clone() {
    let original = MempoolConfig::default().with_max_total_cost(12345);
    let cloned = original.clone();
    assert_eq!(cloned.max_total_cost, 12345);
    assert_eq!(cloned.max_bundle_cost, original.max_bundle_cost);
    assert_eq!(cloned.enable_singleton_ff, original.enable_singleton_ff);
}

/// Test: max_total_cost default is exactly L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER.
///
/// This is the most important default — it determines total mempool capacity.
/// L2_MAX_COST_PER_BLOCK = 550B (from dig-clvm config.rs:9)
/// MEMPOOL_BLOCK_BUFFER = 15 (from config.rs:9)
/// Result: 8,250,000,000,000
#[test]
fn vv_req_api_003_max_total_cost_default() {
    let config = MempoolConfig::default();
    assert_eq!(config.max_total_cost, 8_250_000_000_000);
}

/// Test: with_singleton_ff and with_enable_identical_spend_dedup exist.
///
/// These are the feature flag builder methods. The spec calls them
/// with_singleton_ff and with_identical_spend_dedup respectively.
/// Verify both naming conventions compile.
#[test]
fn vv_req_api_003_feature_flag_builders() {
    let config = MempoolConfig::default()
        .with_singleton_ff(false)
        .with_identical_spend_dedup(false);

    assert!(!config.enable_singleton_ff);
    assert!(!config.enable_identical_spend_dedup);

    // Re-enable
    let config2 = config
        .with_singleton_ff(true)
        .with_identical_spend_dedup(true);
    assert!(config2.enable_singleton_ff);
    assert!(config2.enable_identical_spend_dedup);
}
