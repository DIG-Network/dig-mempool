//! POL-006 — Conflict cache (failed-RBF bundle store).
//!
//! Bundles that lost active-pool RBF are stored here for re-submission after
//! the conflicting active item is confirmed or evicted. Bounded by count and
//! aggregate estimated virtual cost.
//!
//! See: [`docs/requirements/domains/pools/specs/POL-006.md`]

use std::collections::HashMap;

use dig_clvm::{Bytes32, SpendBundle};

/// Conflict cache: raw SpendBundles that lost active-pool RBF.
///
/// Stores `SpendBundle` (not `MempoolItem`) because bundles need full
/// re-validation on retry.
///
/// Chia L1 equivalent: `ConflictTxCache` at
/// [pending_tx_cache.py:13](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/pending_tx_cache.py#L13)
pub(crate) struct ConflictCache {
    /// bundle_id → (spend_bundle, estimated_virtual_cost).
    pub(crate) cache: HashMap<Bytes32, (SpendBundle, u64)>,
    /// Aggregate estimated virtual cost of all cached bundles.
    pub(crate) total_cost: u64,
}

impl ConflictCache {
    pub(crate) fn new() -> Self {
        Self {
            cache: HashMap::new(),
            total_cost: 0,
        }
    }

    /// Insert a bundle into the conflict cache.
    ///
    /// Silently drops if the cache is at `max_count`, adding `estimated_cost`
    /// would exceed `max_cost`, or the bundle ID is already present.
    /// Returns `true` if inserted, `false` if dropped.
    pub(crate) fn insert(
        &mut self,
        bundle: SpendBundle,
        estimated_cost: u64,
        max_count: usize,
        max_cost: u64,
    ) -> bool {
        let id = bundle.name();
        if self.cache.contains_key(&id) {
            return false;
        }
        if self.cache.len() >= max_count {
            return false;
        }
        if self.total_cost.saturating_add(estimated_cost) > max_cost {
            return false;
        }
        self.total_cost = self.total_cost.saturating_add(estimated_cost);
        self.cache.insert(id, (bundle, estimated_cost));
        true
    }

    /// Drain all entries, resetting `total_cost` to 0.
    pub(crate) fn drain(&mut self) -> Vec<SpendBundle> {
        self.total_cost = 0;
        self.cache.drain().map(|(_, (bundle, _))| bundle).collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.cache.len()
    }

    pub(crate) fn contains(&self, id: &Bytes32) -> bool {
        self.cache.contains_key(id)
    }
}
