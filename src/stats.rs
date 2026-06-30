//! Mempool statistics snapshot.
//!
//! # Overview
//!
//! `MempoolStats` provides a point-in-time snapshot of aggregate mempool metrics.
//! Returned by `Mempool::stats()` for monitoring, fee estimation, and capacity
//! management decisions.
//!
//! # Design Rationale
//!
//! Stats are computed at query time from the mempool's internal state (locked
//! accumulators). The struct is `Clone + Debug` so callers can capture and
//! compare snapshots without holding mempool locks.
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 3.6](docs/resources/SPEC.md) — MempoolStats definition
//! - [API-006](docs/requirements/domains/crate_api/specs/API-006.md) — Requirement

/// Aggregate mempool statistics returned by [`crate::Mempool::stats()`].
///
/// All counts and totals reflect the **active pool** only, unless otherwise
/// noted. Pending pool and conflict cache counts are separate fields.
///
/// # Field Reference
///
/// | Field | Source | Pool |
/// |-------|--------|------|
/// | `active_count` | `items.len()` | Active |
/// | `pending_count` | `pending.len()` | Pending |
/// | `conflict_count` | `conflict_cache.len()` | Conflict |
/// | `total_cost` | Sum of active `item.cost` | Active |
/// | `total_fees` | Sum of active `item.fee` | Active |
/// | `max_cost` | `config.max_total_cost` | Config |
/// | `utilization` | `total_cost / max_cost` | Derived |
/// | `min_fpc_scaled` | Lowest active FPC | Active |
/// | `max_fpc_scaled` | Highest active FPC | Active |
/// | `items_with_dependencies` | Active items with `depth > 0` | Active |
/// | `max_current_depth` | Deepest CPFP chain | Active |
/// | `total_spend_count` | Sum of active `item.num_spends` | Active |
/// | `dedup_eligible_count` | Active items with `eligible_for_dedup` | Active |
/// | `singleton_ff_count` | Active items with `singleton_lineage.is_some()` | Active |
#[derive(Debug, Clone)]
pub struct MempoolStats {
    /// Number of items in the active pool.
    pub active_count: usize,

    /// Number of timelocked items in the pending pool.
    pub pending_count: usize,

    /// Total virtual cost of all pending items.
    pub pending_cost: u64,

    /// Number of items in the conflict retry cache.
    pub conflict_count: usize,

    /// Total CLVM cost of all active items (sum of `item.cost`).
    /// Used for utilization calculation.
    pub total_cost: u64,

    /// Total fees of all active items (sum of `item.fee`).
    pub total_fees: u64,

    /// Maximum total cost capacity from `config.max_total_cost`.
    /// For default config: `L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER` = 8.25T.
    pub max_cost: u64,

    /// Mempool utilization ratio: `total_cost / max_cost`.
    /// Range: 0.0 (empty) to 1.0+ (at or over capacity).
    /// Used by `estimate_min_fee()` for fee tier determination.
    /// See: [FEE-001](docs/requirements/domains/fee_estimation/specs/FEE-001.md)
    pub utilization: f64,

    /// Lowest fee-per-virtual-cost (scaled) in the active pool.
    /// 0 if the pool is empty. Used for eviction threshold.
    pub min_fpc_scaled: u128,

    /// Highest fee-per-virtual-cost (scaled) in the active pool.
    /// 0 if the pool is empty.
    pub max_fpc_scaled: u128,

    /// Number of active items with CPFP dependencies (`depth > 0`).
    pub items_with_dependencies: usize,

    /// Maximum CPFP dependency depth among all active items.
    /// 0 if no items have dependencies.
    pub max_current_depth: u32,

    /// Total coin spend count across all active items.
    /// Sum of `item.num_spends`. Used for spend count limit enforcement.
    pub total_spend_count: usize,

    /// Number of active items eligible for identical-spend deduplication.
    pub dedup_eligible_count: usize,

    /// Number of active items with singleton fast-forward lineage info.
    pub singleton_ff_count: usize,
}

impl MempoolStats {
    /// Create an empty stats snapshot for a new mempool with the given max_cost.
    ///
    /// Useful for tests and as a baseline for snapshot comparisons.
    /// All counters and totals are zero; `max_cost` reflects the configured capacity.
    #[allow(dead_code)] // Used in tests and future lifecycle code
    pub(crate) fn empty(max_cost: u64) -> Self {
        Self {
            active_count: 0,
            pending_count: 0,
            pending_cost: 0,
            conflict_count: 0,
            total_cost: 0,
            total_fees: 0,
            max_cost,
            utilization: 0.0,
            min_fpc_scaled: 0,
            max_fpc_scaled: 0,
            items_with_dependencies: 0,
            max_current_depth: 0,
            total_spend_count: 0,
            dedup_eligible_count: 0,
            singleton_ff_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MempoolStats;

    /// `empty()` produces an all-zero baseline snapshot carrying only the
    /// configured `max_cost`.
    ///
    /// This is the baseline used for snapshot comparisons, so every counter and
    /// total must be zero and the utilization ratio must be exactly 0.0; only
    /// `max_cost` reflects the passed capacity. Guards against a field being
    /// added to the struct but left out of the empty constructor.
    #[test]
    fn empty_zeroes_every_counter_and_keeps_max_cost() {
        let max_cost = 8_250_000_000_000u64;
        let s = MempoolStats::empty(max_cost);

        assert_eq!(s.max_cost, max_cost, "max_cost must reflect the argument");

        // Every count/total/derived metric is zero for an empty pool.
        assert_eq!(s.active_count, 0);
        assert_eq!(s.pending_count, 0);
        assert_eq!(s.pending_cost, 0);
        assert_eq!(s.conflict_count, 0);
        assert_eq!(s.total_cost, 0);
        assert_eq!(s.total_fees, 0);
        assert_eq!(s.utilization, 0.0);
        assert_eq!(s.min_fpc_scaled, 0);
        assert_eq!(s.max_fpc_scaled, 0);
        assert_eq!(s.items_with_dependencies, 0);
        assert_eq!(s.max_current_depth, 0);
        assert_eq!(s.total_spend_count, 0);
        assert_eq!(s.dedup_eligible_count, 0);
        assert_eq!(s.singleton_ff_count, 0);
    }

    /// A different `max_cost` is carried through, and the snapshot is `Clone`
    /// (callers capture/compare snapshots without holding mempool locks).
    #[test]
    fn empty_is_cloneable_and_carries_distinct_max_cost() {
        let s = MempoolStats::empty(42);
        let c = s.clone();
        assert_eq!(c.max_cost, 42);
        assert_eq!(c.active_count, 0);
    }
}
