//! Mempool configuration and constants.

use dig_clvm::L2_MAX_COST_PER_BLOCK;

/// Scaling factor for integer fee-per-cost comparisons. 10^12.
pub const FPC_SCALE: u128 = 1_000_000_000_000;

/// Number of blocks of cost capacity the active mempool holds.
pub const MEMPOOL_BLOCK_BUFFER: u64 = 15;

/// Default minimum fee increase for RBF, in mojos.
pub const MIN_RBF_FEE_BUMP: u64 = 10_000_000;

/// Default minimum FPC when mempool is near-full.
pub const FULL_MEMPOOL_MIN_FPC_SCALED: u128 = 5 * FPC_SCALE;

/// Default seen-cache capacity.
pub const DEFAULT_SEEN_CACHE_SIZE: usize = 10_000;

/// Default pending pool item count limit.
pub const DEFAULT_MAX_PENDING_COUNT: usize = 3_000;

/// Default conflict cache item count limit.
pub const DEFAULT_MAX_CONFLICT_COUNT: usize = 1_000;

/// Cost penalty per coin spend (matches Chia L1 SPEND_PENALTY_COST).
pub const SPEND_PENALTY_COST: u64 = 500_000;

/// Default expiry protection window.
pub const DEFAULT_EXPIRY_PROTECTION_BLOCKS: u64 = 100;

/// Default max CPFP depth.
pub const DEFAULT_MAX_DEPENDENCY_DEPTH: u32 = 25;

/// Default max spends per block.
pub const DEFAULT_MAX_SPENDS_PER_BLOCK: usize = 6_000;

/// Default fee estimator window.
pub const DEFAULT_FEE_ESTIMATOR_WINDOW: usize = 100;

/// Default fee estimator buckets.
pub const DEFAULT_FEE_ESTIMATOR_BUCKETS: usize = 50;

/// All tuneable mempool parameters with sensible defaults.
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    // -- Active pool limits --
    pub max_total_cost: u64,
    pub max_bundle_cost: u64,
    pub max_spends_per_block: usize,

    // -- Pending pool limits --
    pub max_pending_count: usize,
    pub max_pending_cost: u64,

    // -- Conflict cache limits --
    pub max_conflict_count: usize,
    pub max_conflict_cost: u64,

    // -- Fee / RBF --
    pub min_rbf_fee_bump: u64,
    pub full_mempool_min_fpc_scaled: u128,

    // -- Virtual cost --
    pub spend_penalty_cost: u64,

    // -- Expiry protection --
    pub expiry_protection_blocks: u64,

    // -- CPFP --
    pub max_dependency_depth: u32,

    // -- Deduplication --
    pub max_seen_cache_size: usize,

    // -- Fee estimation --
    pub fee_estimator_window: usize,
    pub fee_estimator_buckets: usize,

    // -- Singleton fast-forward --
    pub enable_singleton_ff: bool,

    // -- Identical spend dedup --
    pub enable_identical_spend_dedup: bool,
}

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

// Builder pattern — each with_* method consumes self and returns modified Self.
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
