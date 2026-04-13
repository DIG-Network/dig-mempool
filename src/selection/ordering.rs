//! SEL-008 — Topological ordering with fee-density tie-breaking.
//!
//! After the best greedy selection set is chosen, items are sorted by
//! topological layer (parents before children), then fee-per-virtual-cost
//! descending within each layer.
//!
//! See: [`docs/requirements/domains/selection/specs/SEL-008.md`]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dig_clvm::Bytes32;

use crate::item::MempoolItem;

use super::strategies::SelectedSet;

/// Compute the topological layer of `id` within `selected`, memoizing results.
///
/// Layer 0 = no selected parents.
/// Layer N = 1 + max(parent layers in selected).
pub(crate) fn sel_compute_layer(
    id: &Bytes32,
    deps: &HashMap<Bytes32, HashSet<Bytes32>>,
    selected: &HashSet<Bytes32>,
    memo: &mut HashMap<Bytes32, u32>,
) -> u32 {
    if let Some(&l) = memo.get(id) {
        return l;
    }
    let layer = match deps.get(id) {
        None => 0,
        Some(parents) => {
            let selected_parents: Vec<Bytes32> =
                parents.iter().filter(|p| selected.contains(p)).copied().collect();
            if selected_parents.is_empty() {
                0
            } else {
                let max_parent = selected_parents
                    .iter()
                    .map(|p| sel_compute_layer(p, deps, selected, memo))
                    .max()
                    .unwrap_or(0);
                max_parent + 1
            }
        }
    };
    memo.insert(*id, layer);
    layer
}

/// SEL-008: Apply topological + fee-density ordering to the selected set.
///
/// Sorted by `(layer ASC, fee_per_virtual_cost_scaled DESC,
/// height_added ASC, spend_bundle_id ASC)`.
pub(crate) fn sel_008_topological_order(
    selected: &SelectedSet,
    deps: &HashMap<Bytes32, HashSet<Bytes32>>,
) -> Vec<Arc<MempoolItem>> {
    let selected_ids: HashSet<Bytes32> = selected.items.keys().copied().collect();
    let mut memo: HashMap<Bytes32, u32> = HashMap::new();

    for id in &selected_ids {
        sel_compute_layer(id, deps, &selected_ids, &mut memo);
    }

    let mut result: Vec<Arc<MempoolItem>> = selected.items.values().cloned().collect();
    result.sort_by(|a, b| {
        let la = memo.get(&a.spend_bundle_id).copied().unwrap_or(0);
        let lb = memo.get(&b.spend_bundle_id).copied().unwrap_or(0);
        la.cmp(&lb)
            .then_with(|| {
                b.fee_per_virtual_cost_scaled
                    .cmp(&a.fee_per_virtual_cost_scaled)
            })
            .then_with(|| a.height_added.cmp(&b.height_added))
            .then_with(|| a.spend_bundle_id.as_ref().cmp(b.spend_bundle_id.as_ref()))
    });
    result
}
