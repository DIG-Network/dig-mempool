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

use std::sync::{Arc, Mutex, RwLock};

use dig_clvm::BlsCache;
use dig_constants::NetworkConstants;

use crate::config::MempoolConfig;
use crate::fee::FeeTracker;
use crate::pools::{ActivePool, ConflictCache, PendingPool, SeenCache};
use crate::traits::MempoolEventHook;

pub(crate) mod fee;
pub(crate) mod lifecycle;
pub(crate) mod persistence;
pub(crate) mod query;
pub(crate) mod selection;
pub(crate) mod submit;

pub use persistence::MempoolSnapshot;

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
/// Holds all pool state: active pool (`items`, `coin_index`, `mempool_coins`,
/// dependency graph), pending pool, conflict cache, seen cache, BLS cache,
/// and fee tracker. All state is protected by interior `RwLock`s / `Mutex`.
///
/// See: [`config::MempoolConfig`] for all tuneable parameters.
pub struct Mempool {
    /// Network constants (genesis challenge, cost limits, AGG_SIG domains).
    /// From `dig-constants`. Passed through to dig-clvm for CLVM validation.
    /// Immutable after construction.
    pub(crate) constants: NetworkConstants,

    /// Mempool configuration. All capacity limits, fee thresholds, and
    /// feature flags. Immutable after construction.
    /// See: [`MempoolConfig`] and [API-003](docs/requirements/domains/crate_api/specs/API-003.md).
    pub(crate) config: MempoolConfig,

    /// Active pool: items HashMap, coin_index, mempool_coins, and accumulators.
    ///
    /// Protected by a single `RwLock` — read lock for queries, write lock for
    /// `submit()` insertion (Phase 2) and future removal operations. Grouping
    /// all related state in one lock prevents partial-update visibility and
    /// avoids multi-lock acquisition ordering issues.
    ///
    /// See: [POL-001](docs/requirements/domains/pools/specs/POL-001.md)
    pub(crate) pool: RwLock<ActivePool>,

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
    pub(crate) bls_cache: Mutex<BlsCache>,

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
    pub(crate) seen_cache: RwLock<SeenCache>,

    /// Pending pool: timelocked items awaiting height/timestamp advancement.
    ///
    /// Protected by its own `RwLock` to avoid contention with the active pool
    /// lock. Pending items do NOT participate in eviction or block selection.
    ///
    /// See: [POL-004](docs/requirements/domains/pools/specs/POL-004.md)
    pub(crate) pending: RwLock<PendingPool>,

    /// Conflict cache: bundles that lost active-pool RBF, stored for retry.
    ///
    /// Protected by its own `RwLock` so reads (e.g., `conflict_len()`) don't
    /// block active-pool writes and vice versa. Drained by `on_new_block()`.
    ///
    /// See: [POL-006](docs/requirements/domains/pools/specs/POL-006.md)
    pub(crate) conflict: RwLock<ConflictCache>,

    /// Event hooks registered via `add_event_hook()`.
    ///
    /// Called synchronously after state mutations. Hooks are fast, non-blocking
    /// callbacks used for logging, metrics, and external notifications.
    ///
    /// Multiple hooks can be registered. They are called in registration order.
    ///
    /// See: [LCY-005](docs/requirements/domains/lifecycle/specs/LCY-005.md)
    pub(crate) hooks: RwLock<Vec<Arc<dyn MempoolEventHook>>>,

    /// Historical fee-rate tracker for `estimate_fee_rate()`.
    ///
    /// Bucket-based tracker updated via `record_confirmed_block()` each block.
    /// Protected by its own RwLock so reads (`estimate_fee_rate`) can proceed
    /// concurrently with active-pool reads without contending on `pool`.
    ///
    /// See: [FEE-002](docs/requirements/domains/fee_estimation/specs/FEE-002.md)
    pub(crate) fee_tracker: RwLock<FeeTracker>,
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
        let fee_window = config.fee_estimator_window;
        let fee_buckets = config.fee_estimator_buckets;
        Self {
            constants,
            config,
            pool: RwLock::new(ActivePool::new()),
            bls_cache: Mutex::new(BlsCache::default()),
            seen_cache: RwLock::new(SeenCache::new(seen_cache_size)),
            pending: RwLock::new(PendingPool::new()),
            conflict: RwLock::new(ConflictCache::new()),
            hooks: RwLock::new(Vec::new()),
            fee_tracker: RwLock::new(FeeTracker::new(fee_window, fee_buckets)),
        }
    }

    /// Register an event hook to receive mempool mutation callbacks.
    ///
    /// Hooks are called synchronously after state mutations. Multiple hooks can
    /// be registered; they are called in registration order. Implementations MUST
    /// be fast and non-blocking (no I/O, no acquiring external locks).
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::{Arc, Mutex};
    /// use dig_mempool::{Mempool, MempoolEventHook, MempoolItem};
    /// use dig_mempool::Bytes32;
    /// use dig_constants::DIG_TESTNET;
    ///
    /// struct CountingHook { count: Mutex<usize> }
    /// impl MempoolEventHook for CountingHook {
    ///     fn on_item_added(&self, _item: &MempoolItem) {
    ///         *self.count.lock().unwrap() += 1;
    ///     }
    /// }
    ///
    /// let mempool = Mempool::new(DIG_TESTNET);
    /// mempool.add_event_hook(Arc::new(CountingHook { count: Mutex::new(0) }));
    /// ```
    ///
    /// See: [LCY-005](docs/requirements/domains/lifecycle/specs/LCY-005.md)
    pub fn add_event_hook(&self, hook: Arc<dyn MempoolEventHook>) {
        self.hooks.write().unwrap().push(hook);
    }

    /// Fire all registered hooks with the given closure.
    ///
    /// Acquires the hooks read lock and calls each hook in registration order.
    /// Panics in hook implementations are caught and silently discarded to prevent
    /// one misbehaving hook from corrupting mempool state.
    pub(crate) fn fire_hooks<F: Fn(&dyn MempoolEventHook)>(&self, f: F) {
        let hooks = self.hooks.read().unwrap();
        for hook in hooks.iter() {
            // Catch panics so one misbehaving hook can't corrupt mempool state.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                f(hook.as_ref());
            }));
        }
    }

    /// Insert a `MempoolItem` directly into the active pool, bypassing admission.
    ///
    /// ONLY for testing. The item is inserted as-is (no CLVM validation, no
    /// dedup, no capacity checks). Use when you need to populate the pool with
    /// items that have specific field values (e.g., `singleton_lineage`) that
    /// can only be set programmatically and not through the normal submit path.
    #[doc(hidden)]
    pub fn force_insert(&self, item: crate::item::MempoolItem) {
        self.pool.write().unwrap().insert(Arc::new(item));
    }
}
