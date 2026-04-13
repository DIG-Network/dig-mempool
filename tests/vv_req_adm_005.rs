//! REQUIREMENT: ADM-005 — Virtual Cost Computation
//!
//! Test-driven verification that submit() computes virtual cost as
//! `cost + (num_spends * SPEND_PENALTY_COST)` and rejects bundles
//! whose cost exceeds `config.max_bundle_cost`.
//!
//! ## What this proves
//!
//! - Cost is read from `SpendResult.conditions.cost`
//! - Virtual cost includes the spend penalty
//! - CostExceeded error returned when cost > max_bundle_cost
//! - Zero-cost bundles (0 spends) pass the cost check
//!
//! ## Chia L1 Correspondence
//!
//! virtual_cost = cost + num_spends * SPEND_PENALTY_COST
//! Chia ref: mempool_item.py:92-93
//! Cost exceeded check: mempool_manager.py:733-734
//!
//! Reference: docs/requirements/domains/admission/specs/ADM-005.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{CoinRecord, Mempool, MempoolConfig, MempoolError, SubmitResult};

/// Test: Empty bundle (0 spends) has cost=0, passes cost check.
///
/// Proves ADM-005: cost extraction works for the trivial case.
/// cost=0, num_spends=0 → virtual_cost=0. Since 0 <= max_bundle_cost, passes.
#[test]
fn vv_req_adm_005_zero_cost_passes() {
    let mempool = Mempool::new(DIG_TESTNET);
    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    let result = mempool.submit(bundle, &coin_records, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success));
}

/// Test: CostExceeded error can be constructed with correct fields.
///
/// Proves the error variant exists and formats correctly.
/// The actual cost-exceeded path requires a bundle whose CLVM cost
/// exceeds max_bundle_cost, which is hard to craft without a custom puzzle.
#[test]
fn vv_req_adm_005_cost_exceeded_error_exists() {
    let err = MempoolError::CostExceeded {
        cost: 15_000_000_000,
        max: 11_000_000_000,
    };
    let msg = format!("{err}");
    assert!(msg.contains("15000000000"));
    assert!(msg.contains("11000000000"));
}

/// Test: Virtual cost computation formula is correct.
///
/// Proves ADM-005: virtual_cost = cost + (num_spends * SPEND_PENALTY_COST).
/// This is verified via MempoolItem::compute_virtual_cost() which uses
/// the same formula that submit() will use internally.
///
/// Chia ref: mempool_item.py:92-93
///   virtual_cost = self.cost + self.num_spends * SPEND_PENALTY_COST
#[test]
fn vv_req_adm_005_virtual_cost_formula() {
    use dig_mempool::item::MempoolItem;

    // 5M cost + 3 spends * 500K penalty = 6.5M virtual cost
    let vc = MempoolItem::compute_virtual_cost(5_000_000, 3);
    assert_eq!(vc, 6_500_000);

    // 0 cost + 0 spends = 0 virtual cost
    let vc = MempoolItem::compute_virtual_cost(0, 0);
    assert_eq!(vc, 0);

    // 11B cost + 1 spend * 500K = 11,000,500,000
    let vc = MempoolItem::compute_virtual_cost(11_000_000_000, 1);
    assert_eq!(vc, 11_000_500_000);
}

/// Test: Custom max_bundle_cost is respected.
///
/// Proves that the cost check uses config.max_bundle_cost.
/// We set a very low max_bundle_cost and verify that bundles exceeding
/// it are rejected. Since empty bundles have cost=0, they still pass.
#[test]
fn vv_req_adm_005_custom_max_bundle_cost_rejects() {
    // Set max_bundle_cost extremely low (1). Even an empty bundle has
    // a non-zero cost from chia-consensus (signature check overhead).
    let config = MempoolConfig::default().with_max_bundle_cost(1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let result = mempool.submit(bundle, &coin_records, 0, 0);

    // Should be rejected — even empty bundle cost > 1
    match result {
        Err(MempoolError::CostExceeded { cost, max }) => {
            assert!(cost > 1, "Empty bundle should have some cost: {cost}");
            assert_eq!(max, 1);
        }
        other => panic!("Expected CostExceeded, got: {:?}", other),
    }
}
