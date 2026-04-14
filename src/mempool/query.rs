//! Read-only query methods on the mempool.
//!
//! All methods use `&self` and acquire read locks only, enabling concurrent
//! access alongside other reads.
//!
//! See: [API-008](docs/requirements/domains/crate_api/specs/API-008.md)

use std::collections::HashSet;
use std::sync::Arc;

use dig_clvm::{Bytes32, CoinRecord, SpendBundle};

use crate::fee::FeeTrackerStats;
use crate::item::MempoolItem;
use crate::stats::MempoolStats;

use super::Mempool;

impl Mempool {
    /// Number of active (non-pending, non-conflicting) items in the mempool.
    ///
    /// Returns 0 for a newly constructed mempool.
    /// Thread-safe: acquires a read lock on the active pool.
    pub fn len(&self) -> usize {
        self.pool.read().unwrap().items.len()
    }

    /// Whether the active mempool is empty (zero active items).
    ///
    /// Equivalent to `self.len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Aggregate mempool statistics snapshot.
    ///
    /// Returns a point-in-time snapshot of all mempool metrics.
    /// Thread-safe: acquires read locks on relevant pools.
    ///
    /// The `max_cost` field reflects `config.max_total_cost`, which is the
    /// total capacity of the active pool.
    ///
    /// See: [`MempoolStats`] and [API-006](docs/requirements/domains/crate_api/specs/API-006.md).
    pub fn stats(&self) -> MempoolStats {
        let pool = self.pool.read().unwrap();

        let active_count = pool.items.len();
        let total_cost = pool.total_cost;
        let total_fees = pool.total_fees;
        let total_spend_count = pool.total_spends;

        let utilization = if self.config.max_total_cost > 0 {
            total_cost as f64 / self.config.max_total_cost as f64
        } else {
            0.0
        };

        // Compute all per-item stats in a single pass.
        let mut min_fpc_scaled = u128::MAX;
        let mut max_fpc_scaled = 0u128;
        let mut items_with_dependencies = 0usize;
        let mut max_current_depth = 0u32;
        let mut dedup_eligible_count = 0usize;
        let mut singleton_ff_count = 0usize;

        for item in pool.items.values() {
            if item.fee_per_virtual_cost_scaled < min_fpc_scaled {
                min_fpc_scaled = item.fee_per_virtual_cost_scaled;
            }
            if item.fee_per_virtual_cost_scaled > max_fpc_scaled {
                max_fpc_scaled = item.fee_per_virtual_cost_scaled;
            }
            if item.depth > 0 {
                items_with_dependencies += 1;
            }
            if item.depth > max_current_depth {
                max_current_depth = item.depth;
            }
            if item.eligible_for_dedup {
                dedup_eligible_count += 1;
            }
            if item.singleton_lineage.is_some() {
                singleton_ff_count += 1;
            }
        }
        // Normalize min to 0 for an empty pool (sentinel MAX has no meaning).
        if pool.items.is_empty() {
            min_fpc_scaled = 0;
        }

        let (pending_count, pending_cost) = {
            let pending = self.pending.read().unwrap();
            (pending.pending.len(), pending.pending_cost)
        };

        let conflict_count = self.conflict.read().unwrap().len();

        MempoolStats {
            active_count,
            pending_count,
            pending_cost,
            conflict_count,
            total_cost,
            total_fees,
            max_cost: self.config.max_total_cost,
            utilization,
            min_fpc_scaled,
            max_fpc_scaled,
            items_with_dependencies,
            max_current_depth,
            total_spend_count,
            dedup_eligible_count,
            singleton_ff_count,
        }
    }

    /// Look up an active mempool item by its spend bundle ID.
    ///
    /// Returns `None` if the bundle ID is not in the active pool.
    /// The returned `Arc<MempoolItem>` is a cheap reference-counted pointer —
    /// the item remains live as long as the Arc is held, even if the item is
    /// later removed from the pool.
    pub fn get(&self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        self.pool.read().unwrap().items.get(bundle_id).cloned()
    }

    /// Check whether a bundle ID is in the **active** pool.
    ///
    /// Returns `true` only for items that have been fully admitted and are
    /// eligible for block selection. Pending (timelocked) items and conflict-
    /// cache items are intentionally excluded — use `pending_bundle_ids()` or
    /// `conflict_len()` to query those pools.
    pub fn contains(&self, bundle_id: &Bytes32) -> bool {
        self.pool.read().unwrap().items.contains_key(bundle_id)
    }

    /// Return all active (non-pending) bundle IDs.
    ///
    /// The order is not guaranteed. Use `select_for_block()` for ordered selection.
    pub fn active_bundle_ids(&self) -> Vec<Bytes32> {
        self.pool.read().unwrap().items.keys().copied().collect()
    }

    /// Return all pending (timelocked) bundle IDs.
    pub fn pending_bundle_ids(&self) -> Vec<Bytes32> {
        self.pending
            .read()
            .unwrap()
            .pending
            .keys()
            .copied()
            .collect()
    }

    /// Return all active mempool items as Arc references.
    ///
    /// Cheap to call — Arc clones are pointer copies (not item copies).
    pub fn active_items(&self) -> Vec<Arc<MempoolItem>> {
        self.pool.read().unwrap().items.values().cloned().collect()
    }

    /// Return the direct dependents (children) of a bundle.
    ///
    /// A dependent is a bundle that spends a coin created by the given bundle.
    /// Returns empty vec if the bundle has no dependents or doesn't exist.
    /// See: [CPF-002](docs/requirements/domains/cpfp/specs/CPF-002.md)
    pub fn dependents_of(&self, bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        pool.dependents
            .get(bundle_id)
            .into_iter()
            .flatten()
            .filter_map(|id| pool.items.get(id).cloned())
            .collect()
    }

    /// Return all ancestors (parents, grandparents, ...) of a bundle.
    ///
    /// Walks the dependency chain transitively. Used for CPFP package
    /// analysis and cascade eviction planning.
    /// See: [CPF-002](docs/requirements/domains/cpfp/specs/CPF-002.md)
    pub fn ancestors_of(&self, bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        let mut result = Vec::new();
        let mut to_visit: Vec<Bytes32> = pool
            .dependencies
            .get(bundle_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let mut visited: HashSet<Bytes32> = HashSet::new();
        while let Some(ancestor_id) = to_visit.pop() {
            if !visited.insert(ancestor_id) {
                continue;
            }
            if let Some(item) = pool.items.get(&ancestor_id) {
                result.push(item.clone());
                to_visit.extend(item.depends_on.iter().copied());
            }
        }
        result
    }

    /// Number of timelocked items in the pending pool.
    pub fn pending_len(&self) -> usize {
        self.pending.read().unwrap().pending.len()
    }

    /// Extract all pending items whose timelocks are satisfied at `height` / `timestamp`.
    ///
    /// Returns spend bundles for re-submission. Each returned bundle must be
    /// re-submitted via `submit()` with fresh coin records and current chain state,
    /// because coin records and timelock conditions must be re-evaluated.
    ///
    /// This is called internally by `on_new_block()` (LCY-001) when the chain
    /// advances. It is exposed publicly for testing and for callers who manage
    /// the lifecycle directly.
    ///
    /// See: [POL-004](docs/requirements/domains/pools/specs/POL-004.md)
    pub fn drain_pending(&self, height: u64, timestamp: u64) -> Vec<SpendBundle> {
        self.pending.write().unwrap().drain(height, timestamp)
    }

    /// Number of items in the conflict retry cache.
    pub fn conflict_len(&self) -> usize {
        self.conflict.read().unwrap().len()
    }

    /// Add a bundle to the conflict cache after a failed active-pool RBF.
    ///
    /// Silently drops the bundle if the count or cost limit would be exceeded,
    /// or if the bundle ID is already cached. Returns `true` if inserted.
    ///
    /// Called by the active-pool RBF path (CFR-005) and exposed publicly for
    /// testing and for callers who manage conflict state directly.
    ///
    /// See: [POL-006](docs/requirements/domains/pools/specs/POL-006.md)
    pub fn add_to_conflict_cache(&self, bundle: SpendBundle, estimated_cost: u64) -> bool {
        let bundle_id = bundle.name();
        let inserted = self.conflict.write().unwrap().insert(
            bundle,
            estimated_cost,
            self.config.max_conflict_count,
            self.config.max_conflict_cost,
        );
        if inserted {
            self.fire_hooks(|h| h.on_conflict_cached(&bundle_id));
        }
        inserted
    }

    /// Drain all conflict cache entries for re-submission.
    ///
    /// Returns the raw SpendBundles. Each bundle must be re-submitted via
    /// `submit()` with fresh coin records. Called by `on_new_block()` (LCY-001)
    /// when a block is confirmed and previously-conflicting items may now be
    /// admissible.
    ///
    /// See: [POL-006](docs/requirements/domains/pools/specs/POL-006.md)
    pub fn drain_conflict(&self) -> Vec<SpendBundle> {
        self.conflict.write().unwrap().drain()
    }

    /// Look up a coin created by an active mempool item.
    ///
    /// Returns a synthetic `CoinRecord` suitable for use in a subsequent
    /// `submit()` call (CPFP). The synthetic record uses the parent item's
    /// `height_added` as `confirmed_block_index`.
    ///
    /// Returns `None` if the coin was not created by any active item.
    /// Note: TOCTOU safe — if the parent is evicted between this call and
    /// `submit()`, Phase 2 will reject with `CoinNotFound`.
    ///
    /// See: [SPEC.md Section 3.3](docs/resources/SPEC.md) — CPFP Coin Queries
    pub fn get_mempool_coin_record(&self, coin_id: &Bytes32) -> Option<CoinRecord> {
        let pool = self.pool.read().unwrap();
        let &creator_id = pool.mempool_coins.get(coin_id)?;
        let creator = pool.items.get(&creator_id)?;
        // Find the specific coin among the creator's additions.
        let coin = creator.additions.iter().find(|c| c.coin_id() == *coin_id)?;
        Some(CoinRecord {
            coin: *coin,
            coinbase: false,
            confirmed_block_index: creator.height_added as u32,
            spent: false,
            spent_block_index: 0,
            timestamp: 0, // admission timestamp not tracked in MempoolItem
        })
    }

    /// Look up which active mempool item created a given coin.
    ///
    /// Returns the creating bundle's ID, or `None` if the coin was not
    /// created by any active mempool item.
    ///
    /// See: [SPEC.md Section 3.3](docs/resources/SPEC.md) — CPFP Coin Queries
    pub fn get_mempool_coin_creator(&self, coin_id: &Bytes32) -> Option<Bytes32> {
        // POL-001: look up in mempool_coins index
        // mempool_coins: created_coin_id -> creating bundle_id
        self.pool
            .read()
            .unwrap()
            .mempool_coins
            .get(coin_id)
            .copied()
    }

    /// Look up which pending item spends a given coin.
    ///
    /// Returns the spending bundle's ID, or `None` if the coin is not spent
    /// by any pending item. Used for pending-vs-pending conflict detection.
    ///
    /// See: [POL-005](docs/requirements/domains/pools/specs/POL-005.md)
    pub fn get_pending_coin_spender(&self, coin_id: &Bytes32) -> Option<Bytes32> {
        self.pending
            .read()
            .unwrap()
            .pending_coin_index
            .get(coin_id)
            .copied()
    }

    /// Number of entries in the identical-spend dedup index.
    ///
    /// Each entry represents a unique (coin_id, sha256(solution)) pair that has
    /// at least one cost-bearing active bundle. Used for testing and diagnostics.
    ///
    /// See: [POL-008](docs/requirements/domains/pools/specs/POL-008.md)
    pub fn dedup_index_len(&self) -> usize {
        self.pool.read().unwrap().dedup_index.len()
    }

    /// Look up the cost-bearer bundle ID for a (coin_id, solution_hash) dedup key.
    ///
    /// Returns `None` if the key is not in the dedup index (no eligible bundle
    /// for this spend has been admitted, or dedup is disabled).
    ///
    /// See: [POL-008](docs/requirements/domains/pools/specs/POL-008.md)
    pub fn get_dedup_bearer(&self, coin_id: &Bytes32, solution_hash: &Bytes32) -> Option<Bytes32> {
        self.pool
            .read()
            .unwrap()
            .dedup_index
            .get(&(*coin_id, *solution_hash))
            .copied()
    }

    /// Return the ordered bundle ID chain for a singleton launcher.
    ///
    /// Returns bundle IDs in lineage order (oldest first). Returns an empty
    /// vec if the launcher has no active items in the pool.
    ///
    /// See: [POL-009](docs/requirements/domains/pools/specs/POL-009.md)
    pub fn singleton_chain(&self, launcher_id: &Bytes32) -> Vec<Bytes32> {
        self.pool
            .read()
            .unwrap()
            .singleton_spends
            .get(launcher_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Number of distinct singleton launchers currently tracked.
    ///
    /// Each launcher with at least one active item is counted once.
    ///
    /// See: [POL-009](docs/requirements/domains/pools/specs/POL-009.md)
    pub fn singleton_spends_count(&self) -> usize {
        self.pool.read().unwrap().singleton_spends.len()
    }

    /// Return a snapshot of the fee tracker's internal state.
    ///
    /// Exposes bucket counts, window size, history length, and per-bucket
    /// counters for external inspection and testing. Read-only; does not
    /// mutate the tracker.
    ///
    /// See: [FEE-002](docs/requirements/domains/fee_estimation/specs/FEE-002.md)
    pub fn fee_tracker_stats(&self) -> FeeTrackerStats {
        let tracker = self.fee_tracker.read().unwrap();
        FeeTrackerStats {
            bucket_count: tracker.bucket_count(),
            window: tracker.window,
            history_len: tracker.block_history.len(),
            bucket_ranges: tracker.bucket_ranges(),
            bucket_totals: tracker.bucket_totals(),
            bucket_confirmed_in_1: tracker.bucket_confirmed_in_1(),
        }
    }
}
