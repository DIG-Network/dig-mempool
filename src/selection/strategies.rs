//! SEL-002..007 — Greedy selection strategies and best-set comparator.
//!
//! Four greedy strategies are run in parallel. The best resulting set is
//! chosen by the comparator in `sel_007_best`.
//!
//! See: [`docs/requirements/domains/selection/`]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dig_clvm::Bytes32;

use crate::item::MempoolItem;
use crate::pools::ActivePool;

/// Aggregated result of one greedy selection strategy.
pub(crate) struct SelectedSet {
    pub(crate) items: HashMap<Bytes32, Arc<MempoolItem>>,
    pub(crate) total_fees: u64,
    pub(crate) total_cost: u64,
    pub(crate) count: usize,
}

/// Enum identifying which sort key to apply.
pub(crate) enum SortStrategy {
    Density,
    Whale,
    Compact,
    Age,
}

/// SEL-002: Return `true` if the item is eligible for block inclusion at
/// `height` / `timestamp` (not expired, not future-timelocked).
pub(crate) fn sel_002_is_selectable(item: &MempoolItem, height: u64, timestamp: u64) -> bool {
    if let Some(abh) = item.assert_before_height {
        if abh <= height {
            return false;
        }
    }
    if let Some(abs) = item.assert_before_seconds {
        if abs <= timestamp {
            return false;
        }
    }
    if let Some(ah) = item.assert_height {
        if ah > height {
            return false;
        }
    }
    if let Some(as_) = item.assert_seconds {
        if as_ > timestamp {
            return false;
        }
    }
    true
}

/// Sort a mutable slice of items according to the given strategy.
pub(crate) fn sel_sort(candidates: &mut [Arc<MempoolItem>], strategy: &SortStrategy) {
    match strategy {
        SortStrategy::Density => {
            candidates.sort_by(|a, b| {
                b.package_fee_per_virtual_cost_scaled
                    .cmp(&a.package_fee_per_virtual_cost_scaled)
                    .then_with(|| b.fee.cmp(&a.fee))
                    .then_with(|| a.virtual_cost.cmp(&b.virtual_cost))
                    .then_with(|| a.height_added.cmp(&b.height_added))
                    .then_with(|| a.spend_bundle_id.as_ref().cmp(b.spend_bundle_id.as_ref()))
            });
        }
        SortStrategy::Whale => {
            candidates.sort_by(|a, b| {
                b.package_fee
                    .cmp(&a.package_fee)
                    .then_with(|| {
                        b.package_fee_per_virtual_cost_scaled
                            .cmp(&a.package_fee_per_virtual_cost_scaled)
                    })
                    .then_with(|| a.virtual_cost.cmp(&b.virtual_cost))
                    .then_with(|| a.height_added.cmp(&b.height_added))
                    .then_with(|| a.spend_bundle_id.as_ref().cmp(b.spend_bundle_id.as_ref()))
            });
        }
        SortStrategy::Compact => {
            candidates.sort_by(|a, b| {
                b.package_fee_per_virtual_cost_scaled
                    .cmp(&a.package_fee_per_virtual_cost_scaled)
                    .then_with(|| a.virtual_cost.cmp(&b.virtual_cost))
                    .then_with(|| b.fee.cmp(&a.fee))
                    .then_with(|| a.height_added.cmp(&b.height_added))
                    .then_with(|| a.spend_bundle_id.as_ref().cmp(b.spend_bundle_id.as_ref()))
            });
        }
        SortStrategy::Age => {
            candidates.sort_by(|a, b| {
                a.height_added
                    .cmp(&b.height_added)
                    .then_with(|| {
                        b.package_fee_per_virtual_cost_scaled
                            .cmp(&a.package_fee_per_virtual_cost_scaled)
                    })
                    .then_with(|| b.fee.cmp(&a.fee))
                    .then_with(|| a.spend_bundle_id.as_ref().cmp(b.spend_bundle_id.as_ref()))
            });
        }
    }
}

/// Collect all unselected direct and transitive ancestors of `bundle_id`.
pub(crate) fn sel_collect_ancestors(
    bundle_id: &Bytes32,
    deps: &HashMap<Bytes32, HashSet<Bytes32>>,
    already_selected: &HashMap<Bytes32, Arc<MempoolItem>>,
) -> Vec<Bytes32> {
    let mut result = Vec::new();
    let mut to_visit: Vec<Bytes32> = deps.get(bundle_id).into_iter().flatten().copied().collect();
    let mut visited: HashSet<Bytes32> = HashSet::new();
    while let Some(id) = to_visit.pop() {
        if already_selected.contains_key(&id) || !visited.insert(id) {
            continue;
        }
        result.push(id);
        to_visit.extend(deps.get(&id).into_iter().flatten().copied());
    }
    result
}

/// SEL-003..006: Run one greedy selection pass over `candidates` using
/// the given `SortStrategy`.
pub(crate) fn sel_greedy(
    candidates: &[Arc<MempoolItem>],
    pool: &ActivePool,
    candidates_set: &HashSet<Bytes32>,
    max_block_cost: u64,
    max_spends: usize,
    strategy: SortStrategy,
) -> SelectedSet {
    let mut sorted = candidates.to_vec();
    sel_sort(&mut sorted, &strategy);

    let mut selected: HashMap<Bytes32, Arc<MempoolItem>> = HashMap::new();
    let mut spent_coins: HashSet<Bytes32> = HashSet::new();
    let mut total_cost: u64 = 0;
    let mut total_fees: u64 = 0;
    let mut total_spends: usize = 0;

    'outer: for item in &sorted {
        if selected.contains_key(&item.spend_bundle_id) {
            continue;
        }

        let unselected_anc_ids =
            sel_collect_ancestors(&item.spend_bundle_id, &pool.dependencies, &selected);

        for anc_id in &unselected_anc_ids {
            if !candidates_set.contains(anc_id) {
                continue 'outer;
            }
        }

        let unselected_ancs: Vec<Arc<MempoolItem>> = unselected_anc_ids
            .iter()
            .filter_map(|id| pool.items.get(id))
            .cloned()
            .collect();

        // ── POL-009: Singleton chain all-or-nothing ──
        //
        // If this item belongs to a singleton chain, collect all unselected
        // successors in the chain (items after this one in lineage order).
        // All must fit in the budget; if not, skip the whole chain.
        let unselected_successor_ids: Vec<Bytes32> =
            if let Some(ref lineage) = item.singleton_lineage {
                if let Some(chain) = pool.singleton_spends.get(&lineage.launcher_id) {
                    let pos = chain.iter().position(|id| *id == item.spend_bundle_id);
                    if let Some(idx) = pos {
                        chain[idx + 1..]
                            .iter()
                            .copied()
                            .filter(|id| candidates_set.contains(id) && !selected.contains_key(id))
                            .collect()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

        for succ_id in &unselected_successor_ids {
            if !candidates_set.contains(succ_id) {
                continue 'outer;
            }
        }

        let unselected_succs: Vec<Arc<MempoolItem>> = unselected_successor_ids
            .iter()
            .filter_map(|id| pool.items.get(id))
            .cloned()
            .collect();

        let add_cost = item
            .virtual_cost
            .saturating_add(unselected_ancs.iter().map(|a| a.virtual_cost).sum::<u64>())
            .saturating_add(unselected_succs.iter().map(|s| s.virtual_cost).sum::<u64>());
        let add_spends = item.num_spends
            + unselected_ancs.iter().map(|a| a.num_spends).sum::<usize>()
            + unselected_succs.iter().map(|s| s.num_spends).sum::<usize>();

        if total_cost.saturating_add(add_cost) > max_block_cost {
            continue;
        }
        if total_spends.saturating_add(add_spends) > max_spends {
            continue;
        }

        let all_removals: Vec<Bytes32> = item
            .removals
            .iter()
            .chain(unselected_ancs.iter().flat_map(|a| a.removals.iter()))
            .chain(unselected_succs.iter().flat_map(|s| s.removals.iter()))
            .copied()
            .collect();
        if all_removals.iter().any(|r| spent_coins.contains(r)) {
            continue;
        }

        for anc in &unselected_ancs {
            selected.insert(anc.spend_bundle_id, anc.clone());
            spent_coins.extend(anc.removals.iter().copied());
            total_cost = total_cost.saturating_add(anc.virtual_cost);
            total_fees = total_fees.saturating_add(anc.fee);
            total_spends = total_spends.saturating_add(anc.num_spends);
        }
        selected.insert(item.spend_bundle_id, item.clone());
        spent_coins.extend(item.removals.iter().copied());
        total_cost = total_cost.saturating_add(item.virtual_cost);
        total_fees = total_fees.saturating_add(item.fee);
        total_spends = total_spends.saturating_add(item.num_spends);
        for succ in &unselected_succs {
            selected.insert(succ.spend_bundle_id, succ.clone());
            spent_coins.extend(succ.removals.iter().copied());
            total_cost = total_cost.saturating_add(succ.virtual_cost);
            total_fees = total_fees.saturating_add(succ.fee);
            total_spends = total_spends.saturating_add(succ.num_spends);
        }
    }

    let count = selected.len();
    SelectedSet {
        items: selected,
        total_fees,
        total_cost,
        count,
    }
}

/// SEL-007: Return the best `SelectedSet` among the four strategies.
///
/// Comparison: `total_fees DESC`, `total_cost ASC`, `count ASC`.
pub(crate) fn sel_007_best(sets: [&SelectedSet; 4]) -> &SelectedSet {
    let mut best = sets[0];
    for &s in &sets[1..] {
        let cmp = s
            .total_fees
            .cmp(&best.total_fees)
            .then_with(|| best.total_cost.cmp(&s.total_cost))
            .then_with(|| best.count.cmp(&s.count));
        if cmp == std::cmp::Ordering::Greater {
            best = s;
        }
    }
    best
}
