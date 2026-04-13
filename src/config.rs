//! Mempool configuration, constants, and builder pattern.
//!
//! # Overview
//!
//! `MempoolConfig` contains all tuneable parameters for the dig-mempool crate.
//! It uses a builder pattern with `with_*` methods for ergonomic construction,
//! allowing callers to override only the parameters they care about.
//!
//! # Chia L1 Correspondence
//!
//! This corresponds to Chia's `MempoolInfo` passed to the `Mempool` constructor
//! ([mempool.py:107]). dig-mempool's config is more comprehensive due to CPFP,
//! dedup, fee estimation, and singleton FF features not present in Chia L1.
//!
//! # Usage
//!
//! ```rust
//! use dig_mempool::MempoolConfig;
//!
//! // Use all defaults (recommended for most cases):
//! let config = MempoolConfig::default();
//!
//! // Override specific parameters:
//! let config = MempoolConfig::default()
//!     .with_max_total_cost(1_000_000_000)
//!     .with_singleton_ff(false);
//! ```
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 2.4](../docs/resources/SPEC.md) — MempoolConfig definition
//! - [API-003](../docs/requirements/domains/crate_api/specs/API-003.md) — Requirement spec
//!
//! [mempool.py:107]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L107

use dig_clvm::L2_MAX_COST_PER_BLOCK;

// ── Named Constants ──
//
// Each constant corresponds to a row in the API-003 spec's "Default Values"
// table. Using named constants (rather than inline literals) ensures the
// Default impl and tests reference the same authoritative values.

/// Scaling factor for integer fee-per-cost comparisons.
///
/// Value: 10^12. Multiplied into the fee before dividing by virtual_cost
/// to avoid floating-point arithmetic. Chia uses float division
/// ([mempool_item.py:76-77]); we use integer math for determinism.
///
/// Formula: `fpc_scaled = (fee as u128 * FPC_SCALE) / (virtual_cost as u128)`
///
/// [mempool_item.py:76-77]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L76
pub const FPC_SCALE: u128 = 1_000_000_000_000;

/// Number of blocks of cost capacity the active mempool holds.
///
/// Value: 15. Total capacity = `L2_MAX_COST_PER_BLOCK * 15 = 8.25T`.
/// Chia uses `mempool_block_buffer = 10` in ConsensusConstants, but DIG L2
/// uses 15 to account for higher block throughput (550B vs 11B block cost).
///
/// Ref: `ConsensusConstants.mempool_block_buffer` in chia-consensus
pub const MEMPOOL_BLOCK_BUFFER: u64 = 15;

/// Default minimum absolute fee increase for replace-by-fee, in mojos.
///
/// Value: 10,000,000 (10M mojos = 0.00001 XCH equivalent).
/// Matches Chia L1's `MEMPOOL_MIN_FEE_INCREASE` at
/// [mempool_manager.py:52].
///
/// [mempool_manager.py:52]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L52
pub const MIN_RBF_FEE_BUMP: u64 = 10_000_000;

/// Default minimum FPC (scaled) when mempool is 80-100% full.
///
/// Value: `5 * FPC_SCALE` (= 5 mojos per cost unit, scaled).
/// Chia: `nonzero_fee_minimum_fpc = 5` at [mempool_manager.py:746].
///
/// When utilization is below 80%, zero-fee bundles are accepted.
/// Between 80-100%, bundles must meet this minimum FPC threshold.
///
/// [mempool_manager.py:746]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L746
pub const FULL_MEMPOOL_MIN_FPC_SCALED: u128 = 5 * FPC_SCALE;

/// Default seen-cache capacity (LRU bounded set for dedup).
///
/// Value: 10,000 bundle IDs. Added before CLVM validation to prevent
/// DoS via repeated submission of expensive-to-validate bundles.
pub const DEFAULT_SEEN_CACHE_SIZE: usize = 10_000;

/// Default pending pool item count limit.
///
/// Value: 3,000. Matches Chia's `PendingTxCache._cache_max_size` at
/// [pending_tx_cache.py:53].
///
/// [pending_tx_cache.py:53]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/pending_tx_cache.py#L53
pub const DEFAULT_MAX_PENDING_COUNT: usize = 3_000;

/// Default conflict cache item count limit.
///
/// Value: 1,000. Matches Chia's `ConflictTxCache._cache_max_size` at
/// [pending_tx_cache.py:15].
///
/// [pending_tx_cache.py:15]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/pending_tx_cache.py#L15
pub const DEFAULT_MAX_CONFLICT_COUNT: usize = 1_000;

/// Cost penalty per coin spend for virtual cost calculation.
///
/// Value: 500,000. Matches Chia L1's `SPEND_PENALTY_COST` at
/// [mempool_item.py:14].
///
/// `virtual_cost = cost + (num_spends * SPEND_PENALTY_COST)`
///
/// This penalizes transactions with many inputs to prevent spam
/// that is cheap in CLVM cost but expensive in validation/bandwidth.
///
/// [mempool_item.py:14]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L14
pub const SPEND_PENALTY_COST: u64 = 500_000;

/// Default expiry protection window in blocks.
///
/// Value: 100 blocks. Items within this many blocks of their
/// `assert_before_height` expiry are protected from eviction.
///
/// Chia uses 48 blocks / 900 seconds (~15 min) at [mempool.py:408-409].
/// DIG L2 uses 100 blocks (configurable).
///
/// [mempool.py:408-409]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L408
pub const DEFAULT_EXPIRY_PROTECTION_BLOCKS: u64 = 100;

/// Default maximum CPFP dependency chain depth.
///
/// Value: 25. A bundle spending a mempool-created coin has depth 1.
/// Deeper chains are rejected with `MempoolError::DependencyTooDeep`.
/// This is a dig-mempool extension (Chia does not support CPFP).
pub const DEFAULT_MAX_DEPENDENCY_DEPTH: u32 = 25;

/// Default maximum spends per block.
///
/// Value: 6,000. Matches Chia L1's `MAX_SPENDS_PER_BLOCK`.
pub const DEFAULT_MAX_SPENDS_PER_BLOCK: usize = 6_000;

/// Default fee estimator rolling window size in blocks.
///
/// Value: 100 blocks of history tracked for fee estimation.
pub const DEFAULT_FEE_ESTIMATOR_WINDOW: usize = 100;

/// Default number of fee-rate buckets for the fee estimator.
///
/// Value: 50 logarithmically spaced buckets.
pub const DEFAULT_FEE_ESTIMATOR_BUCKETS: usize = 50;

// ── MempoolConfig Struct ──

/// All tuneable mempool parameters with sensible defaults.
///
/// # Field Groups
///
/// | Group | Fields | Purpose |
/// |-------|--------|---------|
/// | Active pool | `max_total_cost`, `max_bundle_cost`, `max_spends_per_block` | Capacity limits |
/// | Pending pool | `max_pending_count`, `max_pending_cost` | Timelocked item limits |
/// | Conflict cache | `max_conflict_count`, `max_conflict_cost` | Failed-RBF retry limits |
/// | Fee / RBF | `min_rbf_fee_bump`, `full_mempool_min_fpc_scaled` | Fee thresholds |
/// | Virtual cost | `spend_penalty_cost` | Per-spend penalty |
/// | Eviction | `expiry_protection_blocks` | Expiry protection window |
/// | CPFP | `max_dependency_depth` | Chain depth limit |
/// | Dedup | `max_seen_cache_size` | LRU dedup cache size |
/// | Fee estimation | `fee_estimator_window`, `fee_estimator_buckets` | Tracker config |
/// | Features | `enable_singleton_ff`, `enable_identical_spend_dedup` | Feature flags |
///
/// # Builder Pattern
///
/// ```rust
/// use dig_mempool::MempoolConfig;
/// let config = MempoolConfig::default()
///     .with_max_total_cost(1_000_000_000)
///     .with_singleton_ff(false);
/// ```
///
/// See: [API-003](../docs/requirements/domains/crate_api/specs/API-003.md)
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    // ── Active pool limits ──
    /// Maximum aggregate CLVM cost across all active mempool items.
    /// Default: `L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER` (8.25T).
    /// Chia: `MempoolInfo.max_size_in_cost` ([mempool.py:509-514]).
    ///
    /// [mempool.py:509-514]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L509
    pub max_total_cost: u64,

    /// Maximum CLVM cost for a single spend bundle.
    /// Default: `L1_MAX_COST_PER_SPEND` (11B) from dig-clvm.
    /// Chia: `max_tx_clvm_cost` ([mempool_manager.py:733]).
    ///
    /// [mempool_manager.py:733]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L733
    pub max_bundle_cost: u64,

    /// Maximum coin spends per block. Default: 6,000.
    pub max_spends_per_block: usize,

    // ── Pending pool limits ──
    /// Max timelocked items. Default: 3,000.
    pub max_pending_count: usize,

    /// Max aggregate cost of timelocked items. Default: 1x block cost (550B).
    pub max_pending_cost: u64,

    // ── Conflict cache limits ──
    /// Max conflict cache items. Default: 1,000.
    pub max_conflict_count: usize,

    /// Max aggregate cost of conflict cache. Default: 1x block cost (550B).
    pub max_conflict_cost: u64,

    // ── Fee / RBF ──
    /// Minimum absolute fee increase for RBF. Default: 10M mojos.
    pub min_rbf_fee_bump: u64,

    /// Minimum FPC (scaled) when mempool is 80-100% full. Default: 5 * FPC_SCALE.
    pub full_mempool_min_fpc_scaled: u128,

    // ── Virtual cost ──
    /// Cost penalty per coin spend. Default: 500,000.
    /// `virtual_cost = cost + (num_spends * spend_penalty_cost)`
    pub spend_penalty_cost: u64,

    // ── Expiry protection ──
    /// Blocks before expiry within which items are eviction-protected. Default: 100.
    pub expiry_protection_blocks: u64,

    // ── CPFP ──
    /// Maximum dependency chain depth. Default: 25.
    pub max_dependency_depth: u32,

    // ── Deduplication ──
    /// Seen-cache LRU capacity. Default: 10,000.
    pub max_seen_cache_size: usize,

    // ── Fee estimation ──
    /// Rolling window of blocks for fee estimation. Default: 100.
    pub fee_estimator_window: usize,

    /// Number of fee-rate buckets. Default: 50.
    pub fee_estimator_buckets: usize,

    // ── Feature flags ──
    /// Enable singleton fast-forward optimization. Default: true.
    pub enable_singleton_ff: bool,

    /// Enable identical spend deduplication. Default: true.
    pub enable_identical_spend_dedup: bool,
}

/// Default implementation providing all values from the API-003 spec table.
///
/// Uses named constants (defined above) to ensure the same authoritative
/// values are used in both the implementation and tests.
impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            max_total_cost: L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER,
            max_bundle_cost: dig_clvm::L1_MAX_COST_PER_SPEND,
            max_spends_per_block: DEFAULT_MAX_SPENDS_PER_BLOCK,
            max_pending_count: DEFAULT_MAX_PENDING_COUNT,
            max_pending_cost: L2_MAX_COST_PER_BLOCK,
            max_conflict_count: DEFAULT_MAX_CONFLICT_COUNT,
            max_conflict_cost: L2_MAX_COST_PER_BLOCK,
            min_rbf_fee_bump: MIN_RBF_FEE_BUMP,
            full_mempool_min_fpc_scaled: FULL_MEMPOOL_MIN_FPC_SCALED,
            spend_penalty_cost: SPEND_PENALTY_COST,
            expiry_protection_blocks: DEFAULT_EXPIRY_PROTECTION_BLOCKS,
            max_dependency_depth: DEFAULT_MAX_DEPENDENCY_DEPTH,
            max_seen_cache_size: DEFAULT_SEEN_CACHE_SIZE,
            fee_estimator_window: DEFAULT_FEE_ESTIMATOR_WINDOW,
            fee_estimator_buckets: DEFAULT_FEE_ESTIMATOR_BUCKETS,
            enable_singleton_ff: true,
            enable_identical_spend_dedup: true,
        }
    }
}

/// Builder pattern — each `with_*` method consumes `self` and returns the
/// modified `Self`, enabling fluent chaining:
///
/// ```rust
/// # use dig_mempool::MempoolConfig;
/// let config = MempoolConfig::default()
///     .with_max_total_cost(1_000_000)
///     .with_singleton_ff(false);
/// ```
impl MempoolConfig {
    pub fn with_max_total_cost(mut self, cost: u64) -> Self {
        self.max_total_cost = cost;
        self
    }
    pub fn with_max_bundle_cost(mut self, cost: u64) -> Self {
        self.max_bundle_cost = cost;
        self
    }
    pub fn with_max_spends_per_block(mut self, count: usize) -> Self {
        self.max_spends_per_block = count;
        self
    }
    pub fn with_max_pending_count(mut self, count: usize) -> Self {
        self.max_pending_count = count;
        self
    }
    pub fn with_max_pending_cost(mut self, cost: u64) -> Self {
        self.max_pending_cost = cost;
        self
    }
    pub fn with_max_conflict_count(mut self, count: usize) -> Self {
        self.max_conflict_count = count;
        self
    }
    pub fn with_max_conflict_cost(mut self, cost: u64) -> Self {
        self.max_conflict_cost = cost;
        self
    }
    pub fn with_min_rbf_fee_bump(mut self, bump: u64) -> Self {
        self.min_rbf_fee_bump = bump;
        self
    }
    pub fn with_full_mempool_min_fpc_scaled(mut self, fpc: u128) -> Self {
        self.full_mempool_min_fpc_scaled = fpc;
        self
    }
    pub fn with_spend_penalty_cost(mut self, cost: u64) -> Self {
        self.spend_penalty_cost = cost;
        self
    }
    pub fn with_expiry_protection_blocks(mut self, blocks: u64) -> Self {
        self.expiry_protection_blocks = blocks;
        self
    }
    pub fn with_max_dependency_depth(mut self, depth: u32) -> Self {
        self.max_dependency_depth = depth;
        self
    }
    pub fn with_max_seen_cache_size(mut self, size: usize) -> Self {
        self.max_seen_cache_size = size;
        self
    }
    pub fn with_fee_estimator_window(mut self, window: usize) -> Self {
        self.fee_estimator_window = window;
        self
    }
    pub fn with_fee_estimator_buckets(mut self, buckets: usize) -> Self {
        self.fee_estimator_buckets = buckets;
        self
    }
    pub fn with_singleton_ff(mut self, enabled: bool) -> Self {
        self.enable_singleton_ff = enabled;
        self
    }
    pub fn with_identical_spend_dedup(mut self, enabled: bool) -> Self {
        self.enable_identical_spend_dedup = enabled;
        self
    }
}
