//! Core Mempool struct and constructors.
//!
//! # Overview
//!
//! The `Mempool` struct is the primary public interface for the crate.
//! It provides `submit()` for transaction admission, `select_for_block()`
//! for block candidate selection, and `on_new_block()` for lifecycle management.
//!
//! # Thread Safety
//!
//! `Mempool` uses interior mutability (`&self` methods with internal `RwLock`s)
//! so it can be shared across threads via `Arc<Mempool>`. This is Decision #1
//! from the spec — enables fine-grained locking where `select_for_block()`
//! (read) can run concurrently with incoming submissions (write).
//!
//! # Chia L1 Correspondence
//!
//! Combines the roles of Chia's `Mempool` class ([mempool.py:94]) and
//! `MempoolManager` class ([mempool_manager.py:295]). Chia separates
//! validation (Manager) from storage (Mempool); dig-mempool combines them
//! but uses a two-phase approach internally (lock-free CLVM validation in
//! Phase 1, locked state mutation in Phase 2).
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 3.1](docs/resources/SPEC.md) — Construction
//! - [API-001](docs/requirements/domains/crate_api/specs/API-001.md) — Requirement
//!
//! [mempool.py:94]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L94
//! [mempool_manager.py:295]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L295

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use dig_clvm::{BlsCache, Bytes32, SpendBundle};
use dig_constants::NetworkConstants;

use std::collections::VecDeque;

use crate::config::MempoolConfig;
use crate::error::MempoolError;
use crate::item::MempoolItem;
use crate::stats::MempoolStats;
use crate::submit::SubmitResult;

/// Simple bounded FIFO seen-cache for bundle ID deduplication.
///
/// Uses a `HashSet` for O(1) lookups and a `VecDeque` to track insertion
/// order for FIFO eviction when capacity is exceeded. This is simpler than
/// a true LRU but sufficient for the DoS protection use case — we only
/// need to reject recent duplicates, not perfectly track access patterns.
///
/// # Capacity and Eviction
///
/// When `entries.len() >= max_size`, the oldest entry (front of `order`)
/// is evicted before inserting the new one. This ensures memory is bounded.
///
/// See: [POL-007](docs/requirements/domains/pools/specs/POL-007.md)
struct SeenCache {
    /// O(1) lookup: is this bundle ID in the cache?
    entries: HashSet<Bytes32>,
    /// Insertion order for FIFO eviction (front = oldest).
    order: VecDeque<Bytes32>,
    /// Maximum number of entries before eviction.
    max_size: usize,
}

impl SeenCache {
    /// Create a new empty seen-cache with the given capacity.
    fn new(max_size: usize) -> Self {
        Self {
            entries: HashSet::with_capacity(max_size),
            order: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Check if a bundle ID is in the cache.
    fn contains(&self, id: &Bytes32) -> bool {
        self.entries.contains(id)
    }

    /// Insert a bundle ID into the cache.
    /// If the cache is full, evicts the oldest entry (FIFO).
    /// Returns true if the ID was newly inserted, false if already present.
    fn insert(&mut self, id: Bytes32) -> bool {
        if self.entries.contains(&id) {
            return false; // Already in cache
        }
        // Evict oldest if at capacity
        while self.entries.len() >= self.max_size && self.max_size > 0 {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(id);
        self.order.push_back(id);
        true
    }

    /// Clear all entries (used by Mempool::clear() for reorg recovery).
    #[allow(dead_code)]
    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

/// Internal active pool state — protected by `Mempool::pool: RwLock<ActivePool>`.
///
/// Groups the three related HashMaps and running accumulators under a single
/// RwLock. All modifications are atomic (all-or-nothing within the write lock),
/// which prevents partial-update visibility to concurrent readers.
///
/// # Data Structures
///
/// - `items`: Primary O(1) item lookup by bundle ID.
///   Chia L1 equivalent: `_items` dict at
///   [mempool.py:151](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L151)
/// - `coin_index`: O(1) conflict detection — maps each spent coin ID to the bundle
///   that spends it. Chia L1: `spends` SQL table at
///   [mempool.py:146-149](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L146)
/// - `mempool_coins`: O(1) CPFP resolution — maps each created coin ID to the bundle
///   that created it. dig-mempool extension (not present in Chia L1).
///
/// See: [POL-001](docs/requirements/domains/pools/specs/POL-001.md)
struct ActivePool {
    /// Primary store: bundle_id → Arc<MempoolItem>.
    ///
    /// `Arc` enables zero-copy sharing — callers hold references that remain
    /// valid even after the item is removed from the pool. All CRUD on the
    /// active pool goes through `insert()` and `remove()`.
    items: HashMap<Bytes32, Arc<MempoolItem>>,

    /// Conflict detection index: spent_coin_id → spending bundle_id.
    ///
    /// Populated on every `insert()`; cleaned on every `remove()`.
    /// Used by CFR-001 to detect coin conflicts in O(1): if a candidate
    /// bundle tries to spend a coin already in `coin_index`, it conflicts.
    coin_index: HashMap<Bytes32, Bytes32>,

    /// CPFP resolution index: created_coin_id → creating bundle_id.
    ///
    /// Populated on every `insert()` for each coin in `item.additions`;
    /// cleaned on every `remove()`. Used by CPF-001 to resolve CPFP parent
    /// bundles in O(1): the caller provides a synthetic CoinRecord for each
    /// coin returned here, enabling the CPFP child to reference parent output.
    mempool_coins: HashMap<Bytes32, Bytes32>,

    /// Running total of `item.virtual_cost` across all active items.
    ///
    /// Updated (+) on `insert()`, (-) on `remove()`. Used for capacity
    /// management: `total_cost + new_item.virtual_cost > max_total_cost`
    /// triggers eviction (POL-002).
    total_cost: u64,

    /// Running total of `item.fee` across all active items.
    ///
    /// Updated on insert/remove. Exposed via `stats().total_fees`.
    total_fees: u64,

    /// Running total of `item.num_spends` across all active items.
    ///
    /// Updated on insert/remove. Exposed via `stats().total_spend_count`.
    total_spends: usize,
}

impl ActivePool {
    /// Create an empty active pool with no items.
    fn new() -> Self {
        Self {
            items: HashMap::new(),
            coin_index: HashMap::new(),
            mempool_coins: HashMap::new(),
            total_cost: 0,
            total_fees: 0,
            total_spends: 0,
        }
    }

    /// Insert an item into the active pool.
    ///
    /// Updates `items`, `coin_index` (for each removal), `mempool_coins`
    /// (for each addition), and the running accumulators.
    ///
    /// # Preconditions (caller enforced)
    ///
    /// - No coin conflict: none of `item.removals` already in `coin_index`.
    ///   (Checked by CFR-001 before this is called.)
    /// - No duplicate: `item.spend_bundle_id` not already in `items`.
    ///
    /// # Complexity
    ///
    /// O(r + a) where r = `item.removals.len()` and a = `item.additions.len()`.
    ///
    /// Chia L1 equivalent: `Mempool.add_to_pool()` at
    /// [mempool.py:273](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L273)
    fn insert(&mut self, item: Arc<MempoolItem>) {
        let id = item.spend_bundle_id;

        // Register all spent coin IDs for O(1) conflict detection (CFR-001).
        for &coin_id in &item.removals {
            self.coin_index.insert(coin_id, id);
        }

        // Register all created coin IDs for O(1) CPFP resolution (CPF-001).
        for coin in &item.additions {
            self.mempool_coins.insert(coin.coin_id(), id);
        }

        // Update running accumulators (used for capacity management + stats).
        self.total_cost = self.total_cost.saturating_add(item.virtual_cost);
        self.total_fees = self.total_fees.saturating_add(item.fee);
        self.total_spends = self.total_spends.saturating_add(item.num_spends);

        self.items.insert(id, item);
    }

    /// Remove an item from the active pool by bundle ID.
    ///
    /// Cleans up `coin_index`, `mempool_coins`, and decrements accumulators.
    /// Returns the removed `Arc<MempoolItem>`, or `None` if not found.
    ///
    /// Called by `on_new_block()` (LCY-001) and capacity eviction (POL-002).
    ///
    /// # Complexity
    ///
    /// O(r + a) where r = number of removals and a = number of additions.
    fn remove(&mut self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        let item = self.items.remove(bundle_id)?;

        for &coin_id in &item.removals {
            self.coin_index.remove(&coin_id);
        }
        for coin in &item.additions {
            self.mempool_coins.remove(&coin.coin_id());
        }

        self.total_cost = self.total_cost.saturating_sub(item.virtual_cost);
        self.total_fees = self.total_fees.saturating_sub(item.fee);
        self.total_spends = self.total_spends.saturating_sub(item.num_spends);

        Some(item)
    }

    /// Evict lowest-`descendant_score` items to make room for `new_item`.
    ///
    /// Items are evicted in ascending `descendant_score` order (cheapest first).
    /// Eviction stops as soon as enough capacity is freed. If the new item's
    /// `fee_per_virtual_cost_scaled` is ≤ any evictable candidate's score, the
    /// item cannot displace it and `MempoolFull` is returned.
    ///
    /// Items with `assert_before_height` within `expiry_protection_blocks` of
    /// `current_height` are skipped (expiry protection — POL-003).
    ///
    /// # Chia L1 Correspondence
    ///
    /// Chia's `add_to_pool()` eviction loop at
    /// [mempool.py:395](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L395)
    ///
    /// See: [POL-002](docs/requirements/domains/pools/specs/POL-002.md)
    fn evict_for(
        &mut self,
        new_item: &Arc<MempoolItem>,
        max_total_cost: u64,
        current_height: u64,
        expiry_protection_blocks: u64,
    ) -> Result<(), MempoolError> {
        // Fast path: new item alone exceeds total capacity — nothing to evict helps.
        if new_item.virtual_cost > max_total_cost {
            return Err(MempoolError::MempoolFull);
        }

        // Collect evictable candidates (not expiry-protected), sorted ASC by score.
        // We snapshot IDs + scores now so the loop can mutate self.items via remove().
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
            // Enough space freed — stop evicting.
            if self.total_cost.saturating_add(new_item.virtual_cost) <= max_total_cost {
                break;
            }
            // New item can't beat this candidate — reject.
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

        // Final guard: did eviction free enough space?
        if self.total_cost.saturating_add(new_item.virtual_cost) > max_total_cost {
            return Err(MempoolError::MempoolFull);
        }

        Ok(())
    }

    /// Returns true if `item` is expiry-protected at `current_height`.
    ///
    /// An item is protected if it expires within `protection_blocks` blocks,
    /// meaning evicting it now would deny it any chance of confirmation.
    /// Protected items are skipped during capacity eviction.
    ///
    /// See: [POL-003](docs/requirements/domains/pools/specs/POL-003.md)
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

// ── Pending Pool ──────────────────────────────────────────────────────────

/// Pending pool: validated but future-timelocked items awaiting promotion.
///
/// Items land here when `assert_height > current_height` or
/// `assert_seconds > current_timestamp`. They are returned for re-submission
/// when `drain_pending()` is called (typically from `on_new_block()` — LCY-001).
///
/// See: [POL-004](docs/requirements/domains/pools/specs/POL-004.md)
struct PendingPool {
    /// Bundle ID → item.
    pending: HashMap<Bytes32, Arc<MempoolItem>>,
    /// Spent coin ID → bundle ID. Populated for pending-vs-pending conflict
    /// detection (POL-005). Cleaned up on removal.
    pending_coin_index: HashMap<Bytes32, Bytes32>,
    /// Sum of all pending items' `virtual_cost`.
    pending_cost: u64,
}

impl PendingPool {
    fn new() -> Self {
        Self {
            pending: HashMap::new(),
            pending_coin_index: HashMap::new(),
            pending_cost: 0,
        }
    }

    fn insert(&mut self, item: Arc<MempoolItem>) {
        let id = item.spend_bundle_id;
        for &coin_id in &item.removals {
            self.pending_coin_index.insert(coin_id, id);
        }
        self.pending_cost = self.pending_cost.saturating_add(item.virtual_cost);
        self.pending.insert(id, item);
    }

    fn remove(&mut self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        let item = self.pending.remove(bundle_id)?;
        for &coin_id in &item.removals {
            self.pending_coin_index.remove(&coin_id);
        }
        self.pending_cost = self.pending_cost.saturating_sub(item.virtual_cost);
        Some(item)
    }

    /// Drain all items whose timelocks are satisfied at `height` / `timestamp`.
    ///
    /// Returns the spend bundles for re-submission. Each bundle must be
    /// re-validated with fresh coin records and current chain state.
    fn drain(&mut self, height: u64, timestamp: u64) -> Vec<SpendBundle> {
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

// CoinRecord re-exported for get_mempool_coin_record return type.
use dig_clvm::CoinRecord;

/// Fee-prioritized, conflict-aware transaction mempool.
///
/// Thread-safe via interior mutability (`&self` methods with internal `RwLock`).
/// The struct is `Send + Sync`, allowing `Arc<Mempool>` sharing across threads.
///
/// # Construction
///
/// ```rust
/// use dig_mempool::Mempool;
/// use dig_constants::DIG_TESTNET;
///
/// let mempool = Mempool::new(DIG_TESTNET);
/// assert!(mempool.is_empty());
/// ```
///
/// # Internal State
///
/// Currently holds a minimal skeleton (config + active count). Full pool data
/// structures (items HashMap, coin_index, mempool_coins, dependency graph,
/// pending pool, conflict cache, seen cache, BLS cache, fee tracker) will be
/// added as their respective requirements are implemented.
///
/// See: [`config::MempoolConfig`] for all tuneable parameters.
pub struct Mempool {
    /// Network constants (genesis challenge, cost limits, AGG_SIG domains).
    /// From `dig-constants`. Passed through to dig-clvm for CLVM validation.
    /// Immutable after construction.
    #[allow(dead_code)]
    constants: NetworkConstants,

    /// Mempool configuration. All capacity limits, fee thresholds, and
    /// feature flags. Immutable after construction.
    /// See: [`MempoolConfig`] and [API-003](docs/requirements/domains/crate_api/specs/API-003.md).
    config: MempoolConfig,

    /// Active pool: items HashMap, coin_index, mempool_coins, and accumulators.
    ///
    /// Protected by a single `RwLock` — read lock for queries, write lock for
    /// `submit()` insertion (Phase 2) and future removal operations. Grouping
    /// all related state in one lock prevents partial-update visibility and
    /// avoids multi-lock acquisition ordering issues.
    ///
    /// See: [POL-001](docs/requirements/domains/pools/specs/POL-001.md)
    pool: RwLock<ActivePool>,

    /// BLS signature verification cache.
    /// Stores verified pairings for reuse across submissions, avoiding
    /// redundant elliptic curve operations. Protected by Mutex because
    /// `validate_spend_bundle()` needs `&mut BlsCache`.
    ///
    /// The cache is accessed during Phase 1 (lock-free CLVM validation).
    /// Multiple threads may contend on this mutex, but BLS verification
    /// is fast relative to CLVM execution.
    ///
    /// From `chia-bls` via dig-clvm re-export.
    /// See: [ADM-002](docs/requirements/domains/admission/specs/ADM-002.md)
    bls_cache: Mutex<BlsCache>,

    /// Seen-cache: LRU-bounded set of recently seen bundle IDs.
    ///
    /// Populated BEFORE CLVM validation as DoS protection — prevents
    /// an attacker from forcing repeated expensive CLVM execution of
    /// the same invalid bundle. Even failed submissions are cached.
    ///
    /// Capacity: `config.max_seen_cache_size` (default 10,000).
    /// Eviction: Oldest entries evicted when full (FIFO, not true LRU
    /// for simplicity — a proper LRU can be added later if needed).
    ///
    /// Chia equivalent: `MempoolManager.seen_bundle_hashes` at
    /// [mempool_manager.py:298](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L298)
    ///
    /// See: [ADM-003](docs/requirements/domains/admission/specs/ADM-003.md)
    seen_cache: RwLock<SeenCache>,

    /// Pending pool: timelocked items awaiting height/timestamp advancement.
    ///
    /// Protected by its own `RwLock` to avoid contention with the active pool
    /// lock. Pending items do NOT participate in eviction or block selection.
    ///
    /// See: [POL-004](docs/requirements/domains/pools/specs/POL-004.md)
    pending: RwLock<PendingPool>,
}

impl Mempool {
    /// Create a mempool with default configuration for the given network.
    ///
    /// Derives `max_total_cost` from the network's block cost limit:
    /// `L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER` (550B * 15 = 8.25T).
    ///
    /// All other parameters use `MempoolConfig::default()` values.
    ///
    /// # Chia Equivalent
    ///
    /// `Mempool.__init__(mempool_info, fee_estimator)` at
    /// [mempool.py:107](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L107)
    pub fn new(constants: NetworkConstants) -> Self {
        Self::with_config(constants, MempoolConfig::default())
    }

    /// Create a mempool with custom configuration.
    ///
    /// The provided `config` is used as-is. The caller is responsible for
    /// providing valid values (e.g., `max_total_cost > 0`).
    ///
    /// # Example
    ///
    /// ```rust
    /// use dig_mempool::{Mempool, MempoolConfig};
    /// use dig_constants::DIG_TESTNET;
    ///
    /// let config = MempoolConfig::default()
    ///     .with_max_total_cost(1_000_000_000);
    /// let mempool = Mempool::with_config(DIG_TESTNET, config);
    /// assert_eq!(mempool.stats().max_cost, 1_000_000_000);
    /// ```
    pub fn with_config(constants: NetworkConstants, config: MempoolConfig) -> Self {
        let seen_cache_size = config.max_seen_cache_size;
        Self {
            constants,
            config,
            pool: RwLock::new(ActivePool::new()),
            bls_cache: Mutex::new(BlsCache::default()),
            seen_cache: RwLock::new(SeenCache::new(seen_cache_size)),
            pending: RwLock::new(PendingPool::new()),
        }
    }

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

        // FPC range — 0 if the pool is empty.
        let min_fpc_scaled = pool
            .items
            .values()
            .map(|i| i.fee_per_virtual_cost_scaled)
            .min()
            .unwrap_or(0);
        let max_fpc_scaled = pool
            .items
            .values()
            .map(|i| i.fee_per_virtual_cost_scaled)
            .max()
            .unwrap_or(0);

        // CPFP metrics — only items with depth > 0 have real dependencies.
        let items_with_dependencies = pool.items.values().filter(|i| i.depth > 0).count();
        let max_current_depth = pool.items.values().map(|i| i.depth).max().unwrap_or(0);

        let dedup_eligible_count = pool.items.values().filter(|i| i.eligible_for_dedup).count();
        let singleton_ff_count = pool
            .items
            .values()
            .filter(|i| i.singleton_lineage.is_some())
            .count();

        let (pending_count, pending_cost) = {
            let pending = self.pending.read().unwrap();
            (pending.pending.len(), pending.pending_cost)
        };

        MempoolStats {
            active_count,
            pending_count,
            pending_cost,
            conflict_count: 0, // TODO: POL-006
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

    // ── Submission Methods (ADM-001) ──
    //
    // These are the primary entry points for transaction admission.
    // The full admission pipeline (CLVM validation, dedup, fee checks,
    // conflict detection, RBF, CPFP, capacity management) will be wired
    // in ADM-002 through ADM-007. For now, the signature is established.
    //
    // Mirrors Chia's `MempoolManager.add_spend_bundle()`:
    // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L538

    /// Validate and submit a spend bundle to the mempool.
    ///
    /// The mempool internally calls `dig_clvm::validate_spend_bundle()` for
    /// CLVM dry-run + BLS signature verification (ADM-002), then runs the
    /// full admission pipeline: dedup (ADM-003), fee check (ADM-004),
    /// virtual cost (ADM-005), timelock resolution (ADM-006), flag extraction
    /// (ADM-007), conflict detection (CFR-001), RBF (CFR-002-004), CPFP
    /// (CPF-001-005), capacity management (POL-002), and optional admission
    /// policy.
    ///
    /// # Parameters
    ///
    /// - `bundle`: The raw `SpendBundle` to validate and admit. Consumed by value.
    /// - `coin_records`: Coin state for every on-chain coin spent by the bundle.
    ///   For CPFP children, include synthetic records from `get_mempool_coin_record()`.
    /// - `current_height`: Current L2 block height (for timelock resolution).
    /// - `current_timestamp`: Current L2 block timestamp.
    ///
    /// # Returns
    ///
    /// - `Ok(SubmitResult::Success)` — admitted to active pool
    /// - `Ok(SubmitResult::Pending { assert_height })` — valid but timelocked
    /// - `Err(MempoolError)` — rejected (see variant for reason)
    ///
    /// # Concurrency
    ///
    /// Takes `&self` (not `&mut self`). Phase 1 (CLVM) runs lock-free;
    /// Phase 2 (insertion) acquires write lock briefly.
    ///
    /// See: [ADM-001](docs/requirements/domains/admission/specs/ADM-001.md)
    pub fn submit(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
    ) -> Result<SubmitResult, MempoolError> {
        // ── Phase 0: Dedup check (ADM-003) ──
        //
        // Check if we've already seen this bundle ID. This runs BEFORE
        // CLVM validation to prevent DoS via repeated submission of
        // expensive-to-validate bundles. The bundle ID is added to the
        // seen-cache immediately, even before validation — so invalid
        // bundles are also cached and rejected quickly on retry.
        //
        // Chia L1 equivalent: seen_bundle_hashes at mempool_manager.py:298
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L298

        let bundle_id = bundle.name();

        {
            // Read lock: check if already seen
            let seen = self.seen_cache.read().unwrap();
            if seen.contains(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }
        // TODO: Also check active_items, pending_items, conflict_cache (POL-001, POL-004, POL-006)

        {
            // Write lock: add to seen-cache BEFORE validation (DoS protection).
            // Even if CLVM validation fails, the ID stays in the cache to
            // prevent repeated validation of the same bad bundle.
            let mut seen = self.seen_cache.write().unwrap();
            // Re-check under write lock (another thread may have inserted)
            if seen.contains(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
            seen.insert(bundle_id);
        }

        // POL-001: Check active pool (fast rejection if bundle already active).
        // This handles the rare case where the seen_cache has evicted a bundle ID
        // that is still in the active pool. Without this check, a seen_cache miss
        // would allow re-validation and double-insertion of an already-active bundle.
        //
        // Does NOT acquire the pool write lock — read-only check before CLVM.
        // TOCTOU safety: Phase 2 re-checks under write lock before inserting.
        //
        // TODO: Also check pending_items (POL-004) and conflict_cache (POL-006).
        {
            let pool = self.pool.read().unwrap();
            if pool.items.contains_key(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }

        // ── Phase 1: Lock-free CLVM validation (ADM-002) ──
        //
        // This is the expensive step. It runs WITHOUT holding the mempool
        // write lock, allowing concurrent validation of multiple submissions.
        //
        // dig-clvm performs:
        //   1. Duplicate spend detection (validate.rs:39-48)
        //   2. Coin existence check (validate.rs:50-62)
        //   3. CLVM execution via chia_consensus::run_spendbundle() (validate.rs:79-102)
        //   4. BLS aggregate signature verification via BlsCache (validate.rs:107-113)
        //   5. Cost enforcement (validate.rs:126-131)
        //   6. Addition/removal extraction (validate.rs:134-146)
        //   7. Conservation check: sum(inputs) >= sum(outputs) (validate.rs:148-159)
        //
        // Chia L1 equivalent: validate_clvm_and_signature() at mempool_manager.py:445
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L445

        // Build the validation context from caller-provided chain state.
        // `coin_records` contains only the coins being spent (not the full UTXO set).
        // `ephemeral_coins` will be populated for CPFP candidates in CPF-002.
        let ctx = dig_clvm::ValidationContext {
            height: current_height as u32,
            timestamp: current_timestamp,
            constants: self.constants.clone(),
            coin_records: coin_records.clone(),
            ephemeral_coins: HashSet::new(), // TODO(CPF-002): populate for CPFP
        };

        // Use MEMPOOL_MODE for strict validation:
        // - Reject unknown condition opcodes
        // - Stricter cost accounting
        // - Canonical encoding checks (enables dedup eligibility flags)
        //
        // Chia L1: MEMPOOL_MODE at mempool.py:13
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L13
        let config = dig_clvm::ValidationConfig {
            flags: dig_clvm::chia_consensus::flags::MEMPOOL_MODE,
            ..Default::default()
        };

        // Acquire the BLS cache mutex for signature verification.
        // The mutex is only held during the validate_spend_bundle() call.
        // This serializes BLS pairing operations but CLVM execution is the
        // bottleneck, not signature verification.
        let mut bls_cache = self.bls_cache.lock().unwrap();

        // The ? operator converts dig_clvm::ValidationError to MempoolError
        // via the From<ValidationError> impl in error.rs (API-004).
        let spend_result =
            dig_clvm::validate_spend_bundle(&bundle, &ctx, &config, Some(&mut bls_cache))?;

        // Release the BLS cache before further processing
        drop(bls_cache);

        // ── ADM-004: Fee extraction + RESERVE_FEE check ──
        //
        // The fee is already computed by chia-consensus:
        //   fee = removal_amount - addition_amount
        // The RESERVE_FEE condition (opcode 52) is pre-summed in
        // conditions.reserve_fee. If the actual fee is less than the
        // declared minimum, reject the bundle.
        //
        // Chia L1 equivalent: mempool_manager.py:728
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L728
        let fee = spend_result.fee;
        let reserve_fee = spend_result.conditions.reserve_fee;
        if fee < reserve_fee {
            return Err(MempoolError::InsufficientFee {
                required: reserve_fee,
                available: fee,
            });
        }

        // ── ADM-005: Cost and virtual cost computation ──
        //
        // The CLVM cost is from chia-consensus. We also compute virtual cost
        // (adds spend penalty) and check against config.max_bundle_cost.
        //
        // Chia L1: max_tx_clvm_cost check at mempool_manager.py:733
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L733
        let cost = spend_result.conditions.cost;
        if cost > self.config.max_bundle_cost {
            return Err(MempoolError::CostExceeded {
                cost,
                max: self.config.max_bundle_cost,
            });
        }
        let num_spends = bundle.coin_spends.len();
        let virtual_cost = MempoolItem::compute_virtual_cost(cost, num_spends);
        let fee_per_virtual_cost_scaled = MempoolItem::compute_fpc_scaled(fee, virtual_cost);

        // ── ADM-006: Timelock resolution ──
        //
        // Resolve relative timelocks to absolute values using coin records.
        // Then check for impossible constraints and expiry.
        //
        // Chia L1 equivalent: compute_assert_height() at mempool_manager.py:81-126
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L81
        let mut assert_height: Option<u64> = None;
        let mut assert_seconds: Option<u64> = None;
        let mut assert_before_height: Option<u64> = None;
        let mut assert_before_seconds: Option<u64> = None;

        // Bundle-level absolute timelocks
        if spend_result.conditions.height_absolute > 0 {
            assert_height = Some(
                assert_height
                    .unwrap_or(0)
                    .max(spend_result.conditions.height_absolute as u64),
            );
        }
        if spend_result.conditions.seconds_absolute > 0 {
            assert_seconds = Some(
                assert_seconds
                    .unwrap_or(0)
                    .max(spend_result.conditions.seconds_absolute),
            );
        }
        if let Some(bha) = spend_result.conditions.before_height_absolute {
            let val = bha as u64;
            assert_before_height = Some(assert_before_height.map_or(val, |v: u64| v.min(val)));
        }
        if let Some(bsa) = spend_result.conditions.before_seconds_absolute {
            assert_before_seconds = Some(assert_before_seconds.map_or(bsa, |v: u64| v.min(bsa)));
        }

        // Per-spend relative timelocks resolved to absolute
        for spend_cond in &spend_result.conditions.spends {
            let coin_id = spend_cond.coin_id;
            // Look up the coin record for this spend's coin.
            // If not found (CPFP candidate), use current_height/timestamp
            // matching Chia's ephemeral coin handling (mempool_manager.py:716-722).
            let (confirmed_index, timestamp) = match coin_records.get(&coin_id) {
                Some(record) => (record.confirmed_block_index as u64, record.timestamp),
                None => (current_height, current_timestamp),
            };

            if let Some(hr) = spend_cond.height_relative {
                let resolved = confirmed_index + hr as u64;
                assert_height = Some(assert_height.unwrap_or(0).max(resolved));
            }
            if let Some(sr) = spend_cond.seconds_relative {
                let resolved = timestamp + sr;
                assert_seconds = Some(assert_seconds.unwrap_or(0).max(resolved));
            }
            if let Some(bhr) = spend_cond.before_height_relative {
                let resolved = confirmed_index + bhr as u64;
                assert_before_height =
                    Some(assert_before_height.map_or(resolved, |v: u64| v.min(resolved)));
            }
            if let Some(bsr) = spend_cond.before_seconds_relative {
                let resolved = timestamp + bsr;
                assert_before_seconds =
                    Some(assert_before_seconds.map_or(resolved, |v: u64| v.min(resolved)));
            }
        }

        // Impossible constraint detection
        // Chia: mempool_manager.py:791-796
        if let (Some(ah), Some(abh)) = (assert_height, assert_before_height) {
            if abh <= ah {
                return Err(MempoolError::ImpossibleTimelocks);
            }
        }
        if let (Some(as_), Some(abs)) = (assert_seconds, assert_before_seconds) {
            if abs <= as_ {
                return Err(MempoolError::ImpossibleTimelocks);
            }
        }

        // Expiry check
        if let Some(abh) = assert_before_height {
            if current_height >= abh {
                return Err(MempoolError::Expired);
            }
        }
        if let Some(abs) = assert_before_seconds {
            if current_timestamp >= abs {
                return Err(MempoolError::Expired);
            }
        }

        // Pending determination: if assert_height > current_height, route to pending
        let is_pending = matches!(assert_height, Some(ah) if ah > current_height)
            || matches!(assert_seconds, Some(as_) if as_ > current_timestamp);

        // ── ADM-007: Dedup/FF flag extraction ──
        //
        // Read ELIGIBLE_FOR_DEDUP (0x1) and ELIGIBLE_FOR_FF (0x4) flags from
        // each spend's conditions.flags. These are set by chia-consensus's
        // MempoolVisitor during CLVM execution when MEMPOOL_MODE is active.
        // The mempool reads these flags; it does NOT compute them.
        //
        // eligible_for_dedup: true only if ALL spends have the flag.
        // singleton_lineage: populated if any spend has ELIGIBLE_FOR_FF and
        // the caller has confirmed FF eligibility (not yet wired — needs
        // SingletonLayer::parse_puzzle() from chia-sdk-driver).
        //
        // Chia L1: mempool_manager.py:662-663 (dedup flag check)
        //          mempool_manager.py:666-667 (FF flag check)
        let eligible_for_dedup = spend_result
            .conditions
            .spends
            .iter()
            .all(|s| s.flags & 0x1 != 0)
            || spend_result.conditions.spends.is_empty(); // vacuously true for 0 spends
        let _any_ff_eligible = spend_result
            .conditions
            .spends
            .iter()
            .any(|s| s.flags & 0x4 != 0);
        // TODO: If _any_ff_eligible, extract singleton lineage info (needs chia-sdk-driver)
        // TODO(CFR-001): Conflict detection against coin_index

        // ── Build MempoolItem ──
        //
        // Construct the immutable MempoolItem from the validated data.
        // Built before routing so it can go to either the pending or active pool.
        //
        // `removals` = coin IDs spent by this bundle (from SpendResult.removals).
        // `additions` = coins created by this bundle (from SpendResult.additions).
        //
        // Chia L1 equivalent: Mempool.add_to_pool() at
        // [mempool.py:273](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L273)

        // Extract SpendResult fields (partial moves are safe — spend_result
        // is no longer borrowed from the timelock/flag loops above).
        let removals: Vec<Bytes32> = spend_result.removals.iter().map(|c| c.coin_id()).collect();
        let additions = spend_result.additions;
        let conditions = spend_result.conditions;

        let item = Arc::new(MempoolItem {
            spend_bundle: bundle,
            spend_bundle_id: bundle_id,
            fee,
            cost,
            virtual_cost,
            fee_per_virtual_cost_scaled,
            // Package fields equal individual fields for root items (no CPFP parent).
            // Will be updated by CPF-002 when ancestry resolution is implemented.
            package_fee: fee,
            package_virtual_cost: virtual_cost,
            package_fee_per_virtual_cost_scaled: fee_per_virtual_cost_scaled,
            descendant_score: fee_per_virtual_cost_scaled,
            additions,
            removals,
            height_added: current_height,
            conditions,
            num_spends,
            assert_height,
            assert_seconds,
            assert_before_height,
            assert_before_seconds,
            depends_on: HashSet::new(), // No CPFP dependencies for new items (CPF-002)
            depth: 0,
            eligible_for_dedup,
            singleton_lineage: None, // TODO: extract from _any_ff_eligible (chia-sdk-driver)
        });

        if is_pending {
            // ── Phase 2a: Route to pending pool (POL-004) ──
            //
            // The item is valid but timelocked. Insert into the pending pool
            // subject to count and cost limits. Returns PendingPoolFull if
            // either limit is exceeded.
            //
            // Chia L1: PendingTxCache.add() at pending_tx_cache.py:60-76
            let mut pending = self.pending.write().unwrap();
            // TOCTOU safety: re-check under write lock.
            if pending.pending.contains_key(&bundle_id) {
                return Ok(SubmitResult::Pending {
                    assert_height: assert_height.unwrap_or(0),
                });
            }

            // ── POL-005: Pending-vs-pending conflict detection + RBF ──
            //
            // If this item spends a coin already spent by a pending item,
            // apply RBF rules: superset check, FPC must be strictly higher,
            // fee bump must meet minimum. Failed RBF rejects the new item
            // without touching the conflict cache (pending RBF ≠ active RBF).
            let conflict_ids: Vec<Bytes32> = {
                let mut seen_conflicts = HashSet::new();
                item.removals
                    .iter()
                    .filter_map(|coin_id| pending.pending_coin_index.get(coin_id).copied())
                    .filter(|id| seen_conflicts.insert(*id))
                    .collect()
            };

            if !conflict_ids.is_empty() {
                // Superset rule: new item must spend every coin spent by each
                // conflicting item — it must be a strict superset of their removals.
                for cid in &conflict_ids {
                    let conflict = pending.pending.get(cid).unwrap();
                    for &coin_id in &conflict.removals {
                        if !item.removals.contains(&coin_id) {
                            return Err(MempoolError::RbfNotSuperset);
                        }
                    }
                }

                // FPC rule: aggregate all conflicting items and compare FPC.
                let total_conflict_fee: u64 = conflict_ids
                    .iter()
                    .map(|cid| pending.pending.get(cid).unwrap().fee)
                    .sum();
                let total_conflict_vc: u64 = conflict_ids
                    .iter()
                    .map(|cid| pending.pending.get(cid).unwrap().virtual_cost)
                    .sum();
                let conflict_fpc =
                    MempoolItem::compute_fpc_scaled(total_conflict_fee, total_conflict_vc);
                if item.fee_per_virtual_cost_scaled <= conflict_fpc {
                    return Err(MempoolError::RbfFpcNotHigher);
                }

                // Fee bump rule: new fee must exceed total conflict fee by at least
                // config.min_rbf_fee_bump (default 10M mojos).
                let required = total_conflict_fee.saturating_add(self.config.min_rbf_fee_bump);
                if item.fee < required {
                    return Err(MempoolError::RbfBumpTooLow {
                        required,
                        provided: item.fee,
                    });
                }

                // All RBF rules passed — evict the conflicting pending items.
                for cid in &conflict_ids {
                    pending.remove(cid);
                }
            }

            if pending.pending.len() >= self.config.max_pending_count {
                return Err(MempoolError::PendingPoolFull);
            }
            if pending.pending_cost.saturating_add(virtual_cost) > self.config.max_pending_cost {
                return Err(MempoolError::PendingPoolFull);
            }
            pending.insert(item);
            return Ok(SubmitResult::Pending {
                assert_height: assert_height.unwrap_or(0),
            });
        }

        // ── Phase 2b: Insert into active pool ──
        {
            let mut pool = self.pool.write().unwrap();
            // TOCTOU safety: re-check under write lock before inserting.
            // Another thread may have inserted the same bundle between our
            // Phase 0 check and now. Idempotent success avoids a double-insertion.
            if pool.items.contains_key(&bundle_id) {
                return Ok(SubmitResult::Success);
            }

            // ── POL-002: Spend count limit ──
            //
            // Hard cap on total active spends to bound block validation time.
            // Not subject to eviction — reject immediately if the budget is exhausted.
            //
            // Chia L1: maximum_spends check at mempool.py:385-390
            if pool.total_spends.saturating_add(num_spends) > self.config.max_spends_per_block {
                return Err(MempoolError::TooManySpends {
                    count: pool.total_spends.saturating_add(num_spends),
                    max: self.config.max_spends_per_block,
                });
            }

            // ── POL-002: Capacity management — evict if needed ──
            //
            // If the new item doesn't fit within max_total_cost, evict the
            // lowest-descendant_score items until there is room. Returns
            // MempoolFull if the new item can't displace any candidate.
            //
            // Chia L1 equivalent: add_to_pool() eviction loop at mempool.py:395
            if pool.total_cost.saturating_add(virtual_cost) > self.config.max_total_cost {
                pool.evict_for(
                    &item,
                    self.config.max_total_cost,
                    current_height,
                    self.config.expiry_protection_blocks,
                )?;
            }

            pool.insert(item);
        }

        Ok(SubmitResult::Success)
    }

    /// Submit with a custom admission policy applied after standard validation.
    ///
    /// Identical to `submit()` except the provided `policy` is invoked after
    /// all standard checks pass (Phase 2, step 16). If the policy returns
    /// `Err(reason)`, the bundle is rejected with `MempoolError::PolicyRejected`.
    ///
    /// # Parameters
    ///
    /// Same as `submit()` plus:
    /// - `policy`: A `&dyn AdmissionPolicy` that inspects the validated item
    ///   and current pool state to make a domain-specific admission decision.
    ///
    /// See: [API-007](docs/requirements/domains/crate_api/specs/API-007.md)
    pub fn submit_with_policy(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
        _policy: &dyn crate::traits::AdmissionPolicy,
    ) -> Result<SubmitResult, MempoolError> {
        // Same pipeline as submit() + policy check at step 16.
        // TODO: invoke _policy.check() after standard validation passes.
        self.submit(bundle, coin_records, current_height, current_timestamp)
    }

    /// Validate and submit multiple bundles in batch.
    ///
    /// Each bundle goes through the full admission pipeline independently.
    /// Results are returned in the same order as inputs. Earlier entries in
    /// the batch can create coins that later entries depend on (CPFP), and
    /// dedup applies across entries (identical bundles in the same batch
    /// will have the second rejected as AlreadySeen).
    ///
    /// # Concurrency Note
    ///
    /// Currently processes sequentially. Parallel Phase 1 (CLVM validation
    /// across bundles) will be added when rayon or similar is integrated.
    /// The sequential approach is correct and sufficient for initial release.
    ///
    /// See: [ADM-008](docs/requirements/domains/admission/specs/ADM-008.md)
    pub fn submit_batch(
        &self,
        bundles: Vec<SpendBundle>,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
    ) -> Vec<Result<SubmitResult, MempoolError>> {
        bundles
            .into_iter()
            .map(|bundle| self.submit(bundle, coin_records, current_height, current_timestamp))
            .collect()
    }

    // ── Query Methods (API-008) ──
    //
    // These methods provide read-only access to mempool state.
    // All use `&self` (not `&mut self`) and acquire read locks only,
    // enabling concurrent access alongside other reads.
    //
    // Currently return empty/default values since the pool data structures
    // (HashMap, coin_index, dependency graph) aren't built yet.
    // Behavioral implementations will be added in Phase 2 (POL-001+).
    //
    // See: [API-008](docs/requirements/domains/crate_api/specs/API-008.md)

    /// Look up an active mempool item by its spend bundle ID.
    ///
    /// Returns `None` if the bundle ID is not in the active pool.
    /// The returned `Arc<MempoolItem>` is a cheap reference-counted pointer —
    /// the item remains live as long as the Arc is held, even if the item is
    /// later removed from the pool.
    pub fn get(&self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        self.pool.read().unwrap().items.get(bundle_id).cloned()
    }

    /// Check whether a bundle ID exists in any pool (active, pending, conflict).
    ///
    /// Returns `true` if found in any pool. Used for dedup checks and
    /// external status queries.
    pub fn contains(&self, bundle_id: &Bytes32) -> bool {
        // POL-001: check active pool
        // TODO: Also check pending (POL-004) and conflict cache (POL-006)
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
    pub fn dependents_of(&self, _bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>> {
        // TODO: Look up in dependents graph (CPF-002)
        vec![]
    }

    /// Return all ancestors (parents, grandparents, ...) of a bundle.
    ///
    /// Walks the dependency chain transitively. Used for CPFP package
    /// analysis and cascade eviction planning.
    /// See: [CPF-002](docs/requirements/domains/cpfp/specs/CPF-002.md)
    pub fn ancestors_of(&self, _bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>> {
        // TODO: Walk dependencies graph transitively (CPF-002)
        vec![]
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
        // TODO: Read from conflict cache (POL-006)
        0
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
    pub fn get_mempool_coin_record(&self, _coin_id: &Bytes32) -> Option<CoinRecord> {
        // TODO: Look up in mempool_coins index (CPF-001)
        None
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
}
