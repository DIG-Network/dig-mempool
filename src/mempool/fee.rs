//! Fee estimation methods on the mempool.
//!
//! See: [FEE-001..005](docs/requirements/domains/fee_estimation/)

use dig_clvm::chia_protocol::FeeRate;

use crate::config::FPC_SCALE;
use crate::fee::{FeeEstimatorState, FeeTracker};
use crate::item::MempoolItem;
use crate::submit::ConfirmedBundleInfo;

use super::Mempool;

impl Mempool {
    /// Estimate the minimum fee required for a transaction to be admitted
    /// under current mempool conditions.
    ///
    /// Implements a 3-tier utilization system:
    ///
    /// | Utilization | Minimum Fee |
    /// |-------------|-------------|
    /// | < 80%       | 0           |
    /// | 80-100%     | `virtual_cost * full_mempool_min_fpc_scaled / FPC_SCALE` |
    /// | >= 100%     | `virtual_cost * (lowest_fpc + 1) / FPC_SCALE` |
    ///
    /// `virtual_cost = cost + (num_spends * config.spend_penalty_cost)`
    ///
    /// See: [FEE-001](docs/requirements/domains/fee_estimation/specs/FEE-001.md)
    pub fn estimate_min_fee(&self, cost: u64, num_spends: usize) -> u64 {
        let virtual_cost = MempoolItem::compute_virtual_cost(cost, num_spends);
        if virtual_cost == 0 {
            return 0;
        }

        let pool = self.pool.read().unwrap();
        let total_cost = pool.total_cost;
        let max_cost = self.config.max_total_cost;

        if max_cost == 0 || pool.items.is_empty() {
            return 0;
        }

        // Tier 1: < 80% utilization — no minimum fee required.
        // Avoid floating point: total_cost / max_cost < 0.80
        //   ↔ total_cost * 100 < max_cost * 80
        //   ↔ total_cost * 10 < max_cost * 8
        // Use u128 arithmetic to prevent overflow.
        if (total_cost as u128) * 10 < (max_cost as u128) * 8 {
            return 0;
        }

        // Tier 3: >= 100% utilization — must beat the lowest-FPC item.
        if total_cost >= max_cost {
            let lowest_fpc = pool
                .items
                .values()
                .map(|i| i.fee_per_virtual_cost_scaled)
                .min()
                .unwrap_or(0);
            let fee =
                (virtual_cost as u128).saturating_mul(lowest_fpc.saturating_add(1)) / FPC_SCALE;
            return fee.min(u64::MAX as u128) as u64;
        }

        // Tier 2: 80-100% utilization — apply minimum FPC threshold.
        let fee = (virtual_cost as u128).saturating_mul(self.config.full_mempool_min_fpc_scaled)
            / FPC_SCALE;
        fee.min(u64::MAX as u128) as u64
    }

    /// Record a confirmed block's transaction data into the fee estimator.
    ///
    /// Feeds confirmed bundle metrics to the internal `FeeTracker`:
    /// 1. Applies 0.998 exponential decay to all existing bucket counters.
    /// 2. Places each bundle into its fee-rate bucket (total_observed++).
    /// 3. Appends `BlockFeeData` to the rolling window.
    ///
    /// Called automatically by `on_new_block()` (step 5). May also be called
    /// directly for historical data seeding on startup.
    ///
    /// # Arguments
    ///
    /// - `height`: confirmed block height.
    /// - `bundles`: slice of per-bundle metrics from the confirmed block.
    ///
    /// See: [FEE-004](docs/requirements/domains/fee_estimation/specs/FEE-004.md)
    pub fn record_confirmed_block(&self, height: u64, bundles: &[ConfirmedBundleInfo]) {
        self.fee_tracker
            .write()
            .unwrap()
            .record_block(height, bundles);
    }

    /// Estimate the fee rate required for confirmation within `target_blocks`.
    ///
    /// Scans fee-rate buckets from highest to lowest and returns the first
    /// bucket whose success rate ≥ 85% for the given confirmation target.
    ///
    /// Returns `None` when:
    /// - Fewer than `fee_estimator_window / 2` blocks have been recorded.
    /// - No bucket meets the 85% confidence threshold (e.g., all empty).
    ///
    /// # Arguments
    ///
    /// - `target_blocks`: desired number of blocks within which to confirm.
    ///   `0` is treated as `1`. Values > 10 use the `confirmed_in_10` counter.
    ///
    /// See: [FEE-003](docs/requirements/domains/fee_estimation/specs/FEE-003.md)
    pub fn estimate_fee_rate(&self, target_blocks: u32) -> Option<FeeRate> {
        let tracker = self.fee_tracker.read().unwrap();
        tracker.estimate_fee_rate(target_blocks).map(FeeRate::new)
    }

    /// Extract the current fee estimator state for persistence.
    ///
    /// Returns a serializable `FeeEstimatorState` that captures the complete
    /// `FeeTracker` state: all bucket statistics and block history.
    /// Use `restore_fee_state()` to reload this state after a restart.
    ///
    /// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
    pub fn snapshot_fee_state(&self) -> FeeEstimatorState {
        self.fee_tracker.read().unwrap().to_state()
    }

    /// Restore the fee estimator state from a persisted snapshot.
    ///
    /// Replaces the current tracker contents with the provided state.
    /// After this call, `estimate_fee_rate()` produces the same results
    /// as the tracker that created the snapshot.
    ///
    /// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
    pub fn restore_fee_state(&self, state: FeeEstimatorState) {
        let window = self.config.fee_estimator_window;
        let new_tracker = FeeTracker::from_state(state, window);
        *self.fee_tracker.write().unwrap() = new_tracker;
    }
}
