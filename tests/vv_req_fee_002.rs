//! REQUIREMENT: FEE-002 — FeeTracker Bucket-Based Tracker
//!
//! Proves FeeTracker:
//! - Has correct default construction (50 buckets, window=100, empty history)
//! - Supports custom bucket count via MempoolConfig
//! - Uses logarithmic bucket spacing (each successive bucket is wider)
//! - Enforces a bounded block history (rolling window eviction)
//! - Places transactions into the correct bucket (total_observed incremented)
//! - Tracks confirmations in confirmed_in_1 (default 1-block wait)
//! - Starts with all-zero state
//!
//! Reference: docs/requirements/domains/fee_estimation/specs/FEE-002.md

use dig_constants::DIG_TESTNET;
use dig_mempool::{ConfirmedBundleInfo, Mempool, MempoolConfig};

/// Default FeeTracker: 50 buckets, window=100, empty history.
///
/// Proves FEE-002: "Default bucket count is 50. Default window size is 100 blocks."
#[test]
fn vv_req_fee_002_default_construction() {
    let m = Mempool::new(DIG_TESTNET);
    let stats = m.fee_tracker_stats();
    assert_eq!(stats.bucket_count, 50, "default bucket count must be 50");
    assert_eq!(stats.window, 100, "default window must be 100");
    assert_eq!(stats.history_len, 0, "history must be empty on construction");
}

/// Custom bucket count is honoured.
///
/// Proves FEE-002: "Default bucket count is 50 (configurable)."
#[test]
fn vv_req_fee_002_custom_bucket_count() {
    let config = MempoolConfig::default().with_fee_estimator_buckets(20);
    let m = Mempool::with_config(DIG_TESTNET, config);
    let stats = m.fee_tracker_stats();
    assert_eq!(stats.bucket_count, 20, "custom bucket count must be respected");
}

/// Buckets are logarithmically spaced — each is wider than the previous.
///
/// Proves FEE-002: "Buckets are logarithmically spaced across the fee-rate range."
/// In log-spaced buckets each covers a multiplicatively equal range, so absolute
/// width increases monotonically.
#[test]
fn vv_req_fee_002_bucket_boundaries_logarithmic() {
    let m = Mempool::new(DIG_TESTNET);
    let stats = m.fee_tracker_stats();
    let ranges = &stats.bucket_ranges;

    assert!(ranges.len() >= 2, "must have at least 2 buckets");

    // Every bucket must have upper > lower.
    for (i, &(lower, upper)) in ranges.iter().enumerate() {
        assert!(
            upper > lower,
            "bucket {i}: upper ({upper}) must exceed lower ({lower})"
        );
    }

    // Successive bucket widths must be non-decreasing (log-spacing means
    // each successive bucket covers a larger absolute range).
    let widths: Vec<u128> = ranges.iter().map(|&(lo, hi)| hi - lo).collect();
    for i in 1..widths.len() {
        assert!(
            widths[i] >= widths[i - 1],
            "bucket {i} width ({}) must be >= bucket {} width ({})",
            widths[i],
            i - 1,
            widths[i - 1]
        );
    }
}

/// Block history is bounded at `window` blocks.
///
/// Proves FEE-002: "The block_history circular buffer retains data for the most
/// recent `window` blocks. When full, the oldest entry is evicted."
#[test]
fn vv_req_fee_002_block_history_bounded() {
    let config = MempoolConfig::default().with_fee_estimator_window(100);
    let m = Mempool::with_config(DIG_TESTNET, config);

    // Record 150 blocks with empty bundle lists.
    for h in 0u64..150 {
        m.record_confirmed_block(h, &[]);
    }

    let stats = m.fee_tracker_stats();
    assert_eq!(
        stats.history_len, 100,
        "window=100 must cap block_history at 100 entries after 150 blocks"
    );
}

/// A transaction with a known FPC lands in exactly one bucket.
///
/// Proves FEE-002: "Each `FeeBucket` tracks … `total_observed`."
#[test]
fn vv_req_fee_002_fee_rate_placement() {
    let m = Mempool::new(DIG_TESTNET);

    // Before recording: all totals must be zero.
    let pre = m.fee_tracker_stats();
    assert!(
        pre.bucket_totals.iter().all(|&t| t == 0.0),
        "all bucket totals must be zero before any recording"
    );

    // Record one block with one bundle (fee=5, cost=1M → fpc_scaled = 5e12/1M = 5_000_000).
    m.record_confirmed_block(
        1,
        &[ConfirmedBundleInfo {
            cost: 1_000_000,
            fee: 5_000_000,
            num_spends: 0,
        }],
    );

    let post = m.fee_tracker_stats();
    let total: f64 = post.bucket_totals.iter().sum();
    // After decay (one call) and one placement: sum ≈ 1.0.
    assert!(
        (total - 1.0).abs() < 0.01,
        "total_observed across all buckets should be ~1 after 1 tx: {total}"
    );
    // Exactly one bucket should have a non-trivial count.
    let hot_buckets: usize = post.bucket_totals.iter().filter(|&&t| t > 0.5).count();
    assert_eq!(hot_buckets, 1, "exactly one bucket must have total_observed > 0");
}

/// Confirmed tx increments confirmed_in_1 for its bucket (default 1-block wait).
///
/// Proves FEE-002: "Each `FeeBucket` tracks `confirmed_in_1` … confirmed within 1 block."
#[test]
fn vv_req_fee_002_confirmation_tracking() {
    let m = Mempool::new(DIG_TESTNET);

    m.record_confirmed_block(
        1,
        &[ConfirmedBundleInfo {
            cost: 1_000_000,
            fee: 5_000_000,
            num_spends: 0,
        }],
    );

    let stats = m.fee_tracker_stats();
    let hot = stats
        .bucket_totals
        .iter()
        .position(|&t| t > 0.5)
        .expect("at least one bucket must be non-zero");

    let confirmed = stats.bucket_confirmed_in_1[hot];
    let total = stats.bucket_totals[hot];
    assert!(
        (confirmed - total).abs() < 0.001,
        "confirmed_in_1 must equal total_observed for default 1-block wait: {confirmed} vs {total}"
    );
}

/// Fresh tracker: all bucket counters are zero.
///
/// Proves FEE-002: "FeeTracker struct exists … All fields start at zero."
#[test]
fn vv_req_fee_002_empty_tracker_state() {
    let m = Mempool::new(DIG_TESTNET);
    let stats = m.fee_tracker_stats();
    assert!(
        stats.bucket_totals.iter().all(|&t| t == 0.0),
        "total_observed must be 0 for all buckets on fresh tracker"
    );
    assert!(
        stats.bucket_confirmed_in_1.iter().all(|&c| c == 0.0),
        "confirmed_in_1 must be 0 for all buckets on fresh tracker"
    );
    assert_eq!(stats.history_len, 0, "history must be empty on fresh tracker");
}
