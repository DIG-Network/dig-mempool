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

use std::sync::{Arc, RwLock};

use dig_clvm::Bytes32;
use dig_constants::NetworkConstants;

use crate::config::MempoolConfig;
use crate::item::MempoolItem;
use crate::stats::MempoolStats;

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

    /// Active pool item count.
    /// Protected by RwLock for thread-safe access.
    /// Will be replaced by the full active pool HashMap in [POL-001].
    ///
    /// [POL-001]: docs/requirements/domains/pools/specs/POL-001.md
    active_count: RwLock<usize>,
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
        Self {
            constants,
            config,
            active_count: RwLock::new(0),
        }
    }

    /// Number of active (non-pending, non-conflicting) items in the mempool.
    ///
    /// Returns 0 for a newly constructed mempool.
    /// Thread-safe: acquires a read lock on the active pool.
    pub fn len(&self) -> usize {
        *self.active_count.read().unwrap()
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
        MempoolStats::empty(self.config.max_total_cost)
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
    /// The returned `Arc<MempoolItem>` is a cheap reference-counted pointer.
    pub fn get(&self, _bundle_id: &Bytes32) -> Option<Arc<MempoolItem>> {
        // TODO: Look up in active pool HashMap (POL-001)
        None
    }

    /// Check whether a bundle ID exists in any pool (active, pending, conflict).
    ///
    /// Returns `true` if found in any pool. Used for dedup checks and
    /// external status queries.
    pub fn contains(&self, _bundle_id: &Bytes32) -> bool {
        // TODO: Check active + pending + conflict (POL-001, POL-004, POL-006)
        false
    }

    /// Return all active (non-pending) bundle IDs.
    ///
    /// The order is not guaranteed. Use `select_for_block()` for ordered selection.
    pub fn active_bundle_ids(&self) -> Vec<Bytes32> {
        // TODO: Collect keys from active pool HashMap (POL-001)
        vec![]
    }

    /// Return all pending (timelocked) bundle IDs.
    pub fn pending_bundle_ids(&self) -> Vec<Bytes32> {
        // TODO: Collect keys from pending pool (POL-004)
        vec![]
    }

    /// Return all active mempool items as Arc references.
    ///
    /// Cheap to call — Arc clones are pointer copies.
    pub fn active_items(&self) -> Vec<Arc<MempoolItem>> {
        // TODO: Collect values from active pool HashMap (POL-001)
        vec![]
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
        // TODO: Read from pending pool (POL-004)
        0
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
    pub fn get_mempool_coin_creator(&self, _coin_id: &Bytes32) -> Option<Bytes32> {
        // TODO: Look up in mempool_coins index (CPF-001)
        None
    }
}
