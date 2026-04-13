//! Fee estimation module: FeeTracker, BlockFeeData, FeeTrackerStats.
//!
//! # Overview
//!
//! Provides:
//! - `FeeTracker` — internal bucket-based historical tracker (pub(crate))
//! - `BlockFeeData` — per-block fee summary, public for serialization + testing
//! - `FeeTrackerStats` — snapshot of FeeTracker state, public for testing
//!
//! # Spec Reference
//!
//! - [FEE-002](docs/requirements/domains/fee_estimation/specs/FEE-002.md)
//! - [FEE-003](docs/requirements/domains/fee_estimation/specs/FEE-003.md)
//! - [FEE-004](docs/requirements/domains/fee_estimation/specs/FEE-004.md)

pub(crate) mod tracker;

pub use tracker::BlockFeeData;
pub(crate) use tracker::FeeTracker;

/// Snapshot of `FeeTracker` state for testing and diagnostics.
///
/// Returned by `Mempool::fee_tracker_stats()`. Provides read-only access to
/// internal bucket data without exposing `FeeTracker` directly.
///
/// # Fields
///
/// | Field | Description |
/// |-------|-------------|
/// | `bucket_count` | Number of logarithmically spaced fee-rate buckets |
/// | `window` | Rolling window size (max blocks retained) |
/// | `history_len` | Number of blocks currently in `block_history` |
/// | `bucket_ranges` | `(lower, upper)` bounds for each bucket (scaled FPC) |
/// | `bucket_totals` | `total_observed` counter for each bucket |
/// | `bucket_confirmed_in_1` | `confirmed_in_1` counter for each bucket |
#[derive(Debug, Clone)]
pub struct FeeTrackerStats {
    /// Number of fee-rate buckets.
    pub bucket_count: usize,
    /// Rolling window size in blocks.
    pub window: usize,
    /// Number of blocks currently in block_history.
    pub history_len: usize,
    /// (lower, upper) bounds for each bucket (scaled FPC units).
    pub bucket_ranges: Vec<(u128, u128)>,
    /// total_observed per bucket (f64, includes exponential decay).
    pub bucket_totals: Vec<f64>,
    /// confirmed_in_1 per bucket (f64, includes exponential decay).
    pub bucket_confirmed_in_1: Vec<f64>,
}
