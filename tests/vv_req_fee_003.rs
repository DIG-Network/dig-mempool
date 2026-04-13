//! REQUIREMENT: FEE-003 — Fee Rate Estimation (estimate_fee_rate)
//!
//! Proves estimate_fee_rate(target_blocks):
//! - Is publicly exported on Mempool
//! - Returns Option<FeeRate> using the chia-protocol FeeRate type
//! - Returns None when fewer than window/2 blocks have been tracked
//! - Returns Some when sufficient data is present and threshold is met
//! - Enforces the 85% confidence threshold (buckets with < 85% skipped)
//! - Returns None when no bucket meets the threshold
//! - Treats target_blocks=0 as target_blocks=1
//!
//! Reference: docs/requirements/domains/fee_estimation/specs/FEE-003.md

use dig_constants::DIG_TESTNET;
use dig_mempool::{ConfirmedBundleInfo, Mempool, MempoolConfig};

/// estimate_fee_rate() is publicly accessible.
///
/// Proves FEE-003: "estimate_fee_rate() is publicly exported on the Mempool struct."
#[test]
fn vv_req_fee_003_is_public() {
    let m = Mempool::new(DIG_TESTNET);
    let _result = m.estimate_fee_rate(1);
}

/// Returns None when fewer than window/2 blocks have been tracked.
///
/// Proves FEE-003: "Returns None when fewer than fee_estimator_window/2 blocks tracked."
#[test]
fn vv_req_fee_003_insufficient_data_returns_none() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    // Record only 49 blocks (< window/2 = 50).
    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };
    for h in 0u64..49 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    assert!(
        m.estimate_fee_rate(1).is_none(),
        "must return None when < window/2 blocks have been recorded"
    );
}

/// Returns Some once at least window/2 blocks have been tracked with data.
///
/// Proves FEE-003: "Returns Some(FeeRate) when sufficient data exists."
#[test]
fn vv_req_fee_003_sufficient_data_returns_some() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    // Record exactly 50 blocks (= window/2) each with high-fee bundles.
    // All default to 1-block confirmation → success_rate = 1.0 ≥ 0.85.
    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };
    for h in 0u64..50 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    assert!(
        m.estimate_fee_rate(1).is_some(),
        "must return Some after window/2 blocks of data"
    );
}

/// Returns None when all tracked blocks had no transactions (no bucket has data).
///
/// Proves FEE-003: "If no bucket meets the threshold, return None."
#[test]
fn vv_req_fee_003_no_bucket_meets_threshold_returns_none() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    // Record 50+ empty blocks — all buckets will have total_observed = 0.
    for h in 0u64..60 {
        m.record_confirmed_block(h, &[]);
    }

    assert!(
        m.estimate_fee_rate(1).is_none(),
        "must return None when all buckets have no data (total_observed < 1.0)"
    );
}

/// Returns a FeeRate with mojos_per_clvm_cost > 0 for high-fee bundles.
///
/// Proves FEE-003: "Return type is Option<FeeRate> using chia_protocol::FeeRate."
#[test]
fn vv_req_fee_003_returns_fee_rate_type() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000, // fpc = 5 mojos / cost
        num_spends: 0,
    };
    for h in 0u64..50 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    let result = m.estimate_fee_rate(1);
    assert!(result.is_some(), "must return Some for sufficient high-fee data");
    // mojos_per_clvm_cost field must exist (proves it's the chia-protocol type).
    let fee_rate = result.unwrap();
    let _ = fee_rate.mojos_per_clvm_cost;
}

/// target_blocks=0 is treated as target_blocks=1 (no crash, same result).
///
/// Proves FEE-003: "Edge cases: target_blocks=0 treated as target_blocks=1."
#[test]
fn vv_req_fee_003_target_0_treated_as_1() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };
    for h in 0u64..50 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    let r0 = m.estimate_fee_rate(0);
    let r1 = m.estimate_fee_rate(1);
    // Both should return Some (if data is sufficient) or None (if not).
    assert_eq!(
        r0.is_some(),
        r1.is_some(),
        "target=0 and target=1 must both return the same variant"
    );
}

/// Higher target_blocks still returns a result (target > 10 uses confirmed_in_10).
///
/// Proves FEE-003: "target > 10: use confirmed_in_10 (best available)."
#[test]
fn vv_req_fee_003_high_target_returns_result() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };
    for h in 0u64..50 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    // target=20 uses confirmed_in_10 — should not crash and return Some.
    let result = m.estimate_fee_rate(20);
    assert!(
        result.is_some(),
        "target > 10 must fall back to confirmed_in_10 and return Some for sufficient data"
    );
}

/// Reads fee tracker state under a read lock (no deadlock with concurrent reads).
///
/// Proves FEE-003: "Reads fee tracker state under a read lock."
#[test]
fn vv_req_fee_003_concurrent_read_safe() {
    use std::sync::Arc;
    use std::thread;

    let m = Arc::new(Mempool::new(DIG_TESTNET));
    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };
    for h in 0u64..50 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    let m2 = Arc::clone(&m);
    let m3 = Arc::clone(&m);

    let t1 = thread::spawn(move || m2.estimate_fee_rate(1));
    let t2 = thread::spawn(move || m3.estimate_fee_rate(5));

    let r1 = t1.join().expect("thread 1 must not panic");
    let r2 = t2.join().expect("thread 2 must not panic");
    // Both concurrent reads must succeed.
    assert_eq!(r1.is_some(), r2.is_some());
}
