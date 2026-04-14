//! FeeTracker: bucket-based historical fee data for rate estimation.
//!
//! Internal data structure — not part of the public API. Exposed only through
//! `Mempool::record_confirmed_block()` and `Mempool::estimate_fee_rate()`.
//!
//! # Architecture
//!
//! Maintains logarithmically spaced fee-rate buckets and a bounded circular
//! buffer of per-block confirmation data. Inspired by Bitcoin Core's
//! `CBlockPolicyEstimator` and Chia's `BitcoinFeeEstimator`.
//!
//! # Spec Reference
//!
//! - [FEE-002](../../docs/requirements/domains/fee_estimation/specs/FEE-002.md)
//! - [FEE-003](../../docs/requirements/domains/fee_estimation/specs/FEE-003.md)
//! - [FEE-004](../../docs/requirements/domains/fee_estimation/specs/FEE-004.md)

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::config::FPC_SCALE;
use crate::item::MempoolItem;
use crate::submit::ConfirmedBundleInfo;

// ── Constants ──

/// Minimum fee rate for the lowest bucket (absolute minimum, 1 scaled unit).
const MIN_FPC_SCALED: u128 = 1;

/// Maximum fee rate for the highest bucket.
/// = FPC_SCALE * 1_000_000 = 10^18.
/// Covers fee rates up to 1_000_000 mojos per CLVM cost unit.
const MAX_FPC_SCALED: u128 = 1_000_000_000_000_000_000; // 10^18

/// Exponential decay factor applied once per block to all bucket counters.
/// `0.998^100 ≈ 0.818` — data from 100 blocks ago retains ~82% weight.
const DECAY_FACTOR: f64 = 0.998;

/// Minimum success-rate threshold for bucket selection in `estimate_fee_rate`.
const CONFIDENCE_THRESHOLD: f64 = 0.85;

// ── Internal bucket type ──

/// One logarithmically spaced fee-rate bucket.
///
/// Tracks how many transactions at this fee-rate tier were confirmed within
/// N blocks, plus the total number of transactions observed.
struct FeeBucket {
    /// Lower bound of this bucket's fee rate (scaled FPC).
    fee_rate_lower: u128,
    /// Upper bound of this bucket's fee rate (scaled FPC).
    fee_rate_upper: u128,
    /// Transactions confirmed within 1 block.
    confirmed_in_1: f64,
    /// Transactions confirmed within 2 blocks.
    confirmed_in_2: f64,
    /// Transactions confirmed within 5 blocks.
    confirmed_in_5: f64,
    /// Transactions confirmed within 10 blocks.
    confirmed_in_10: f64,
    /// Total transactions observed at this fee rate.
    total_observed: f64,
}

// ── Public per-block summary ──

/// Fee statistics for a single confirmed block.
///
/// Appended to `FeeTracker::block_history` for each call to
/// `record_confirmed_block()`. Oldest entries are evicted when the rolling
/// window is full.
///
/// See: [FEE-002](docs/requirements/domains/fee_estimation/specs/FEE-002.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockFeeData {
    /// Block height this data was recorded for.
    pub height: u64,
    /// Minimum FPC (scaled) among included transactions (0 if none).
    #[serde(with = "serde_u128_str")]
    pub min_fpc_included: u128,
    /// Maximum FPC (scaled) among included transactions (0 if none).
    #[serde(with = "serde_u128_str")]
    pub max_fpc_included: u128,
    /// Median FPC (scaled) among included transactions (0 if none).
    #[serde(with = "serde_u128_str")]
    pub median_fpc: u128,
    /// Number of confirmed transactions recorded in this block.
    pub num_transactions: usize,
}

// ── serde helper for u128 as decimal string ──
//
// serde_json does not reliably round-trip u128 as a JSON number across all
// implementations. Encoding as a decimal string is universally safe.
mod serde_u128_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(val: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&val.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// ── FEE-005: FeeEstimatorState serialization ──

/// Serializable snapshot of a single fee-rate bucket's statistics.
///
/// Mirrors `FeeBucket` with all fields public and serde-derived.
/// Stored as `FeeEstimatorState::buckets` for persistence.
///
/// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedBucket {
    #[serde(with = "serde_u128_str")]
    pub fee_rate_lower: u128,
    #[serde(with = "serde_u128_str")]
    pub fee_rate_upper: u128,
    pub confirmed_in_1: f64,
    pub confirmed_in_2: f64,
    pub confirmed_in_5: f64,
    pub confirmed_in_10: f64,
    pub total_observed: f64,
}

/// Serializable snapshot of the complete `FeeTracker` state.
///
/// Enables the fee estimator to survive mempool restarts without losing
/// historical data. Include in `MempoolSnapshot::fee_estimator_state`.
///
/// Restoration contract: `FeeTracker::from_state(state, window)` MUST produce
/// a tracker that returns identical `estimate_fee_rate()` results as the
/// original at the time of snapshot.
///
/// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeEstimatorState {
    /// Serialized bucket statistics (all boundaries and counters).
    pub buckets: Vec<SerializedBucket>,
    /// Per-block fee summaries (up to `window` entries).
    pub block_history: Vec<BlockFeeData>,
    /// Height of the last recorded block.
    pub current_height: u64,
}

// ── FeeTracker ──

/// Internal fee-rate tracker with bucket-based historical statistics.
///
/// Created and owned by `Mempool` behind a `RwLock`. Not part of the public API.
pub(crate) struct FeeTracker {
    /// Number of recent blocks to retain. Default: 100.
    pub(crate) window: usize,
    /// Fee-rate buckets (logarithmically spaced from min to max FPC).
    buckets: Vec<FeeBucket>,
    /// Circular buffer of per-block confirmed transaction data.
    pub(crate) block_history: VecDeque<BlockFeeData>,
    /// Current height (last recorded block).
    #[allow(dead_code)]
    pub(crate) current_height: u64,
}

impl FeeTracker {
    /// Create a new tracker with the given rolling window and bucket count.
    pub(crate) fn new(window: usize, num_buckets: usize) -> Self {
        let num_buckets = num_buckets.max(2);
        Self {
            window,
            buckets: build_buckets(num_buckets),
            block_history: VecDeque::new(),
            current_height: 0,
        }
    }

    /// Record a newly confirmed block.
    ///
    /// 1. Applies 0.998 exponential decay to all bucket counters.
    /// 2. Places each `ConfirmedBundleInfo` into the appropriate bucket.
    /// 3. Appends `BlockFeeData` to the rolling window, evicting oldest if full.
    /// 4. Updates `current_height`.
    pub(crate) fn record_block(&mut self, height: u64, bundles: &[ConfirmedBundleInfo]) {
        self.apply_decay();

        let mut fpc_values: Vec<u128> = Vec::with_capacity(bundles.len());
        for bundle in bundles {
            if bundle.cost == 0 {
                continue;
            }
            let virtual_cost = MempoolItem::compute_virtual_cost(bundle.cost, bundle.num_spends);
            let fpc_scaled = MempoolItem::compute_fpc_scaled(bundle.fee, virtual_cost);
            fpc_values.push(fpc_scaled);

            let idx = self.find_bucket_idx(fpc_scaled);
            let b = &mut self.buckets[idx];
            b.total_observed += 1.0;
            // Default: assume 1-block confirmation wait (ConfirmedBundleInfo
            // does not carry blocks_waited, so we use the conservative default).
            b.confirmed_in_1 += 1.0;
            b.confirmed_in_2 += 1.0;
            b.confirmed_in_5 += 1.0;
            b.confirmed_in_10 += 1.0;
        }

        // Compute per-block summary statistics.
        fpc_values.sort_unstable();
        let num_tx = fpc_values.len();
        let (min_fpc, max_fpc, median_fpc) = if fpc_values.is_empty() {
            (0, 0, 0)
        } else {
            (
                fpc_values[0],
                fpc_values[num_tx - 1],
                fpc_values[num_tx / 2],
            )
        };

        // Maintain the rolling window.
        if self.block_history.len() >= self.window {
            self.block_history.pop_front();
        }
        self.block_history.push_back(BlockFeeData {
            height,
            min_fpc_included: min_fpc,
            max_fpc_included: max_fpc,
            median_fpc,
            num_transactions: num_tx,
        });

        self.current_height = height;
    }

    /// Estimate the fee rate needed for confirmation within `target_blocks`.
    ///
    /// Returns `None` when insufficient data (< `window / 2` blocks recorded).
    /// Scans buckets from highest to lowest fee rate; returns the first bucket
    /// whose `confirmed_in_N / total_observed ≥ 0.85`.
    ///
    /// The return value is `mojos_per_clvm_cost = bucket.fee_rate_lower / FPC_SCALE`.
    pub(crate) fn estimate_fee_rate(&self, target_blocks: u32) -> Option<u64> {
        if self.block_history.len() < self.window / 2 {
            return None;
        }

        for bucket in self.buckets.iter().rev() {
            if bucket.total_observed < 1.0 {
                continue;
            }
            let confirmed = match target_blocks {
                0 | 1 => bucket.confirmed_in_1,
                2 => bucket.confirmed_in_2,
                3..=5 => bucket.confirmed_in_5,
                _ => bucket.confirmed_in_10,
            };
            if confirmed / bucket.total_observed >= CONFIDENCE_THRESHOLD {
                let mojos = (bucket.fee_rate_lower / FPC_SCALE) as u64;
                return Some(mojos);
            }
        }

        None
    }

    // ── Inspection helpers (used by Mempool::fee_tracker_stats) ──

    pub(crate) fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    pub(crate) fn bucket_ranges(&self) -> Vec<(u128, u128)> {
        self.buckets
            .iter()
            .map(|b| (b.fee_rate_lower, b.fee_rate_upper))
            .collect()
    }

    pub(crate) fn bucket_totals(&self) -> Vec<f64> {
        self.buckets.iter().map(|b| b.total_observed).collect()
    }

    pub(crate) fn bucket_confirmed_in_1(&self) -> Vec<f64> {
        self.buckets.iter().map(|b| b.confirmed_in_1).collect()
    }

    // ── FEE-005: State serialization / restoration ──

    /// Snapshot the tracker's current state for persistence.
    ///
    /// Returns a `FeeEstimatorState` that can be round-tripped via serde and
    /// used to reconstruct an equivalent tracker via `from_state()`.
    ///
    /// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
    pub(crate) fn to_state(&self) -> FeeEstimatorState {
        FeeEstimatorState {
            buckets: self
                .buckets
                .iter()
                .map(|b| SerializedBucket {
                    fee_rate_lower: b.fee_rate_lower,
                    fee_rate_upper: b.fee_rate_upper,
                    confirmed_in_1: b.confirmed_in_1,
                    confirmed_in_2: b.confirmed_in_2,
                    confirmed_in_5: b.confirmed_in_5,
                    confirmed_in_10: b.confirmed_in_10,
                    total_observed: b.total_observed,
                })
                .collect(),
            block_history: self.block_history.iter().cloned().collect(),
            current_height: self.current_height,
        }
    }

    /// Reconstruct a `FeeTracker` from a persisted `FeeEstimatorState`.
    ///
    /// The reconstructed tracker will produce identical `estimate_fee_rate()`
    /// results as the original at the time of the snapshot.
    ///
    /// If the snapshot has a different bucket count than `num_buckets`, the
    /// state is loaded as-is (bucket count from the snapshot takes precedence).
    ///
    /// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
    pub(crate) fn from_state(state: FeeEstimatorState, window: usize) -> Self {
        let buckets: Vec<FeeBucket> = state
            .buckets
            .into_iter()
            .map(|b| FeeBucket {
                fee_rate_lower: b.fee_rate_lower,
                fee_rate_upper: b.fee_rate_upper,
                confirmed_in_1: b.confirmed_in_1,
                confirmed_in_2: b.confirmed_in_2,
                confirmed_in_5: b.confirmed_in_5,
                confirmed_in_10: b.confirmed_in_10,
                total_observed: b.total_observed,
            })
            .collect();
        Self {
            window,
            buckets,
            block_history: state.block_history.into_iter().collect(),
            current_height: state.current_height,
        }
    }

    // ── Internal helpers ──

    fn apply_decay(&mut self) {
        for b in &mut self.buckets {
            b.confirmed_in_1 *= DECAY_FACTOR;
            b.confirmed_in_2 *= DECAY_FACTOR;
            b.confirmed_in_5 *= DECAY_FACTOR;
            b.confirmed_in_10 *= DECAY_FACTOR;
            b.total_observed *= DECAY_FACTOR;
        }
    }

    /// Find the index of the bucket whose range contains `fpc_scaled`.
    ///
    /// Binary-searches the lower bounds. Values below the minimum bucket or
    /// above the maximum bucket are clamped to the first or last bucket.
    fn find_bucket_idx(&self, fpc_scaled: u128) -> usize {
        let n = self.buckets.len();
        if n == 0 {
            return 0;
        }
        if fpc_scaled <= self.buckets[0].fee_rate_lower {
            return 0;
        }
        if fpc_scaled >= self.buckets[n - 1].fee_rate_upper {
            return n - 1;
        }
        // Find the last bucket whose lower bound ≤ fpc_scaled.
        let mut lo = 0usize;
        let mut hi = n - 1;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.buckets[mid].fee_rate_lower <= fpc_scaled {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }
}

// ── Bucket construction ──

/// Build `num_buckets` logarithmically spaced `FeeBucket`s.
///
/// Spacing formula: bucket i spans
///   `[10^(log_min + i * range / n), 10^(log_min + (i+1) * range / n)]`
/// where `range = log10(MAX_FPC_SCALED) - log10(MIN_FPC_SCALED)`.
fn build_buckets(num_buckets: usize) -> Vec<FeeBucket> {
    let log_min = (MIN_FPC_SCALED as f64).log10();
    let log_max = (MAX_FPC_SCALED as f64).log10();
    let range = log_max - log_min;
    let n = num_buckets as f64;

    (0..num_buckets)
        .map(|i| {
            let lower_f = 10f64.powf(log_min + (i as f64) * range / n);
            let upper_f = 10f64.powf(log_min + ((i + 1) as f64) * range / n);
            FeeBucket {
                fee_rate_lower: lower_f.round() as u128,
                fee_rate_upper: upper_f.round() as u128,
                confirmed_in_1: 0.0,
                confirmed_in_2: 0.0,
                confirmed_in_5: 0.0,
                confirmed_in_10: 0.0,
                total_observed: 0.0,
            }
        })
        .collect()
}
