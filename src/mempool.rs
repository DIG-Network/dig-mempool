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

use std::sync::RwLock;

use dig_constants::NetworkConstants;

use crate::config::MempoolConfig;
use crate::stats::MempoolStats;

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
}
