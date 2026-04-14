//! Lifecycle management: `on_new_block()`, `clear()`, `remove()`, `evict_lowest_percent()`.
//!
//! Also contains `collect_descendants_parent_map` and `cascade_evict_and_record` free functions
//! used by the lifecycle and eviction logic.
//!
//! See: [LCY-001..008](docs/requirements/domains/lifecycle/)

use std::collections::{HashMap, HashSet};

use dig_clvm::Bytes32;

use crate::pools::ActivePool;
use crate::submit::{ConfirmedBundleInfo, RetryBundles};
use crate::traits::RemovalReason;

use super::Mempool;

/// Build a child → direct-parent map for all descendants of `root`.
///
/// Walks `dependents` (parent → children) depth-first before eviction.
/// Used by `on_new_block()` so that `CascadeEvicted { parent_id }` hooks
/// reference the direct parent rather than the eviction root.
pub(crate) fn collect_descendants_parent_map(
    dependents: &HashMap<Bytes32, HashSet<Bytes32>>,
    root: &Bytes32,
) -> HashMap<Bytes32, Bytes32> {
    let mut map = HashMap::new();
    let mut stack = vec![*root];
    while let Some(parent_id) = stack.pop() {
        if let Some(children) = dependents.get(&parent_id) {
            for &child_id in children {
                map.entry(child_id).or_insert(parent_id);
                stack.push(child_id);
            }
        }
    }
    map
}

/// Cascade-evict `bundle_id` from `pool`, then push removal events for the
/// root (with `root_reason`) and all cascade-evicted dependents.
///
/// If `cascade_out` is provided, the dependent IDs are also appended to it
/// (used by `on_new_block` to return the full cascade set to callers).
pub(crate) fn cascade_evict_and_record(
    pool: &mut ActivePool,
    bundle_id: &Bytes32,
    parent_map: &HashMap<Bytes32, Bytes32>,
    root_reason: RemovalReason,
    removal_events: &mut Vec<(Bytes32, RemovalReason)>,
    cascade_out: Option<&mut Vec<Bytes32>>,
) {
    let evicted = pool.cascade_evict(bundle_id);
    if let Some((root, dependents)) = evicted.split_last() {
        removal_events.push((*root, root_reason));
        for dep_id in dependents {
            let parent_id = parent_map.get(dep_id).copied().unwrap_or(*root);
            removal_events.push((*dep_id, RemovalReason::CascadeEvicted { parent_id }));
        }
        if let Some(out) = cascade_out {
            out.extend_from_slice(dependents);
        }
    }
}

impl Mempool {
    /// Clear all mempool state for reorg recovery.
    ///
    /// Drops all items from the active pool, pending pool, conflict cache, and
    /// seen cache. After this call the mempool is in the same state as a newly
    /// constructed one. Use when a chain reorganization invalidates the current
    /// pool state.
    ///
    /// # Concurrency
    ///
    /// Acquires write locks on all four state components. Callers must not hold
    /// any mempool read or write locks when calling this method.
    ///
    /// See: [LCY-004](docs/requirements/domains/lifecycle/specs/LCY-004.md)
    pub fn clear(&self) {
        // Collect all bundle IDs to notify hooks before clearing.
        // Fire hooks after releasing locks (hook implementations must not
        // acquire mempool locks to avoid deadlocks).
        let active_ids: Vec<Bytes32>;
        let pending_ids: Vec<Bytes32>;

        // Active pool
        {
            let mut pool = self.pool.write().unwrap();
            active_ids = pool.items.keys().copied().collect();
            pool.items.clear();
            pool.coin_index.clear();
            pool.mempool_coins.clear();
            pool.dependencies.clear();
            pool.dependents.clear();
            pool.dedup_index.clear();
            pool.dedup_waiters.clear();
            pool.singleton_spends.clear();
            pool.total_cost = 0;
            pool.total_fees = 0;
            pool.total_spends = 0;
        }
        // Pending pool
        {
            let mut pending = self.pending.write().unwrap();
            pending_ids = pending.pending.keys().copied().collect();
            pending.pending.clear();
            pending.pending_coin_index.clear();
            pending.pending_cost = 0;
        }
        // Conflict cache
        {
            let mut conflict = self.conflict.write().unwrap();
            conflict.cache.clear();
            conflict.total_cost = 0;
        }
        // Seen cache
        {
            self.seen_cache.write().unwrap().clear();
        }

        // LCY-004: Fire on_item_removed(Cleared) for all removed active + pending items.
        for id in active_ids {
            self.fire_hooks(|h| h.on_item_removed(&id, RemovalReason::Cleared));
        }
        for id in pending_ids {
            self.fire_hooks(|h| h.on_item_removed(&id, RemovalReason::Cleared));
        }
    }

    /// Remove an active item by bundle ID.
    ///
    /// Returns `true` if the item was found and removed, `false` if it was not
    /// in the active pool. This is the single-item removal primitive used by
    /// `on_new_block()` (LCY-001) when confirmed coins are removed from the pool.
    ///
    /// Updates all indexes: `coin_index`, `mempool_coins`, `dedup_index`, and
    /// `dedup_waiters`. The seen-cache is NOT modified — previously-submitted
    /// bundles remain cached as seen to prevent re-admission.
    ///
    /// See: [LCY-001](docs/requirements/domains/lifecycle/specs/LCY-001.md)
    pub fn remove(&self, bundle_id: &Bytes32) -> bool {
        self.pool.write().unwrap().remove(bundle_id).is_some()
    }

    /// Evict the lowest-value items to relieve memory pressure.
    ///
    /// Removes approximately `percent`% of the active pool's total virtual cost
    /// by evicting items in ascending `descendant_score` order. Items within the
    /// expiry protection window are skipped (they expire naturally via `on_new_block()`).
    /// Cascade-evicts CPFP dependents of each evicted item.
    ///
    /// # Parameters
    ///
    /// - `percent`: Fraction of total cost to free (0–100). Values > 100 are treated as 100.
    /// - `current_height`: Current block height (for expiry protection checks).
    ///
    /// # Behavior
    ///
    /// - `percent = 0`: no-op (nothing removed).
    /// - `percent = 100`: evict all non-expiry-protected items.
    /// - Fires `on_item_removed(CapacityEviction)` for primary evictions.
    /// - Fires `on_item_removed(CascadeEvicted { parent_id })` for cascade evictions.
    ///
    /// See: [LCY-008](docs/requirements/domains/lifecycle/specs/LCY-008.md)
    pub fn evict_lowest_percent(&self, percent: u8, current_height: u64) {
        if percent == 0 {
            return;
        }
        let percent = percent.min(100) as u64;

        let mut removal_events: Vec<(Bytes32, RemovalReason)> = Vec::new();

        {
            let mut pool = self.pool.write().unwrap();
            let target = pool.total_cost.saturating_mul(percent) / 100;
            if target == 0 {
                return;
            }

            // Sort items by descendant_score ascending (lowest value first).
            let mut sorted: Vec<(u128, Bytes32)> = pool
                .items
                .values()
                .map(|item| (item.descendant_score, item.spend_bundle_id))
                .collect();
            sorted.sort_by_key(|(score, _)| *score);

            let protection_blocks = self.config.expiry_protection_blocks;
            let mut cost_removed: u64 = 0;

            for (_, bundle_id) in sorted {
                if cost_removed >= target {
                    break;
                }

                // Item may have been cascade-evicted by a previous iteration.
                if !pool.items.contains_key(&bundle_id) {
                    continue;
                }

                // Skip expiry-protected items.
                if let Some(item) = pool.items.get(&bundle_id) {
                    let protected = item.assert_before_height.is_some_and(|abh| {
                        abh > current_height
                            && abh <= current_height.saturating_add(protection_blocks)
                    });
                    if protected {
                        continue;
                    }
                }

                // Collect parent map before eviction for CascadeEvicted hooks.
                let parent_map = collect_descendants_parent_map(&pool.dependents, &bundle_id);

                // Pre-compute cost of this item + all its descendants before eviction.
                let mut subtree_cost: u64 = 0;
                {
                    // Collect all IDs in subtree (root + descendants).
                    let mut stack = vec![bundle_id];
                    let mut visited = std::collections::HashSet::new();
                    while let Some(id) = stack.pop() {
                        if !visited.insert(id) {
                            continue;
                        }
                        if let Some(item) = pool.items.get(&id) {
                            subtree_cost = subtree_cost.saturating_add(item.virtual_cost);
                        }
                        if let Some(children) = pool.dependents.get(&id) {
                            stack.extend(children.iter().copied());
                        }
                    }
                }

                cascade_evict_and_record(
                    &mut pool,
                    &bundle_id,
                    &parent_map,
                    RemovalReason::CapacityEviction,
                    &mut removal_events,
                    None,
                );

                cost_removed = cost_removed.saturating_add(subtree_cost);
            }
        }

        // Fire hooks after releasing the write lock.
        for (id, reason) in removal_events {
            self.fire_hooks(|h| h.on_item_removed(&id, reason.clone()));
        }
    }

    /// Process a newly confirmed block: remove confirmed and expired items,
    /// collect pending promotions, and drain eligible conflict-cache retries.
    ///
    /// # Arguments
    ///
    /// - `height`: the new confirmed block height.
    /// - `timestamp`: the new confirmed block timestamp.
    /// - `spent_coin_ids`: coin IDs spent (confirmed) in this block.
    /// - `confirmed_bundles`: per-bundle metrics for the confirmed transactions
    ///   (forwarded to the fee estimator — currently a no-op until FEE-004).
    ///
    /// # Processing Order
    ///
    /// 1. Remove confirmed items (spending `spent_coin_ids`) + cascade-evict their dependents.
    /// 2. Remove expired items (`assert_before_height <= height` or `assert_before_seconds <= timestamp`) + cascade.
    /// 3. Collect pending promotions (`assert_height <= height`).
    /// 4. Collect conflict retries (bundles whose conflicting active items are gone).
    /// 5. Update fee estimator via `record_confirmed_block()` (FEE-004).
    ///
    /// See: [LCY-001](docs/requirements/domains/lifecycle/specs/LCY-001.md)
    pub fn on_new_block(
        &self,
        height: u64,
        timestamp: u64,
        spent_coin_ids: &[Bytes32],
        confirmed_bundles: &[ConfirmedBundleInfo],
    ) -> RetryBundles {
        let mut cascade_evicted: Vec<Bytes32> = Vec::new();

        // Accumulate (bundle_id, reason) pairs for hooks — fired after all locks released.
        let mut removal_events: Vec<(Bytes32, RemovalReason)> = Vec::new();

        // Step 0: Clear the seen cache so that promoted/retry bundles can be resubmitted.
        //
        // The seen cache prevents re-validation of bundles seen in the same block cycle.
        // On a new block boundary, all previously seen hashes are stale — bundles must
        // be re-evaluated against the new chain state. This matches Chia's behaviour in
        // `MempoolManager.new_peak()`, which clears `seen_bundle_hashes` at each peak.
        self.seen_cache.write().unwrap().clear();

        // Steps 1 + 2: Remove confirmed + expired items under a single write lock.
        {
            let mut pool = self.pool.write().unwrap();

            // Step 1: Confirmed items — bundles spending any of the confirmed coins.
            let confirmed_ids: Vec<Bytes32> = {
                let mut seen = HashSet::new();
                spent_coin_ids
                    .iter()
                    .filter_map(|coin_id| pool.coin_index.get(coin_id).copied())
                    .filter(|id| seen.insert(*id))
                    .collect()
            };

            for bundle_id in confirmed_ids {
                // Collect child→parent map BEFORE eviction (dependents map is cleared).
                let parent_map = collect_descendants_parent_map(&pool.dependents, &bundle_id);

                // cascade_evict removes the root AND all dependents (children first).
                // The last element is the root (confirmed item); everything before it
                // is a cascade-evicted dependent.
                cascade_evict_and_record(
                    &mut pool,
                    &bundle_id,
                    &parent_map,
                    RemovalReason::Confirmed,
                    &mut removal_events,
                    Some(&mut cascade_evicted),
                );
            }

            // Step 2: Expired items — past assert_before_height or assert_before_seconds.
            let expired_ids: Vec<Bytes32> = pool
                .items
                .values()
                .filter(|item| {
                    let h_expired = item.assert_before_height.is_some_and(|h| h <= height);
                    let s_expired = item.assert_before_seconds.is_some_and(|s| s <= timestamp);
                    h_expired || s_expired
                })
                .map(|item| item.spend_bundle_id)
                .collect();

            for bundle_id in expired_ids {
                let parent_map = collect_descendants_parent_map(&pool.dependents, &bundle_id);

                cascade_evict_and_record(
                    &mut pool,
                    &bundle_id,
                    &parent_map,
                    RemovalReason::Expired,
                    &mut removal_events,
                    Some(&mut cascade_evicted),
                );
            }
        }

        // Step 3: Pending promotions — timelocked items whose height is now satisfied.
        let pending_promotions = {
            let mut pending = self.pending.write().unwrap();
            pending.drain(height, timestamp)
        };

        // Step 4: Conflict retries — bundles whose conflicting active items are gone.
        //
        // A conflict-cache bundle is retryable when none of the coins it spends
        // are still claimed by an active pool item. If any conflicting coin is still
        // active, the bundle would fail RBF again immediately.
        let conflict_retries = {
            let pool = self.pool.read().unwrap();
            let mut conflict = self.conflict.write().unwrap();

            let retryable: Vec<Bytes32> = conflict
                .cache
                .iter()
                .filter(|(_, (bundle, _))| {
                    !bundle
                        .coin_spends
                        .iter()
                        .any(|cs| pool.coin_index.contains_key(&cs.coin.coin_id()))
                })
                .map(|(id, _)| *id)
                .collect();

            let mut bundles = Vec::with_capacity(retryable.len());
            for id in retryable {
                if let Some((bundle, cost)) = conflict.cache.remove(&id) {
                    conflict.total_cost = conflict.total_cost.saturating_sub(cost);
                    bundles.push(bundle);
                }
            }
            bundles
        };

        // Step 5: Update fee estimator with confirmed block data (FEE-004).
        self.record_confirmed_block(height, confirmed_bundles);

        // LCY-005: Fire removal hooks after all locks are released.
        for (id, reason) in removal_events {
            self.fire_hooks(|h| h.on_item_removed(&id, reason.clone()));
        }

        RetryBundles {
            conflict_retries,
            pending_promotions,
            cascade_evicted,
        }
    }
}
