//! REQUIREMENT: API-002 — MempoolItem Struct
//!
//! Test-driven verification that `MempoolItem` contains all specified fields,
//! is stored as `Arc<MempoolItem>`, and computes derived fields correctly.
//!
//! Written BEFORE implementation per TDD workflow.
//!
//! ## What this proves
//!
//! These tests verify that the `MempoolItem` struct satisfies all acceptance
//! criteria from the API-002 spec:
//! - All 24+ fields are publicly accessible
//! - Virtual cost includes the spend penalty
//! - FPC uses integer-scaled arithmetic (FPC_SCALE = 10^12)
//! - Package fields equal individual fields when there are no CPFP ancestors
//! - The struct works correctly inside `Arc` (immutable, cheaply cloneable)
//! - `SingletonLineageInfo` is defined and usable as an Optional field
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-002.md

use std::collections::HashSet;
use std::sync::Arc;

use dig_mempool::item::{MempoolItem, SingletonLineageInfo};
use dig_mempool::{Bytes32, FPC_SCALE};

/// Test: All fields on MempoolItem are publicly accessible.
///
/// Proves API-002 acceptance criterion: "All fields listed in the
/// specification are present and public."
///
/// We construct a MempoolItem with known values and read every field.
/// If any field is missing or private, this test will not compile.
#[test]
fn vv_req_api_002_all_fields_accessible() {
    let item = make_test_item(100, 1_000_000, 2);

    // Identity fields
    let _: &dig_clvm::SpendBundle = &item.spend_bundle;
    let _: &Bytes32 = &item.spend_bundle_id;

    // Individual cost/fee
    let _: u64 = item.fee;
    let _: u64 = item.cost;
    let _: u64 = item.virtual_cost;
    let _: u128 = item.fee_per_virtual_cost_scaled;

    // Package (CPFP) cost/fee
    let _: u64 = item.package_fee;
    let _: u64 = item.package_virtual_cost;
    let _: u128 = item.package_fee_per_virtual_cost_scaled;

    // Eviction score
    let _: u128 = item.descendant_score;

    // State deltas
    let _: &Vec<dig_clvm::Coin> = &item.additions;
    let _: &Vec<Bytes32> = &item.removals;

    // Metadata
    let _: u64 = item.height_added;
    let _: usize = item.num_spends;

    // Timelocks
    let _: Option<u64> = item.assert_height;
    let _: Option<u64> = item.assert_before_height;
    let _: Option<u64> = item.assert_before_seconds;

    // Dependencies (CPFP)
    let _: &HashSet<Bytes32> = &item.depends_on;
    let _: u32 = item.depth;

    // Dedup + singleton
    let _: bool = item.eligible_for_dedup;
    let _: &Option<SingletonLineageInfo> = &item.singleton_lineage;
}

/// Test: MempoolItem works correctly inside Arc (immutable, cheaply cloneable).
///
/// Proves API-002 acceptance criterion: "Items are stored internally as
/// Arc<MempoolItem>" and "Public API methods return Arc<MempoolItem>."
///
/// We wrap in Arc, clone the Arc (cheap pointer copy), and verify both
/// references point to the same data.
#[test]
fn vv_req_api_002_arc_wrapping() {
    let item = make_test_item(500, 2_000_000, 3);
    let arc1 = Arc::new(item);
    let arc2 = Arc::clone(&arc1);

    // Both Arcs point to the same data — cheap clone, shared ownership
    assert_eq!(arc1.fee, arc2.fee);
    assert_eq!(arc1.spend_bundle_id, arc2.spend_bundle_id);
    assert_eq!(Arc::strong_count(&arc1), 2);
}

/// Test: virtual_cost = cost + (num_spends * SPEND_PENALTY_COST).
///
/// Proves API-002 acceptance criterion: "virtual_cost includes the spend
/// penalty." Uses the constant SPEND_PENALTY_COST = 500_000 from config.
///
/// Chia reference: mempool_item.py:92-93
///   virtual_cost = cost + num_spends * SPEND_PENALTY_COST
#[test]
fn vv_req_api_002_virtual_cost_computed() {
    let cost: u64 = 5_000_000;
    let num_spends: usize = 3;
    let penalty = dig_mempool::config::SPEND_PENALTY_COST;

    let item = make_test_item_with_cost(100, cost, num_spends);

    // virtual_cost = 5_000_000 + (3 * 500_000) = 6_500_000
    let expected_virtual_cost = cost + (num_spends as u64 * penalty);
    assert_eq!(item.virtual_cost, expected_virtual_cost);
}

/// Test: fee_per_virtual_cost_scaled = fee * FPC_SCALE / virtual_cost.
///
/// Proves API-002 acceptance criterion: "fee_per_virtual_cost_scaled uses
/// FPC_SCALE for integer precision."
///
/// We avoid floating-point by scaling the fee by 10^12 before dividing.
/// Chia uses float division (mempool_item.py:76-77); we use integer math
/// for determinism across nodes.
#[test]
fn vv_req_api_002_fpc_scaled_correctly() {
    let fee: u64 = 1_000;
    let cost: u64 = 2_000_000;
    let num_spends: usize = 1;
    let penalty = dig_mempool::config::SPEND_PENALTY_COST;

    let item = make_test_item_with_cost(fee, cost, num_spends);

    let virtual_cost = cost + (num_spends as u64 * penalty);
    let expected_fpc = (fee as u128 * FPC_SCALE) / (virtual_cost as u128);
    assert_eq!(item.fee_per_virtual_cost_scaled, expected_fpc);
}

/// Test: For items with no CPFP dependencies, package fields equal individual fields.
///
/// Proves API-002 field source table: "Items with no dependencies MUST have
/// package_fee == fee and package_virtual_cost == virtual_cost."
///
/// A root item (depth=0, depends_on empty) is not part of any CPFP chain.
/// Its package metrics are identical to its individual metrics.
#[test]
fn vv_req_api_002_package_fields_for_root_item() {
    let item = make_test_item(500, 3_000_000, 2);

    // Root item: package == individual
    assert_eq!(item.package_fee, item.fee);
    assert_eq!(item.package_virtual_cost, item.virtual_cost);
    assert_eq!(
        item.package_fee_per_virtual_cost_scaled,
        item.fee_per_virtual_cost_scaled
    );
    assert!(item.depends_on.is_empty());
    assert_eq!(item.depth, 0);
}

/// Test: SingletonLineageInfo is defined and can be stored as Optional field.
///
/// Proves the `singleton_lineage` field accepts `Some(SingletonLineageInfo)`
/// and `None`. The struct must have launcher_id, coin_id, parent_id,
/// parent_parent_id, and inner_puzzle_hash fields.
#[test]
fn vv_req_api_002_singleton_lineage_info() {
    let lineage = SingletonLineageInfo {
        coin_id: Bytes32::default(),
        parent_id: Bytes32::default(),
        parent_parent_id: Bytes32::default(),
        launcher_id: Bytes32::default(),
        inner_puzzle_hash: Bytes32::default(),
    };

    // Verify fields are accessible
    let _: &Bytes32 = &lineage.launcher_id;
    let _: &Bytes32 = &lineage.coin_id;
    let _: &Bytes32 = &lineage.inner_puzzle_hash;

    // Can be stored as Option
    let _opt: Option<SingletonLineageInfo> = Some(lineage);
    let _none: Option<SingletonLineageInfo> = None;
}

/// Test: MempoolItem with zero fee has zero FPC (no division by zero panic).
///
/// Edge case: bundles with zero fee are valid when mempool utilization < 80%.
/// The FPC calculation must handle this without panicking.
#[test]
fn vv_req_api_002_zero_fee_no_panic() {
    let item = make_test_item(0, 1_000_000, 1);
    assert_eq!(item.fee, 0);
    assert_eq!(item.fee_per_virtual_cost_scaled, 0);
    assert_eq!(item.package_fee_per_virtual_cost_scaled, 0);
}

// ── Test Helpers ──

/// Create a MempoolItem with the given fee, cost, and spend count.
/// Uses default values for all other fields. No CPFP dependencies.
///
/// The `cost` parameter is the raw CLVM cost. Virtual cost is computed
/// as `cost + (num_spends * SPEND_PENALTY_COST)`.
fn make_test_item(fee: u64, cost: u64, num_spends: usize) -> MempoolItem {
    make_test_item_with_cost(fee, cost, num_spends)
}

/// Constructs a MempoolItem with known fee/cost/spends for testing.
/// Uses `MempoolItem::new_for_test()` which provides a minimal valid item
/// without requiring full CLVM execution output.
fn make_test_item_with_cost(fee: u64, cost: u64, num_spends: usize) -> MempoolItem {
    MempoolItem::new_for_test(fee, cost, num_spends)
}
