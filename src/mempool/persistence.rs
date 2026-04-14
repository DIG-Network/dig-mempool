//! Snapshot / Restore persistence for mempool state.
//!
//! See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)

use std::sync::Arc;

use dig_clvm::SpendBundle;
use serde::{Deserialize, Serialize};

use crate::fee::{FeeEstimatorState, FeeTracker};
use crate::item::MempoolItem;

use super::Mempool;

/// Serializable snapshot of complete mempool state.
///
/// Returned by [`Mempool::snapshot()`] and accepted by [`Mempool::restore()`].
/// The seen-cache is intentionally excluded: bundles that were seen before the
/// snapshot are not rejected as `AlreadySeen` after restore.
///
/// # JSON compatibility
///
/// `MempoolSnapshot` derives `Serialize + Deserialize`. Use `serde_json` to
/// persist the snapshot across restarts:
///
/// ```ignore
/// let json = serde_json::to_string(&mempool.snapshot())?;
/// mempool.restore(serde_json::from_str(&json)?);
/// ```
///
/// See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)
#[derive(Serialize, Deserialize)]
pub struct MempoolSnapshot {
    /// All active (non-pending, non-conflicting) items at snapshot time.
    pub active_items: Vec<MempoolItem>,
    /// All pending (timelocked) items at snapshot time.
    pub pending_items: Vec<MempoolItem>,
    /// Conflict-cache entries: `(SpendBundle, estimated_virtual_cost)`.
    pub conflict_bundles: Vec<(SpendBundle, u64)>,
    /// Fee estimator state: bucket statistics + block history.
    pub fee_estimator_state: FeeEstimatorState,
}

impl Mempool {
    /// Capture a serializable snapshot of the complete mempool state.
    ///
    /// Includes all active items, pending items, conflict-cache bundles,
    /// and fee estimator state. The seen-cache is intentionally excluded:
    /// after `restore()`, bundles that were previously seen can be resubmitted
    /// without being rejected as `AlreadySeen`.
    ///
    /// # Thread safety
    ///
    /// Acquires read locks on all four state components.
    ///
    /// See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)
    pub fn snapshot(&self) -> MempoolSnapshot {
        let active_items: Vec<MempoolItem> = self
            .pool
            .read()
            .unwrap()
            .items
            .values()
            .map(|arc| (**arc).clone())
            .collect();

        let pending_items: Vec<MempoolItem> = self
            .pending
            .read()
            .unwrap()
            .pending
            .values()
            .map(|arc| (**arc).clone())
            .collect();

        let conflict_bundles: Vec<(SpendBundle, u64)> = self
            .conflict
            .read()
            .unwrap()
            .cache
            .values()
            .map(|(bundle, cost)| (bundle.clone(), *cost))
            .collect();

        let fee_estimator_state = self.fee_tracker.read().unwrap().to_state();

        MempoolSnapshot {
            active_items,
            pending_items,
            conflict_bundles,
            fee_estimator_state,
        }
    }

    /// Restore mempool state from a snapshot.
    ///
    /// Clears the active pool, pending pool, and conflict cache (the seen-cache
    /// is intentionally left unchanged), then rebuilds all indexes from the
    /// snapshot data. The fee estimator is fully replaced.
    ///
    /// After this call:
    /// - `len()` / `stats()` / `active_items()` reflect the snapshot state.
    /// - `pending_len()` reflects the snapshot state.
    /// - `conflict_len()` reflects the snapshot state.
    /// - `estimate_fee_rate()` returns results from the restored fee tracker.
    ///
    /// # Invariants
    ///
    /// All derived indexes (`coin_index`, `mempool_coins`, `dedup_index`, etc.)
    /// are rebuilt from the stored items via `ActivePool::insert()`.
    ///
    /// # Thread safety
    ///
    /// Acquires write locks on all four state components sequentially.
    ///
    /// See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)
    pub fn restore(&self, snap: MempoolSnapshot) {
        // ── Clear active pool ──
        {
            let mut pool = self.pool.write().unwrap();
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

        // ── Clear pending pool ──
        {
            let mut pending = self.pending.write().unwrap();
            pending.pending.clear();
            pending.pending_coin_index.clear();
            pending.pending_cost = 0;
        }

        // ── Clear conflict cache ──
        {
            let mut conflict = self.conflict.write().unwrap();
            conflict.cache.clear();
            conflict.total_cost = 0;
        }

        // ── Restore active items — insert() rebuilds all indexes ──
        {
            let mut pool = self.pool.write().unwrap();
            for item in snap.active_items {
                pool.insert(Arc::new(item));
            }
        }

        // ── Restore pending items ──
        {
            let mut pending = self.pending.write().unwrap();
            for item in snap.pending_items {
                pending.insert(Arc::new(item));
            }
        }

        // ── Restore conflict cache ──
        {
            let mut conflict = self.conflict.write().unwrap();
            for (bundle, cost) in snap.conflict_bundles {
                let id = bundle.name();
                conflict.total_cost = conflict.total_cost.saturating_add(cost);
                conflict.cache.insert(id, (bundle, cost));
            }
        }

        // ── Restore fee estimator ──
        let window = self.config.fee_estimator_window;
        let new_tracker = FeeTracker::from_state(snap.fee_estimator_state, window);
        *self.fee_tracker.write().unwrap() = new_tracker;
    }
}
