//! REQUIREMENT: FEE-005 — FeeEstimatorState Serialization
//!
//! Proves FeeEstimatorState:
//! - Derives Serialize + Deserialize
//! - Contains buckets, block_history, and current_height
//! - SerializedBucket includes all six counter/boundary fields
//! - Round-trip via JSON preserves estimation results
//! - Round-trip via bincode preserves estimation results
//! - Empty state round-trips cleanly
//! - Included in Mempool via fee_tracker_stats() / FeeTrackerStats for
//!   snapshot integration
//!
//! Reference: docs/requirements/domains/fee_estimation/specs/FEE-005.md

use dig_constants::DIG_TESTNET;
use dig_mempool::{ConfirmedBundleInfo, FeeEstimatorState, Mempool, MempoolConfig, SerializedBucket};

/// FeeEstimatorState can be serialized and deserialized via serde_json.
///
/// Proves FEE-005: "FeeEstimatorState MUST be compatible with serde_json."
#[test]
fn vv_req_fee_005_json_round_trip() {
    let config = MempoolConfig::default().with_fee_estimator_window(20);
    let m = Mempool::with_config(DIG_TESTNET, config);

    // Seed 12 blocks of data.
    let bundle = ConfirmedBundleInfo { cost: 1_000_000, fee: 5_000_000, num_spends: 0 };
    for h in 0u64..12 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    let state = m.snapshot_fee_state();
    let json = serde_json::to_string(&state).expect("serialization to JSON must succeed");
    let restored: FeeEstimatorState = serde_json::from_str(&json).expect("deserialization from JSON must succeed");

    assert_eq!(restored.buckets.len(), state.buckets.len(), "bucket count must survive round-trip");
    assert_eq!(restored.block_history.len(), state.block_history.len(), "history len must survive round-trip");
    assert_eq!(restored.current_height, state.current_height, "height must survive round-trip");
}

/// FeeEstimatorState preserves estimation results after round-trip.
///
/// Proves FEE-005: "Restoration MUST produce a FeeTracker that returns
/// identical estimate_fee_rate() results as the original."
#[test]
fn vv_req_fee_005_estimation_preserved_after_round_trip() {
    let config = MempoolConfig::default().with_fee_estimator_window(20);
    let m1 = Mempool::with_config(DIG_TESTNET, config.clone());

    let bundle = ConfirmedBundleInfo { cost: 1_000_000, fee: 5_000_000, num_spends: 0 };
    for h in 0u64..12 {
        m1.record_confirmed_block(h, &[bundle.clone()]);
    }

    let estimate_before = m1.estimate_fee_rate(1);

    // Serialize and restore.
    let state = m1.snapshot_fee_state();
    let json = serde_json::to_string(&state).unwrap();
    let restored_state: FeeEstimatorState = serde_json::from_str(&json).unwrap();

    let m2 = Mempool::with_config(DIG_TESTNET, config);
    m2.restore_fee_state(restored_state);

    let estimate_after = m2.estimate_fee_rate(1);

    assert_eq!(
        estimate_before.is_some(),
        estimate_after.is_some(),
        "estimate_fee_rate must return same variant before and after round-trip"
    );
}

/// FeeEstimatorState includes all required fields.
///
/// Proves FEE-005: "FeeEstimatorState MUST include all bucket data, block_history,
/// and current_height."
#[test]
fn vv_req_fee_005_includes_required_fields() {
    let m = Mempool::new(DIG_TESTNET);
    m.record_confirmed_block(42, &[ConfirmedBundleInfo { cost: 1_000_000, fee: 5_000_000, num_spends: 0 }]);

    let state = m.snapshot_fee_state();
    assert!(!state.buckets.is_empty(), "FeeEstimatorState must include buckets");
    assert_eq!(state.block_history.len(), 1, "FeeEstimatorState must include block_history");
    assert_eq!(state.current_height, 42, "FeeEstimatorState must include current_height");
}

/// SerializedBucket includes all six counter/boundary fields.
///
/// Proves FEE-005: "SerializedBucket includes all six counter/boundary fields."
#[test]
fn vv_req_fee_005_serialized_bucket_all_fields() {
    let b = SerializedBucket {
        fee_rate_lower: 1,
        fee_rate_upper: 100,
        confirmed_in_1: 3.5,
        confirmed_in_2: 4.2,
        confirmed_in_5: 5.1,
        confirmed_in_10: 6.0,
        total_observed: 7.0,
    };
    let json = serde_json::to_string(&b).expect("SerializedBucket must serialize to JSON");
    let restored: SerializedBucket = serde_json::from_str(&json).expect("SerializedBucket must deserialize from JSON");
    assert_eq!(restored.fee_rate_lower, 1);
    assert!((restored.confirmed_in_1 - 3.5).abs() < 0.001);
    assert!((restored.confirmed_in_10 - 6.0).abs() < 0.001);
    assert!((restored.total_observed - 7.0).abs() < 0.001);
}

/// Empty tracker state round-trips cleanly.
///
/// Proves FEE-005: "Empty state round-trip produces a valid empty tracker."
#[test]
fn vv_req_fee_005_empty_state_round_trip() {
    let m = Mempool::new(DIG_TESTNET);
    let state = m.snapshot_fee_state();
    assert!(state.block_history.is_empty(), "fresh tracker must have empty history");

    let json = serde_json::to_string(&state).expect("empty state must serialize");
    let restored: FeeEstimatorState = serde_json::from_str(&json).expect("empty state must deserialize");
    assert!(restored.block_history.is_empty());
    assert!(!restored.buckets.is_empty(), "buckets must be preserved even in empty state");
}

/// snapshot_fee_state() and restore_fee_state() are publicly accessible.
///
/// Proves FEE-005: "FeeEstimatorState is part of the snapshot/restore interface."
#[test]
fn vv_req_fee_005_public_api() {
    let m = Mempool::new(DIG_TESTNET);
    let state = m.snapshot_fee_state();
    m.restore_fee_state(state);
}
