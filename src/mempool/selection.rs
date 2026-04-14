//! Block candidate selection.
//!
//! See: [SEL-001..008](docs/requirements/domains/selection/)

use std::collections::HashSet;
use std::sync::Arc;

use dig_clvm::Bytes32;

use crate::item::MempoolItem;
use crate::selection::{
    sel_002_is_selectable, sel_007_best, sel_008_topological_order, sel_greedy, SortStrategy,
};

use super::Mempool;

impl Mempool {
    /// Select an ordered set of active items for block inclusion.
    ///
    /// Returns items in topological order (parents before children) with
    /// fee-density descending within each layer.  Only items from the active
    /// pool are considered; pending items are never returned.
    ///
    /// # Selection Algorithm
    ///
    /// 1. **SEL-002** Pre-filter: remove expired / future-timelocked items.
    /// 2. **SEL-003..006** Run four greedy strategies (density, whale, compact, age).
    /// 3. **SEL-007** Best-set comparator: highest fees → lowest cost → fewest bundles.
    /// 4. **SEL-008** Topological ordering: layer 0 first, FPC-desc within layer.
    ///
    /// # Arguments
    ///
    /// - `max_block_cost`: virtual-cost budget for the block.
    /// - `height`: current block height (for timelock evaluation).
    /// - `timestamp`: current block timestamp (for timelock evaluation).
    ///
    /// See: [SEL-001](docs/requirements/domains/selection/specs/SEL-001.md)
    pub fn select_for_block(
        &self,
        max_block_cost: u64,
        height: u64,
        timestamp: u64,
    ) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        let max_spends = self.config.max_spends_per_block;

        // SEL-002: Pre-filter expired / future-timelocked items.
        let candidates: Vec<Arc<MempoolItem>> = pool
            .items
            .values()
            .filter(|item| sel_002_is_selectable(item, height, timestamp))
            .cloned()
            .collect();

        if candidates.is_empty() {
            return Vec::new();
        }

        let candidates_set: HashSet<Bytes32> =
            candidates.iter().map(|i| i.spend_bundle_id).collect();

        // Run all four strategies.
        let s1 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Density,
        );
        let s2 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Whale,
        );
        let s3 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Compact,
        );
        let s4 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Age,
        );

        // SEL-007: pick the best set.
        let best = sel_007_best([&s1, &s2, &s3, &s4]);

        // SEL-008: topological ordering.
        let result = sel_008_topological_order(best, &pool.dependencies);
        drop(pool);

        // LCY-005: Fire on_block_selected hook.
        self.fire_hooks(|h| h.on_block_selected(&result));

        result
    }

    /// Select an ordered set of active items using a custom strategy.
    ///
    /// Applies SEL-002 pre-filtering (expired / future-timelocked items removed),
    /// then delegates selection to `strategy.select()` instead of the built-in
    /// 4-way greedy algorithm.
    ///
    /// The strategy receives only eligible (non-expired, non-pending) items and
    /// must respect `max_block_cost` and `max_spends`. The mempool does NOT
    /// validate strategy output — invalid output may cause downstream failures.
    ///
    /// # Arguments
    ///
    /// - `strategy`: custom selection strategy (e.g., governance-priority ordering).
    /// - `max_block_cost`: virtual-cost budget for the block.
    /// - `height`: current block height (for timelock evaluation).
    /// - `timestamp`: current block timestamp (for timelock evaluation).
    ///
    /// See: [API-007](docs/requirements/domains/crate_api/specs/API-007.md)
    pub fn select_for_block_with_strategy(
        &self,
        strategy: &dyn crate::traits::BlockSelectionStrategy,
        max_block_cost: u64,
        height: u64,
        timestamp: u64,
    ) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        let max_spends = self.config.max_spends_per_block;

        // SEL-002: Pre-filter expired / future-timelocked items.
        let eligible: Vec<Arc<MempoolItem>> = pool
            .items
            .values()
            .filter(|item| sel_002_is_selectable(item, height, timestamp))
            .cloned()
            .collect();

        drop(pool);

        let result = strategy.select(&eligible, max_block_cost, max_spends);

        // LCY-005: Fire on_block_selected hook.
        self.fire_hooks(|h| h.on_block_selected(&result));

        result
    }
}
