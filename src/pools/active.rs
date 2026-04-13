//! POL-001..003, POL-008, CPF-001..002, CPF-006..007 — Active pool storage.
//!
//! The `ActivePool` holds the primary item store plus all the indexes needed
//! for O(1) conflict detection, CPFP dependency tracking, and dedup.
//!
//! All state is grouped under a single `RwLock<ActivePool>` in `Mempool` —
//! reads for queries, writes for admission and removal.
//!
//! See: [`docs/requirements/domains/pools/specs/POL-001.md`]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use dig_clvm::Bytes32;

use crate::error::MempoolError;
use crate::item::MempoolItem;

/// Compute SHA-256 of arbitrary bytes, returning a `Bytes32`.
///
/// Used by POL-008 to hash CoinSpend solution bytes into dedup index keys.
pub(crate) fn sha256_bytes(bytes: &[u8]) -> Bytes32 {
    let hash = Sha256::digest(bytes);
    let array: [u8; 32] = hash.into();
    Bytes32::from(array)
}

/// Internal active pool state — protected by `Mempool::pool: RwLock<ActivePool>`.
///
/// Groups the primary item store and all associated indexes under a single lock.
/// All modifications are atomic (all-or-nothing within the write lock), which
/// prevents partial-update visibility to concurrent readers.
///
/// # Data Structures
///
/// - `items`: Primary O(1) item lookup by bundle ID.
/// - `coin_index`: O(1) conflict detection (CFR-001).
/// - `mempool_coins`: O(1) CPFP resolution (CPF-001).
/// - `dependencies` / `dependents`: bidirectional CPFP graph (CPF-002..007).
/// - `dedup_index` / `dedup_waiters`: identical-spend dedup (POL-008).
pub(crate) struct ActivePool {
    /// Primary store: bundle_id → Arc<MempoolItem>.
    pub(crate) items: HashMap<Bytes32, Arc<MempoolItem>>,

    /// Conflict detection: spent_coin_id → spending bundle_id.
    pub(crate) coin_index: HashMap<Bytes32, Bytes32>,

    /// CPFP resolution: created_coin_id → creating bundle_id.
    pub(crate) mempool_coins: HashMap<Bytes32, Bytes32>,

    /// CPFP graph (parent direction): bundle_id → set of parent bundle_ids.
    pub(crate) dependencies: HashMap<Bytes32, HashSet<Bytes32>>,

    /// CPFP graph (child direction): bundle_id → set of child bundle_ids.
    pub(crate) dependents: HashMap<Bytes32, HashSet<Bytes32>>,

    /// Identical-spend dedup index: (coin_id, sha256(solution)) → cost-bearer bundle_id.
    pub(crate) dedup_index: HashMap<(Bytes32, Bytes32), Bytes32>,

    /// Dedup waiters: (coin_id, sha256(solution)) → ordered list of waiting bundle_ids.
    pub(crate) dedup_waiters: HashMap<(Bytes32, Bytes32), Vec<Bytes32>>,

    /// Running total of `item.virtual_cost` across all active items.
    pub(crate) total_cost: u64,

    /// Running total of `item.fee` across all active items.
    pub(crate) total_fees: u64,

    /// Running total of `item.num_spends` across all active items.
    pub(crate) total_spends: usize,
}

impl ActivePool {
    pub(crate) fn new() -> Self {
        Self {
            items: HashMap::new(),
            coin_index: HashMap::new(),
            mempool_coins: HashMap::new(),
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            dedup_index: HashMap::new(),
            dedup_waiters: HashMap::new(),
            total_cost: 0,
            total_fees: 0,
            total_spends: 0,
        }
    }

    /// Insert an item into the active pool.
    ///
    /// Updates `items`, `coin_index`, `mempool_coins`, dedup indexes,
    /// CPFP graph edges, and running accumulators.
    ///
    /// # Preconditions (caller enforced)
    ///
    /// - No coin conflict: none of `item.removals` already in `coin_index`.
    /// - No duplicate: `item.spend_bundle_id` not already in `items`.
    pub(crate) fn insert(&mut self, item: Arc<MempoolItem>) {
        let id = item.spend_bundle_id;

        // Register all spent coin IDs for O(1) conflict detection (CFR-001).
        for &coin_id in &item.removals {
            self.coin_index.insert(coin_id, id);
        }

        // Register all created coin IDs for O(1) CPFP resolution (CPF-001).
        for coin in &item.additions {
            self.mempool_coins.insert(coin.coin_id(), id);
        }

        // ── POL-008: Identical-spend dedup index ──
        for key in &item.dedup_keys {
            if self.dedup_index.contains_key(key) {
                self.dedup_waiters.entry(*key).or_default().push(id);
            } else {
                self.dedup_index.insert(*key, id);
            }
        }

        // ── CPF-002: Insert CPFP dependency graph edges ──
        for &parent_id in &item.depends_on {
            self.dependencies.entry(id).or_default().insert(parent_id);
            self.dependents.entry(parent_id).or_default().insert(id);
        }

        // Update running accumulators.
        self.total_cost = self.total_cost.saturating_add(item.virtual_cost);
        self.total_fees = self.total_fees.saturating_add(item.fee);
        self.total_spends = self.total_spends.saturating_add(item.num_spends);

        self.items.insert(id, item);
    }

    /// Remove an item from the active pool by bundle ID.
    ///
    /// Cleans up all indexes and decrements accumulators.
    /// Returns the removed `Arc<MempoolItem>`, or `None` if not found.
    pub(crate) fn remove(&mut self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        let item = self.items.remove(bundle_id)?;

        for &coin_id in &item.removals {
            self.coin_index.remove(&coin_id);
        }
        for coin in &item.additions {
            self.mempool_coins.remove(&coin.coin_id());
        }

        // ── POL-008: Dedup index cleanup and bearer re-assignment ──
        for key in &item.dedup_keys {
            if self.dedup_index.get(key) == Some(bundle_id) {
                let new_bearer = self
                    .dedup_waiters
                    .get_mut(key)
                    .and_then(|w| if w.is_empty() { None } else { Some(w.remove(0)) });

                match new_bearer {
                    Some(new_id) => {
                        self.dedup_index.insert(*key, new_id);
                        if self.dedup_waiters.get(key).map_or(true, |w| w.is_empty()) {
                            self.dedup_waiters.remove(key);
                        }
                    }
                    None => {
                        self.dedup_index.remove(key);
                        self.dedup_waiters.remove(key);
                    }
                }
            } else {
                if let Some(waiters) = self.dedup_waiters.get_mut(key) {
                    waiters.retain(|id| id != bundle_id);
                    if waiters.is_empty() {
                        self.dedup_waiters.remove(key);
                    }
                }
            }
        }

        // ── CPF-002: Clean CPFP dependency graph edges ──
        if let Some(parents) = self.dependencies.remove(bundle_id) {
            for parent_id in &parents {
                if let Some(children) = self.dependents.get_mut(parent_id) {
                    children.remove(bundle_id);
                    if children.is_empty() {
                        self.dependents.remove(parent_id);
                    }
                }
                self.recompute_descendant_score(parent_id);
            }
        }
        self.dependents.remove(bundle_id);

        self.total_cost = self.total_cost.saturating_sub(item.virtual_cost);
        self.total_fees = self.total_fees.saturating_sub(item.fee);
        self.total_spends = self.total_spends.saturating_sub(item.num_spends);

        Some(item)
    }

    /// Recompute `descendant_score` for `bundle_id` from its remaining children.
    ///
    /// Called after a child is removed (CPF-006).
    pub(crate) fn recompute_descendant_score(&mut self, bundle_id: &Bytes32) {
        let children_max: u128 = self
            .dependents
            .get(bundle_id)
            .map(|children| {
                children
                    .iter()
                    .filter_map(|cid| self.items.get(cid))
                    .map(|c| c.package_fee_per_virtual_cost_scaled)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let own_fpc = match self.items.get(bundle_id) {
            Some(i) => i.fee_per_virtual_cost_scaled,
            None => return,
        };
        let new_score = own_fpc.max(children_max);

        if let Some(existing) = self.items.get_mut(bundle_id) {
            if existing.descendant_score != new_score {
                let mut updated = (**existing).clone();
                updated.descendant_score = new_score;
                *existing = Arc::new(updated);
            }
        }
    }

    /// Walk ancestors of `new_bundle_id` and update `descendant_score` for each.
    ///
    /// Called after a new item is inserted (CPF-006). Early termination when
    /// no score improvement is possible.
    pub(crate) fn update_descendant_scores_on_add(&mut self, new_bundle_id: &Bytes32) {
        let pkg_fpc = match self.items.get(new_bundle_id) {
            Some(i) => i.package_fee_per_virtual_cost_scaled,
            None => return,
        };

        let init_parents: Vec<Bytes32> = self
            .dependencies
            .get(new_bundle_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        if init_parents.is_empty() {
            return;
        }

        let mut to_visit = init_parents;
        let mut visited: HashSet<Bytes32> = HashSet::new();

        while let Some(ancestor_id) = to_visit.pop() {
            if !visited.insert(ancestor_id) {
                continue;
            }

            let current_score = match self.items.get(&ancestor_id) {
                Some(i) => i.descendant_score,
                None => continue,
            };

            if pkg_fpc <= current_score {
                continue;
            }

            let grandparents: Vec<Bytes32> = self
                .dependencies
                .get(&ancestor_id)
                .into_iter()
                .flatten()
                .copied()
                .collect();

            if let Some(existing) = self.items.get_mut(&ancestor_id) {
                let mut updated = (**existing).clone();
                updated.descendant_score = pkg_fpc;
                *existing = Arc::new(updated);
            }

            to_visit.extend(grandparents);
        }
    }

    /// Recursively evict `bundle_id` and all its transitive dependents.
    ///
    /// Children are evicted before parents (depth-first).
    /// Returns all evicted bundle IDs.
    pub(crate) fn cascade_evict(&mut self, bundle_id: &Bytes32) -> Vec<Bytes32> {
        let mut evicted = Vec::new();
        self.cascade_evict_inner(bundle_id, &mut evicted);
        evicted
    }

    fn cascade_evict_inner(&mut self, bundle_id: &Bytes32, evicted: &mut Vec<Bytes32>) {
        let children: Vec<Bytes32> = self
            .dependents
            .get(bundle_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();

        for child_id in children {
            self.cascade_evict_inner(&child_id, evicted);
        }

        if self.remove(bundle_id).is_some() {
            evicted.push(*bundle_id);
        }
    }

    /// Evict lowest-`descendant_score` items to make room for `new_item`.
    ///
    /// Items protected by expiry (POL-003) are skipped unless the new item is
    /// also expiry-protected. Returns `Err(MempoolFull)` if space cannot be
    /// freed without displacing items with higher score than `new_item`.
    pub(crate) fn evict_for(
        &mut self,
        new_item: &Arc<MempoolItem>,
        max_total_cost: u64,
        current_height: u64,
        expiry_protection_blocks: u64,
    ) -> Result<(), MempoolError> {
        if new_item.virtual_cost > max_total_cost {
            return Err(MempoolError::MempoolFull);
        }

        let mut candidates: Vec<(u128, Bytes32)> = self
            .items
            .values()
            .filter(|item| {
                !Self::is_expiry_protected(item, current_height, expiry_protection_blocks)
            })
            .map(|item| (item.descendant_score, item.spend_bundle_id))
            .collect();
        candidates.sort_by_key(|(score, _)| *score);

        for (score, candidate_id) in &candidates {
            if self.total_cost.saturating_add(new_item.virtual_cost) <= max_total_cost {
                break;
            }
            if new_item.fee_per_virtual_cost_scaled <= *score {
                return Err(MempoolError::MempoolFull);
            }
            self.remove(candidate_id);
        }

        // Pass 2 (POL-003): if still not enough space and the new item is itself
        // expiry-protected, it may also evict protected items with lower FPC.
        if self.total_cost.saturating_add(new_item.virtual_cost) > max_total_cost
            && Self::is_expiry_protected(new_item, current_height, expiry_protection_blocks)
        {
            let mut protected: Vec<(u128, Bytes32)> = self
                .items
                .values()
                .filter(|item| {
                    Self::is_expiry_protected(item, current_height, expiry_protection_blocks)
                })
                .map(|item| (item.descendant_score, item.spend_bundle_id))
                .collect();
            protected.sort_by_key(|(score, _)| *score);

            for (score, candidate_id) in &protected {
                if self.total_cost.saturating_add(new_item.virtual_cost) <= max_total_cost {
                    break;
                }
                if new_item.fee_per_virtual_cost_scaled <= *score {
                    return Err(MempoolError::MempoolFull);
                }
                self.remove(candidate_id);
            }
        }

        if self.total_cost.saturating_add(new_item.virtual_cost) > max_total_cost {
            return Err(MempoolError::MempoolFull);
        }

        Ok(())
    }

    /// Returns true if `item` is expiry-protected at `current_height`.
    fn is_expiry_protected(
        item: &MempoolItem,
        current_height: u64,
        protection_blocks: u64,
    ) -> bool {
        if let Some(abh) = item.assert_before_height {
            return abh > current_height && abh <= current_height.saturating_add(protection_blocks);
        }
        false
    }
}
