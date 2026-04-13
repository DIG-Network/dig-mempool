//! POL-004, POL-005 — Pending pool (timelocked item queue).
//!
//! Items land here when `assert_height > current_height` or
//! `assert_seconds > current_timestamp`. They are drained for re-submission
//! when `on_new_block()` advances the chain state.
//!
//! See: [`docs/requirements/domains/pools/specs/POL-004.md`]

use std::collections::HashMap;
use std::sync::Arc;

use dig_clvm::{Bytes32, SpendBundle};

use crate::item::MempoolItem;

/// Pending pool: validated but future-timelocked items awaiting promotion.
pub(crate) struct PendingPool {
    /// Bundle ID → item.
    pub(crate) pending: HashMap<Bytes32, Arc<MempoolItem>>,
    /// Spent coin ID → bundle ID (for POL-005 pending-vs-pending conflict detection).
    pub(crate) pending_coin_index: HashMap<Bytes32, Bytes32>,
    /// Sum of all pending items' `virtual_cost`.
    pub(crate) pending_cost: u64,
}

impl PendingPool {
    pub(crate) fn new() -> Self {
        Self {
            pending: HashMap::new(),
            pending_coin_index: HashMap::new(),
            pending_cost: 0,
        }
    }

    pub(crate) fn insert(&mut self, item: Arc<MempoolItem>) {
        let id = item.spend_bundle_id;
        for &coin_id in &item.removals {
            self.pending_coin_index.insert(coin_id, id);
        }
        self.pending_cost = self.pending_cost.saturating_add(item.virtual_cost);
        self.pending.insert(id, item);
    }

    pub(crate) fn remove(&mut self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        let item = self.pending.remove(bundle_id)?;
        for &coin_id in &item.removals {
            self.pending_coin_index.remove(&coin_id);
        }
        self.pending_cost = self.pending_cost.saturating_sub(item.virtual_cost);
        Some(item)
    }

    /// Drain all items whose timelocks are satisfied at `height` / `timestamp`.
    ///
    /// Returns the spend bundles for re-submission (each must be re-validated
    /// with current coin records and chain state).
    pub(crate) fn drain(&mut self, height: u64, timestamp: u64) -> Vec<SpendBundle> {
        let to_promote: Vec<Bytes32> = self
            .pending
            .values()
            .filter(|item| {
                let height_ok = item.assert_height.map_or(true, |h| h <= height);
                let seconds_ok = item.assert_seconds.map_or(true, |s| s <= timestamp);
                height_ok && seconds_ok
            })
            .map(|item| item.spend_bundle_id)
            .collect();

        to_promote
            .iter()
            .filter_map(|id| self.remove(id))
            .map(|item| item.spend_bundle.clone())
            .collect()
    }
}
