//! REQUIREMENT: FEE-004 — Confirmed Block Recording (record_confirmed_block)
//!
//! Proves record_confirmed_block(height, bundles):
//! - Is publicly exported on Mempool
//! - Accepts height: u64 and bundles: &[ConfirmedBundleInfo]
//! - Applies exponential decay 0.998 to all bucket counters before recording
//! - Places each bundle into the correct fee-rate bucket
//! - Appends BlockFeeData to the rolling window
//! - Evicts oldest block_history when window is exceeded
//! - Is called automatically by on_new_block()
//! - Is callable manually for historical data seeding
//!
//! Reference: docs/requirements/domains/fee_estimation/specs/FEE-004.md

use dig_constants::DIG_TESTNET;
use dig_mempool::{ConfirmedBundleInfo, Mempool, MempoolConfig};

/// record_confirmed_block() is publicly accessible.
///
/// Proves FEE-004: "record_confirmed_block() is publicly exported on the Mempool struct."
#[test]
fn vv_req_fee_004_is_public() {
    let m = Mempool::new(DIG_TESTNET);
    // Calling it compiles → proves it's public.
    m.record_confirmed_block(1, &[]);
}

/// Single block recording: correct bucket incremented, BlockFeeData appended.
///
/// Proves FEE-004: "Places each confirmed bundle into the correct fee-rate bucket."
#[test]
fn vv_req_fee_004_single_block_recording() {
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
    assert_eq!(stats.history_len, 1, "one block recorded → history_len must be 1");

    // Exactly one bucket must have total_observed ≈ 1.0.
    let hot: Vec<_> = stats.bucket_totals.iter().filter(|&&t| t > 0.5).collect();
    assert_eq!(hot.len(), 1, "exactly one bucket must have data after one tx");
}

/// Empty bundles slice still causes decay and appends a BlockFeeData entry.
///
/// Proves FEE-004: "Empty `bundles` slice: still apply decay and record an empty BlockFeeData."
#[test]
fn vv_req_fee_004_empty_bundles_still_appends_block_data() {
    let m = Mempool::new(DIG_TESTNET);

    m.record_confirmed_block(5, &[]);

    let stats = m.fee_tracker_stats();
    assert_eq!(
        stats.history_len, 1,
        "empty bundles must still append a BlockFeeData entry"
    );
    // All buckets zero (no transactions placed).
    assert!(
        stats.bucket_totals.iter().all(|&t| t == 0.0),
        "no tx → all buckets must remain zero"
    );
}

/// Decay is applied per block call.
///
/// Proves FEE-004: "Applies exponential decay factor 0.998 to all bucket counters
/// before recording new data."
#[test]
fn vv_req_fee_004_decay_applied_per_block() {
    let m = Mempool::new(DIG_TESTNET);

    // Seed one tx in block 0.
    m.record_confirmed_block(
        0,
        &[ConfirmedBundleInfo {
            cost: 1_000_000,
            fee: 5_000_000,
            num_spends: 0,
        }],
    );

    let stats_before = m.fee_tracker_stats();
    let hot_idx = stats_before
        .bucket_totals
        .iter()
        .position(|&t| t > 0.5)
        .unwrap();
    let initial_total = stats_before.bucket_totals[hot_idx];

    // Record a second empty block → decay is applied, no new tx.
    m.record_confirmed_block(1, &[]);

    let stats_after = m.fee_tracker_stats();
    let after_total = stats_after.bucket_totals[hot_idx];

    // After one decay: total ≈ initial * 0.998.
    let expected = initial_total * 0.998;
    assert!(
        (after_total - expected).abs() < 0.001,
        "decay must reduce total_observed by factor 0.998: got {after_total}, expected {expected}"
    );
}

/// Window eviction: after window blocks, oldest entries are evicted.
///
/// Proves FEE-004: "Evicts oldest block_history entry when window is exceeded."
#[test]
fn vv_req_fee_004_window_eviction() {
    let config = MempoolConfig::default().with_fee_estimator_window(50);
    let m = Mempool::with_config(DIG_TESTNET, config);

    for h in 0u64..80 {
        m.record_confirmed_block(h, &[]);
    }

    let stats = m.fee_tracker_stats();
    assert_eq!(
        stats.history_len, 50,
        "window=50 must cap block_history at 50 entries after 80 blocks"
    );
}

/// Correct bucket placement: a bundle with known FPC lands in the expected bucket.
///
/// Proves FEE-004: "Places each confirmed bundle into the correct fee-rate bucket."
#[test]
fn vv_req_fee_004_correct_bucket_placement() {
    let m = Mempool::new(DIG_TESTNET);

    // Two bundles with very different fee rates: one very low, one very high.
    // They should land in different buckets.
    m.record_confirmed_block(
        1,
        &[
            ConfirmedBundleInfo {
                cost: 1_000_000,
                fee: 1, // extremely low fee
                num_spends: 0,
            },
            ConfirmedBundleInfo {
                cost: 1_000_000,
                fee: 1_000_000_000_000_000, // extremely high fee
                num_spends: 0,
            },
        ],
    );

    let stats = m.fee_tracker_stats();
    // Both bundles should land in different buckets (total across all = ~2).
    let total: f64 = stats.bucket_totals.iter().sum();
    assert!(
        (total - 2.0).abs() < 0.01,
        "two distinct-rate bundles should each land in a bucket: total={total}"
    );
    // At least two buckets should have non-trivial counts.
    let hot_count = stats.bucket_totals.iter().filter(|&&t| t > 0.5).count();
    assert!(
        hot_count >= 2,
        "two very different fee rates must land in different buckets: hot_count={hot_count}"
    );
}

/// Manual seeding: record_confirmed_block() works independently of on_new_block().
///
/// Proves FEE-004: "Callable manually for historical data seeding."
#[test]
fn vv_req_fee_004_manual_seeding_works() {
    let m = Mempool::new(DIG_TESTNET);

    // Seed 50 historical blocks directly — no on_new_block() involved.
    let bundle = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };
    for h in 0u64..50 {
        m.record_confirmed_block(h, &[bundle.clone()]);
    }

    let stats = m.fee_tracker_stats();
    assert_eq!(stats.history_len, 50, "manual seeding must accumulate block history");

    // estimate_fee_rate should now work (>= window/2 = 50 blocks).
    let result = m.estimate_fee_rate(1);
    assert!(
        result.is_some(),
        "estimate_fee_rate must return Some after sufficient manual seeding"
    );
}

/// on_new_block() integration: fee tracker is updated during block processing.
///
/// Proves FEE-004: "Called automatically by on_new_block()."
#[test]
fn vv_req_fee_004_on_new_block_integration() {
    use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};

    let m = Mempool::new(DIG_TESTNET);

    // Before any blocks, history is empty.
    assert_eq!(m.fee_tracker_stats().history_len, 0);

    // Call on_new_block with a confirmed_bundles slice.
    let bundle_info = ConfirmedBundleInfo {
        cost: 1_000_000,
        fee: 5_000_000,
        num_spends: 0,
    };

    m.on_new_block(1, 1000, &[], &[bundle_info]);

    let stats = m.fee_tracker_stats();
    assert_eq!(
        stats.history_len, 1,
        "on_new_block must call record_confirmed_block and update block_history"
    );
}
