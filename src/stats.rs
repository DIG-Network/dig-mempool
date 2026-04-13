//! Mempool statistics snapshot.

/// Aggregate mempool statistics returned by `Mempool::stats()`.
#[derive(Debug, Clone)]
pub struct MempoolStats {
    pub active_count: usize,
    pub pending_count: usize,
    pub conflict_count: usize,
    pub total_cost: u64,
    pub total_fees: u64,
    pub max_cost: u64,
    pub utilization: f64,
    pub min_fpc_scaled: u128,
    pub max_fpc_scaled: u128,
    pub items_with_dependencies: usize,
    pub max_current_depth: u32,
    pub total_spend_count: usize,
    pub dedup_eligible_count: usize,
    pub singleton_ff_count: usize,
}

impl MempoolStats {
    /// Create an empty stats snapshot for a new mempool with the given max_cost.
    pub(crate) fn empty(max_cost: u64) -> Self {
        Self {
            active_count: 0,
            pending_count: 0,
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
