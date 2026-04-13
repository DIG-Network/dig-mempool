//! POL-007 — Seen-cache (LRU-bounded dedup set).
//!
//! Bounded FIFO cache of recently seen bundle IDs used as DoS protection.
//! Insertions before CLVM validation mean even invalid re-submissions are
//! rejected without expensive re-execution.
//!
//! See: [`docs/requirements/domains/pools/specs/POL-007.md`]

use std::collections::{HashSet, VecDeque};

use dig_clvm::Bytes32;

/// Simple bounded FIFO seen-cache for bundle ID deduplication.
///
/// Uses a `HashSet` for O(1) lookups and a `VecDeque` to track insertion
/// order for FIFO eviction when capacity is exceeded.
pub(crate) struct SeenCache {
    /// O(1) lookup: is this bundle ID in the cache?
    pub(crate) entries: HashSet<Bytes32>,
    /// Insertion order for FIFO eviction (front = oldest).
    pub(crate) order: VecDeque<Bytes32>,
    /// Maximum number of entries before eviction.
    pub(crate) max_size: usize,
}

impl SeenCache {
    pub(crate) fn new(max_size: usize) -> Self {
        Self {
            entries: HashSet::with_capacity(max_size),
            order: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    pub(crate) fn contains(&self, id: &Bytes32) -> bool {
        self.entries.contains(id)
    }

    /// Insert a bundle ID. Evicts the oldest entry if at capacity (FIFO).
    /// Returns `true` if newly inserted, `false` if already present.
    pub(crate) fn insert(&mut self, id: Bytes32) -> bool {
        if self.entries.contains(&id) {
            return false;
        }
        while self.entries.len() >= self.max_size && self.max_size > 0 {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(id);
        self.order.push_back(id);
        true
    }

    /// Clear all entries (used by `Mempool::clear()` for reorg recovery).
    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}
