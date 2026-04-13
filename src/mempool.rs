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

use dig_clvm::{chia_protocol::FeeRate, BlsCache, Bytes32, SpendBundle};
use serde::{Deserialize, Serialize};
use dig_constants::NetworkConstants;

use crate::config::{MempoolConfig, FPC_SCALE};
use crate::error::MempoolError;
use crate::fee::{FeeEstimatorState, FeeTracker, FeeTrackerStats};
use crate::item::MempoolItem;
use crate::pools::active::sha256_bytes;
use crate::pools::{ActivePool, ConflictCache, PendingPool, SeenCache};
use crate::selection::{
    sel_002_is_selectable, sel_007_best, sel_008_topological_order, sel_greedy, SortStrategy,
};
use crate::stats::MempoolStats;
use crate::submit::{ConfirmedBundleInfo, RetryBundles, SubmitResult};
use crate::traits::{MempoolEventHook, RemovalReason};

// CoinRecord re-exported for get_mempool_coin_record return type.
use dig_clvm::CoinRecord;

/// Serializable snapshot of complete mempool state.
///
/// Returned by [`Mempool::snapshot()`] and accepted by [`Mempool::restore()`].
/// The seen-cache is intentionally excluded: bundles that were seen before the
/// snapshot are not rejected as `AlreadySeen` after restore.
///
/// # JSON compatibility
///
/// `MempoolSnapshot` derives `Serialize + Deserialize`. Use `serde_json` to
/// persist the snapshot across restarts:
///
/// ```ignore
/// let json = serde_json::to_string(&mempool.snapshot())?;
/// mempool.restore(serde_json::from_str(&json)?);
/// ```
///
/// See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)
#[derive(Serialize, Deserialize)]
pub struct MempoolSnapshot {
    /// All active (non-pending, non-conflicting) items at snapshot time.
    pub active_items: Vec<MempoolItem>,
    /// All pending (timelocked) items at snapshot time.
    pub pending_items: Vec<MempoolItem>,
    /// Conflict-cache entries: `(SpendBundle, estimated_virtual_cost)`.
    pub conflict_bundles: Vec<(SpendBundle, u64)>,
    /// Fee estimator state: bucket statistics + block history.
    pub fee_estimator_state: FeeEstimatorState,
}

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

    /// Conflict cache: bundles that lost active-pool RBF, stored for retry.
    ///
    /// Protected by its own `RwLock` so reads (e.g., `conflict_len()`) don't
    /// block active-pool writes and vice versa. Drained by `on_new_block()`.
    ///
    /// See: [POL-006](docs/requirements/domains/pools/specs/POL-006.md)
    conflict: RwLock<ConflictCache>,

    /// Event hooks registered via `add_event_hook()`.
    ///
    /// Called synchronously after state mutations. Hooks are fast, non-blocking
    /// callbacks used for logging, metrics, and external notifications.
    ///
    /// Multiple hooks can be registered. They are called in registration order.
    ///
    /// See: [LCY-005](docs/requirements/domains/lifecycle/specs/LCY-005.md)
    hooks: RwLock<Vec<Arc<dyn MempoolEventHook>>>,

    /// Historical fee-rate tracker for `estimate_fee_rate()`.
    ///
    /// Bucket-based tracker updated via `record_confirmed_block()` each block.
    /// Protected by its own RwLock so reads (`estimate_fee_rate`) can proceed
    /// concurrently with active-pool reads without contending on `pool`.
    ///
    /// See: [FEE-002](docs/requirements/domains/fee_estimation/specs/FEE-002.md)
    fee_tracker: RwLock<FeeTracker>,
}

/// Build a child → direct-parent map for all descendants of `root`.
///
/// Walks `dependents` (parent → children) depth-first before eviction.
/// Used by `on_new_block()` so that `CascadeEvicted { parent_id }` hooks
/// reference the direct parent rather than the eviction root.
fn collect_descendants_parent_map(
    dependents: &HashMap<Bytes32, std::collections::HashSet<Bytes32>>,
    root: &Bytes32,
) -> HashMap<Bytes32, Bytes32> {
    let mut map = HashMap::new();
    let mut stack = vec![*root];
    while let Some(parent_id) = stack.pop() {
        if let Some(children) = dependents.get(&parent_id) {
            for &child_id in children {
                map.entry(child_id).or_insert(parent_id);
                stack.push(child_id);
            }
        }
    }
    map
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
    fn fire_hooks<F: Fn(&dyn MempoolEventHook)>(&self, f: F) {
        let hooks = self.hooks.read().unwrap();
        for hook in hooks.iter() {
            // Catch panics so one misbehaving hook can't corrupt mempool state.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                f(hook.as_ref());
            }));
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

        let conflict_count = self.conflict.read().unwrap().len();

        MempoolStats {
            active_count,
            pending_count,
            pending_cost,
            conflict_count,
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

        // POL-001 / POL-007: Check active pool, pending pool, and conflict cache.
        //
        // Handles the case where the seen_cache has evicted a bundle ID that is
        // still in another pool. Without these checks, a seen_cache miss would
        // allow re-validation and double-insertion of an already-present bundle.
        //
        // Each check acquires a read lock independently. TOCTOU safety: Phase 2
        // re-checks under the write lock before inserting into each pool.
        //
        // Chia L1: mempool_manager.py checks active pool at line ~560.
        {
            let pool = self.pool.read().unwrap();
            if pool.items.contains_key(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }
        {
            let pending = self.pending.read().unwrap();
            if pending.pending.contains_key(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }
        {
            let conflict = self.conflict.read().unwrap();
            if conflict.contains(&bundle_id) {
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

        // ── CPF-001/CPF-002: Snapshot mempool_coins for ephemeral coin support ──
        //
        // CPFP children spend coins created by unconfirmed parent bundles.
        // These coins are not in `coin_records` (they're not on-chain yet), but
        // the CLVM validator needs to know they exist to avoid CoinNotFound errors.
        //
        // A brief pool read lock snapshots the current set of mempool coin IDs.
        // Phase 2 re-validates under the write lock using the live index.
        // TOCTOU note: if a parent is evicted between this snapshot and Phase 2,
        // the Phase 2 dependency resolution will return CoinNotFound (correct).
        let ephemeral_coins: HashSet<Bytes32> = {
            let pool = self.pool.read().unwrap();
            pool.mempool_coins.keys().copied().collect()
        };

        // Build the validation context from caller-provided chain state.
        // `coin_records` contains on-chain coins; `ephemeral_coins` covers
        // unconfirmed coins from active mempool parents (CPFP support).
        let ctx = dig_clvm::ValidationContext {
            height: current_height as u32,
            timestamp: current_timestamp,
            constants: self.constants.clone(),
            coin_records: coin_records.clone(),
            ephemeral_coins,
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

        // ── Build MempoolItem (split by routing path) ──
        //
        // `removals` = coin IDs spent by this bundle (from SpendResult.removals).
        // `additions` = coins created by this bundle (from SpendResult.additions).
        //
        // Item construction is split between the pending and active paths because
        // the active path (POL-008) needs to compute cost_saving under the pool
        // write lock — requiring it to be built inside the lock scope.
        //
        // Chia L1 equivalent: Mempool.add_to_pool() at
        // [mempool.py:273](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L273)

        // Extract SpendResult fields (partial moves are safe — spend_result
        // is no longer borrowed from the timelock/flag loops above).
        let removals: Vec<Bytes32> = spend_result.removals.iter().map(|c| c.coin_id()).collect();
        let additions = spend_result.additions;
        let conditions = spend_result.conditions;

        // ── POL-008: Pre-compute dedup keys (lock-free) ──
        //
        // Compute (coin_id, sha256(solution)) keys for each eligible CoinSpend.
        // Done BEFORE `bundle` is consumed (item construction moves it).
        // Only meaningful for active-pool submissions — pending items don't
        // participate in dedup (POL-008 applies to the active pool only).
        //
        // We hash the solution bytes using raw SHA-256 (not CLVM tree hash)
        // to match the Chia L1 `IdenticalSpendDedup` implementation.
        let dedup_keys: Vec<(Bytes32, Bytes32)> =
            if !is_pending && eligible_for_dedup && self.config.enable_identical_spend_dedup {
                bundle
                    .coin_spends
                    .iter()
                    .map(|cs| (cs.coin.coin_id(), sha256_bytes(cs.solution.as_ref())))
                    .collect()
            } else {
                vec![]
            };

        if is_pending {
            // ── Phase 2a: Route to pending pool (POL-004) ──
            //
            // Build the item before acquiring the lock — item construction is cheap
            // and the pending path requires removals/fee/fpc for RBF checks.
            // Pending items do NOT participate in dedup (POL-008 is active pool only).
            //
            // Chia L1: PendingTxCache.add() at pending_tx_cache.py:60-76
            let item = Arc::new(MempoolItem {
                spend_bundle: bundle,
                spend_bundle_id: bundle_id,
                fee,
                cost,
                virtual_cost,
                fee_per_virtual_cost_scaled,
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
                depends_on: HashSet::new(),
                depth: 0,
                eligible_for_dedup,
                singleton_lineage: None,
                cost_saving: 0,
                effective_virtual_cost: virtual_cost,
                dedup_keys: vec![],
            });

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
            let item_ref = Arc::clone(&item);
            pending.insert(item);
            drop(pending);
            let pending_height = assert_height.unwrap_or(0);
            self.fire_hooks(|h| h.on_pending_added(&item_ref));
            return Ok(SubmitResult::Pending {
                assert_height: pending_height,
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

            // ── CPF-002: Dependency resolution ──
            //
            // For each coin this bundle spends (each removal):
            // - If the coin is in `coin_records` (on-chain) → no CPFP dependency.
            // - If the coin is in `pool.mempool_coins` (unconfirmed) → CPFP dep.
            // - If the coin is in neither → reject with CoinNotFound.
            //
            // This is a dig-mempool extension; Chia L1 does not support CPFP.
            let mut depends_on: HashSet<Bytes32> = HashSet::new();
            for &coin_id in &removals {
                if coin_records.contains_key(&coin_id) {
                    continue; // on-chain coin: no CPFP dependency
                }
                match pool.mempool_coins.get(&coin_id) {
                    Some(&parent_id) => {
                        depends_on.insert(parent_id);
                    }
                    None => {
                        return Err(MempoolError::CoinNotFound(coin_id));
                    }
                }
            }

            // ── CPF-002: Compute dependency depth ──
            //
            // depth = 0 for on-chain-only items; depth = 1 + max(parent.depth)
            // for CPFP children. Uses `pool.items` which already holds parents.
            let depth: u32 = if depends_on.is_empty() {
                0
            } else {
                1 + depends_on
                    .iter()
                    .filter_map(|id| pool.items.get(id))
                    .map(|p| p.depth)
                    .max()
                    .unwrap_or(0)
            };

            // ── CPF-003: Maximum dependency depth ──
            //
            // Reject if depth exceeds the configured limit (default 25).
            // Keeps cascade eviction, package fee computation, and block
            // selection traversals bounded.
            if depth > self.config.max_dependency_depth {
                return Err(MempoolError::DependencyTooDeep {
                    depth,
                    max: self.config.max_dependency_depth,
                });
            }

            // ── CPF-004: Defensive cycle check ──
            //
            // Walk the ancestor chain from this bundle's direct parents.
            // If we encounter `bundle_id` itself → DependencyCycle.
            // In the UTXO coinset model, cycles are structurally impossible
            // (requiring a SHA-256 hash collision). This check guards against
            // implementation bugs or corrupted state.
            if !depends_on.is_empty() {
                let mut to_visit: Vec<Bytes32> = depends_on.iter().copied().collect();
                let mut visited: HashSet<Bytes32> = HashSet::new();
                while let Some(ancestor_id) = to_visit.pop() {
                    if ancestor_id == bundle_id {
                        return Err(MempoolError::DependencyCycle);
                    }
                    if !visited.insert(ancestor_id) {
                        continue;
                    }
                    if let Some(ancestor_item) = pool.items.get(&ancestor_id) {
                        to_visit.extend(ancestor_item.depends_on.iter().copied());
                    }
                }
            }

            // ── CPF-005: Package fee rate computation ──
            //
            // Aggregate fees and costs across the entire ancestor chain.
            // Uses each parent's `package_fee` / `package_virtual_cost` so that
            // transitive ancestors are included without explicit traversal.
            let (package_fee, package_virtual_cost) = if depends_on.is_empty() {
                (fee, virtual_cost)
            } else {
                let ancestor_fee: u64 = depends_on
                    .iter()
                    .filter_map(|id| pool.items.get(id))
                    .map(|p| p.package_fee)
                    .sum();
                let ancestor_vc: u64 = depends_on
                    .iter()
                    .filter_map(|id| pool.items.get(id))
                    .map(|p| p.package_virtual_cost)
                    .sum();
                (
                    fee.saturating_add(ancestor_fee),
                    virtual_cost.saturating_add(ancestor_vc),
                )
            };
            let package_fpc_scaled =
                MempoolItem::compute_fpc_scaled(package_fee, package_virtual_cost);

            // ── CFR-001 through CFR-005: Active pool conflict detection + RBF ──
            //
            // For each coin spent by the new bundle, look up `coin_index` to find
            // any existing active bundle spending the same coin. Collect all unique
            // conflicting bundle IDs.
            //
            // If conflicts are found, evaluate RBF rules (CFR-002, CFR-003, CFR-004):
            //   1. Superset rule: new bundle must spend every coin from each conflict.
            //   2. FPC strictly higher: new bundle's fee rate exceeds aggregate rate.
            //   3. Minimum fee bump: absolute fee exceeds sum of conflicts + MIN_RBF.
            //
            // On failure: add new bundle to conflict cache (CFR-005) and return error.
            // On success: remove conflicting items (CFR-006 partial — cascade eviction
            //   of CPFP dependents is deferred to CPF-007).
            //
            // Lock note: `add_to_conflict_cache()` acquires `self.conflict.write()` while
            // `pool` (self.pool.write()) is held. This follows the POL-010 ordering:
            //   pool_lock → conflict_lock.
            //
            // Chia L1 equivalents:
            //   check_removals(): mempool_manager.py:229-292
            //   can_replace():    mempool_manager.py:1077-1126
            {
                // Build a set of dedup keys for O(1) lookup during conflict
                // filtering.  A "conflict" that is actually a dedup-waiter
                // relationship (same coin, same solution, dedup-eligible) must
                // NOT go through RBF — the bundle is admitted as a waiter.
                // Chia handles the same special case at mempool_manager.py:270-288.
                let dedup_key_set: HashSet<(Bytes32, Bytes32)> =
                    dedup_keys.iter().copied().collect();

                let mut seen_conflicts = HashSet::new();
                let conflict_ids: Vec<Bytes32> = removals
                    .iter()
                    .filter_map(|coin_id| {
                        let conflict_bundle_id = pool.coin_index.get(coin_id).copied()?;
                        // If the incoming bundle's spend of this coin is a dedup match
                        // (same solution already indexed in dedup_index), treat as a
                        // waiter, not a conflict — exclude from RBF evaluation.
                        if !dedup_key_set.is_empty() {
                            // Find the solution hash for this coin in the incoming bundle.
                            // dedup_key_set is keyed by (coin_id, sol_hash) so we need
                            // to search for any entry whose first element is *coin_id*.
                            let is_dedup = dedup_keys.iter().any(|(cid, sol_hash)| {
                                cid == coin_id && pool.dedup_index.contains_key(&(*cid, *sol_hash))
                            });
                            if is_dedup {
                                return None; // waiter — not a real conflict
                            }
                        }
                        Some(conflict_bundle_id)
                    })
                    .filter(|id| seen_conflicts.insert(*id))
                    .collect();

                if !conflict_ids.is_empty() {
                    // Fetch all conflicting items for RBF evaluation.
                    // coin_index is kept in sync with items, so all IDs resolve.
                    let conflicting_items: Vec<Arc<MempoolItem>> = conflict_ids
                        .iter()
                        .filter_map(|id| pool.items.get(id).cloned())
                        .collect();

                    let total_conflict_fee: u64 = conflicting_items.iter().map(|i| i.fee).sum();
                    let total_conflict_vc: u64 =
                        conflicting_items.iter().map(|i| i.virtual_cost).sum();

                    // ── CFR-002: Superset rule ──
                    //
                    // Every coin spent by any conflicting bundle must also appear in the
                    // new bundle's removals. Prevents "freeing" a conflicting bundle's
                    // coins while only partially replacing it.
                    let new_removals_set: HashSet<Bytes32> = removals.iter().copied().collect();
                    for conflict_item in &conflicting_items {
                        for &coin_id in &conflict_item.removals {
                            if !new_removals_set.contains(&coin_id) {
                                self.add_to_conflict_cache(bundle, virtual_cost);
                                return Err(MempoolError::RbfNotSuperset);
                            }
                        }
                    }

                    // ── CFR-003: FPC must be strictly higher than aggregate ──
                    //
                    // The new bundle's fee-per-virtual-cost must strictly exceed the
                    // combined FPC of all conflicting items. Uses scaled integer
                    // arithmetic (FPC_SCALE) to avoid floating-point non-determinism.
                    //
                    // Chia L1: uses fee_per_cost (float); we use scaled integers.
                    let conflict_fpc =
                        MempoolItem::compute_fpc_scaled(total_conflict_fee, total_conflict_vc);
                    if fee_per_virtual_cost_scaled <= conflict_fpc {
                        self.add_to_conflict_cache(bundle, virtual_cost);
                        return Err(MempoolError::RbfFpcNotHigher);
                    }

                    // ── CFR-004: Minimum absolute fee bump ──
                    //
                    // The new bundle's absolute fee must exceed the sum of all conflicting
                    // fees by at least `min_rbf_fee_bump` (default 10M mojos, matching
                    // Chia's MEMPOOL_MIN_FEE_INCREASE). Prevents "penny-shaving" DoS.
                    let required_fee =
                        total_conflict_fee.saturating_add(self.config.min_rbf_fee_bump);
                    if fee < required_fee {
                        self.add_to_conflict_cache(bundle, virtual_cost);
                        return Err(MempoolError::RbfBumpTooLow {
                            required: required_fee,
                            provided: fee,
                        });
                    }

                    // ── CFR-006 + CPF-007: Remove conflicting items with cascade ──
                    //
                    // All RBF conditions satisfied — evict the conflicting active items
                    // and recursively evict all their CPFP dependents (CPF-007).
                    // Cascaded children are irrecoverably invalid (their input coins no
                    // longer exist) and do NOT go to the conflict cache.
                    //
                    // Chia L1 equivalent: remove_from_pool() at mempool.py:303-349
                    for conflict_id in &conflict_ids {
                        pool.cascade_evict(conflict_id);
                    }
                }
            }

            // ── POL-008: Compute cost_saving under pool write lock ──
            //
            // Check how many of this item's dedup keys are already borne by
            // another active bundle. Each matching key saves approximately
            // `cost / num_spends` in effective block cost (uniform distribution
            // approximation — chia-consensus 0.26 does not expose per-spend cost).
            //
            // This must be computed under the write lock so that the dedup_index
            // state and item construction are atomic.
            let num_deduped = dedup_keys
                .iter()
                .filter(|k| pool.dedup_index.contains_key(k))
                .count();
            let per_spend_cost = if num_spends > 0 {
                cost / num_spends as u64
            } else {
                0
            };
            let cost_saving = per_spend_cost.saturating_mul(num_deduped as u64);
            let effective_virtual_cost = virtual_cost.saturating_sub(cost_saving);

            // ── CPF-008: Cross-bundle announcement validation (trivially passes) ──
            //
            // For CPFP items (non-empty depends_on), verify that any ancestor
            // announcements the child asserts can be satisfied by the ancestor chain.
            // Per spec section 5.9: assertions referencing non-ancestor bundles are
            // NOT rejected — they may be satisfied by other bundles in the same block.
            // Therefore this step never rejects; it is a best-effort consistency check.
            // (See CPF-008 spec for full details.)
            //
            // Full announcement validation will be implemented when block selection
            // and CPFP semantics are exercised end-to-end (CPF-008 acceptance tests).

            // Build the item inside the lock so cost_saving and CPFP fields are
            // consistent with the pool state at insertion time.
            let item = Arc::new(MempoolItem {
                spend_bundle: bundle,
                spend_bundle_id: bundle_id,
                fee,
                cost,
                virtual_cost,
                fee_per_virtual_cost_scaled,
                package_fee,
                package_virtual_cost,
                package_fee_per_virtual_cost_scaled: package_fpc_scaled,
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
                depends_on,
                depth,
                eligible_for_dedup,
                singleton_lineage: None,
                cost_saving,
                effective_virtual_cost,
                dedup_keys,
            });

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

            let item_ref = Arc::clone(&item);
            pool.insert(item);

            // ── CPF-006: Update ancestor descendant_scores ──
            //
            // After inserting the new item, walk up its ancestor chain and update
            // each ancestor's `descendant_score` to max(current_score, child_pkg_fpc).
            // This protects valuable parents from capacity eviction.
            pool.update_descendant_scores_on_add(&bundle_id);

            // ── LCY-005: Fire on_item_added hook ──
            drop(pool);
            self.fire_hooks(|h| h.on_item_added(&item_ref));
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
    pub fn dependents_of(&self, bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        pool.dependents
            .get(bundle_id)
            .into_iter()
            .flatten()
            .filter_map(|id| pool.items.get(id).cloned())
            .collect()
    }

    /// Return all ancestors (parents, grandparents, ...) of a bundle.
    ///
    /// Walks the dependency chain transitively. Used for CPFP package
    /// analysis and cascade eviction planning.
    /// See: [CPF-002](docs/requirements/domains/cpfp/specs/CPF-002.md)
    pub fn ancestors_of(&self, bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        let mut result = Vec::new();
        let mut to_visit: Vec<Bytes32> = pool
            .dependencies
            .get(bundle_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let mut visited: HashSet<Bytes32> = HashSet::new();
        while let Some(ancestor_id) = to_visit.pop() {
            if !visited.insert(ancestor_id) {
                continue;
            }
            if let Some(item) = pool.items.get(&ancestor_id) {
                result.push(item.clone());
                to_visit.extend(item.depends_on.iter().copied());
            }
        }
        result
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
        self.conflict.read().unwrap().len()
    }

    /// Add a bundle to the conflict cache after a failed active-pool RBF.
    ///
    /// Silently drops the bundle if the count or cost limit would be exceeded,
    /// or if the bundle ID is already cached. Returns `true` if inserted.
    ///
    /// Called by the active-pool RBF path (CFR-005) and exposed publicly for
    /// testing and for callers who manage conflict state directly.
    ///
    /// See: [POL-006](docs/requirements/domains/pools/specs/POL-006.md)
    pub fn add_to_conflict_cache(&self, bundle: SpendBundle, estimated_cost: u64) -> bool {
        let bundle_id = bundle.name();
        let inserted = self.conflict.write().unwrap().insert(
            bundle,
            estimated_cost,
            self.config.max_conflict_count,
            self.config.max_conflict_cost,
        );
        if inserted {
            self.fire_hooks(|h| h.on_conflict_cached(&bundle_id));
        }
        inserted
    }

    /// Drain all conflict cache entries for re-submission.
    ///
    /// Returns the raw SpendBundles. Each bundle must be re-submitted via
    /// `submit()` with fresh coin records. Called by `on_new_block()` (LCY-001)
    /// when a block is confirmed and previously-conflicting items may now be
    /// admissible.
    ///
    /// See: [POL-006](docs/requirements/domains/pools/specs/POL-006.md)
    pub fn drain_conflict(&self) -> Vec<SpendBundle> {
        self.conflict.write().unwrap().drain()
    }

    /// Clear all mempool state for reorg recovery.
    ///
    /// Drops all items from the active pool, pending pool, conflict cache, and
    /// seen cache. After this call the mempool is in the same state as a newly
    /// constructed one. Use when a chain reorganization invalidates the current
    /// pool state.
    ///
    /// # Concurrency
    ///
    /// Acquires write locks on all four state components. Callers must not hold
    /// any mempool read or write locks when calling this method.
    ///
    /// See: [LCY-004](docs/requirements/domains/lifecycle/specs/LCY-004.md)
    pub fn clear(&self) {
        // Collect all bundle IDs to notify hooks before clearing.
        // Fire hooks after releasing locks (hook implementations must not
        // acquire mempool locks to avoid deadlocks).
        let active_ids: Vec<Bytes32>;
        let pending_ids: Vec<Bytes32>;

        // Active pool
        {
            let mut pool = self.pool.write().unwrap();
            active_ids = pool.items.keys().copied().collect();
            pool.items.clear();
            pool.coin_index.clear();
            pool.mempool_coins.clear();
            pool.dependencies.clear();
            pool.dependents.clear();
            pool.dedup_index.clear();
            pool.dedup_waiters.clear();
            pool.total_cost = 0;
            pool.total_fees = 0;
            pool.total_spends = 0;
        }
        // Pending pool
        {
            let mut pending = self.pending.write().unwrap();
            pending_ids = pending.pending.keys().copied().collect();
            pending.pending.clear();
            pending.pending_coin_index.clear();
            pending.pending_cost = 0;
        }
        // Conflict cache
        {
            let mut conflict = self.conflict.write().unwrap();
            conflict.cache.clear();
            conflict.total_cost = 0;
        }
        // Seen cache
        {
            self.seen_cache.write().unwrap().clear();
        }

        // LCY-004: Fire on_item_removed(Cleared) for all removed active + pending items.
        for id in active_ids {
            self.fire_hooks(|h| h.on_item_removed(&id, RemovalReason::Cleared));
        }
        for id in pending_ids {
            self.fire_hooks(|h| h.on_item_removed(&id, RemovalReason::Cleared));
        }
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
    pub fn get_mempool_coin_record(&self, coin_id: &Bytes32) -> Option<CoinRecord> {
        let pool = self.pool.read().unwrap();
        let &creator_id = pool.mempool_coins.get(coin_id)?;
        let creator = pool.items.get(&creator_id)?;
        // Find the specific coin among the creator's additions.
        let coin = creator.additions.iter().find(|c| c.coin_id() == *coin_id)?;
        Some(CoinRecord {
            coin: *coin,
            coinbase: false,
            confirmed_block_index: creator.height_added as u32,
            spent: false,
            spent_block_index: 0,
            timestamp: 0, // admission timestamp not tracked in MempoolItem
        })
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

    /// Remove an active item by bundle ID.
    ///
    /// Returns `true` if the item was found and removed, `false` if it was not
    /// in the active pool. This is the single-item removal primitive used by
    /// `on_new_block()` (LCY-001) when confirmed coins are removed from the pool.
    ///
    /// Updates all indexes: `coin_index`, `mempool_coins`, `dedup_index`, and
    /// `dedup_waiters`. The seen-cache is NOT modified — previously-submitted
    /// bundles remain cached as seen to prevent re-admission.
    ///
    /// See: [LCY-001](docs/requirements/domains/lifecycle/specs/LCY-001.md)
    pub fn remove(&self, bundle_id: &Bytes32) -> bool {
        self.pool.write().unwrap().remove(bundle_id).is_some()
    }

    /// Evict the lowest-value items to relieve memory pressure.
    ///
    /// Removes approximately `percent`% of the active pool's total virtual cost
    /// by evicting items in ascending `descendant_score` order. Items within the
    /// expiry protection window are skipped (they expire naturally via `on_new_block()`).
    /// Cascade-evicts CPFP dependents of each evicted item.
    ///
    /// # Parameters
    ///
    /// - `percent`: Fraction of total cost to free (0–100). Values > 100 are treated as 100.
    /// - `current_height`: Current block height (for expiry protection checks).
    ///
    /// # Behavior
    ///
    /// - `percent = 0`: no-op (nothing removed).
    /// - `percent = 100`: evict all non-expiry-protected items.
    /// - Fires `on_item_removed(CapacityEviction)` for primary evictions.
    /// - Fires `on_item_removed(CascadeEvicted { parent_id })` for cascade evictions.
    ///
    /// See: [LCY-008](docs/requirements/domains/lifecycle/specs/LCY-008.md)
    pub fn evict_lowest_percent(&self, percent: u8, current_height: u64) {
        if percent == 0 {
            return;
        }
        let percent = percent.min(100) as u64;

        let mut removal_events: Vec<(Bytes32, RemovalReason)> = Vec::new();

        {
            let mut pool = self.pool.write().unwrap();
            let target = pool.total_cost.saturating_mul(percent) / 100;
            if target == 0 {
                return;
            }

            // Sort items by descendant_score ascending (lowest value first).
            let mut sorted: Vec<(u128, Bytes32)> = pool
                .items
                .values()
                .map(|item| (item.descendant_score, item.spend_bundle_id))
                .collect();
            sorted.sort_by_key(|(score, _)| *score);

            let protection_blocks = self.config.expiry_protection_blocks;
            let mut cost_removed: u64 = 0;

            for (_, bundle_id) in sorted {
                if cost_removed >= target {
                    break;
                }

                // Item may have been cascade-evicted by a previous iteration.
                if !pool.items.contains_key(&bundle_id) {
                    continue;
                }

                // Skip expiry-protected items.
                if let Some(item) = pool.items.get(&bundle_id) {
                    let protected = item.assert_before_height.map_or(false, |abh| {
                        abh > current_height
                            && abh <= current_height.saturating_add(protection_blocks)
                    });
                    if protected {
                        continue;
                    }
                }

                // Collect parent map before eviction for CascadeEvicted hooks.
                let parent_map = collect_descendants_parent_map(&pool.dependents, &bundle_id);

                // Pre-compute cost of this item + all its descendants before eviction.
                let mut subtree_cost: u64 = 0;
                {
                    // Collect all IDs in subtree (root + descendants).
                    let mut stack = vec![bundle_id];
                    let mut visited = std::collections::HashSet::new();
                    while let Some(id) = stack.pop() {
                        if !visited.insert(id) {
                            continue;
                        }
                        if let Some(item) = pool.items.get(&id) {
                            subtree_cost = subtree_cost.saturating_add(item.virtual_cost);
                        }
                        if let Some(children) = pool.dependents.get(&id) {
                            stack.extend(children.iter().copied());
                        }
                    }
                }

                let evicted = pool.cascade_evict(&bundle_id);
                if let Some((root, dependents)) = evicted.split_last() {
                    removal_events.push((*root, RemovalReason::CapacityEviction));
                    for dep_id in dependents {
                        let parent_id =
                            parent_map.get(dep_id).copied().unwrap_or(*root);
                        removal_events
                            .push((*dep_id, RemovalReason::CascadeEvicted { parent_id }));
                    }
                }

                cost_removed = cost_removed.saturating_add(subtree_cost);
            }
        }

        // Fire hooks after releasing the write lock.
        for (id, reason) in removal_events {
            self.fire_hooks(|h| h.on_item_removed(&id, reason.clone()));
        }
    }

    /// Number of entries in the identical-spend dedup index.
    ///
    /// Each entry represents a unique (coin_id, sha256(solution)) pair that has
    /// at least one cost-bearing active bundle. Used for testing and diagnostics.
    ///
    /// See: [POL-008](docs/requirements/domains/pools/specs/POL-008.md)
    pub fn dedup_index_len(&self) -> usize {
        self.pool.read().unwrap().dedup_index.len()
    }

    /// Look up the cost-bearer bundle ID for a (coin_id, solution_hash) dedup key.
    ///
    /// Returns `None` if the key is not in the dedup index (no eligible bundle
    /// for this spend has been admitted, or dedup is disabled).
    ///
    /// See: [POL-008](docs/requirements/domains/pools/specs/POL-008.md)
    pub fn get_dedup_bearer(&self, coin_id: &Bytes32, solution_hash: &Bytes32) -> Option<Bytes32> {
        self.pool
            .read()
            .unwrap()
            .dedup_index
            .get(&(*coin_id, *solution_hash))
            .copied()
    }

    // ── Fee Estimation ────────────────────────────────────────────────────

    /// Estimate the minimum fee required for a transaction to be admitted
    /// under current mempool conditions.
    ///
    /// Implements a 3-tier utilization system:
    ///
    /// | Utilization | Minimum Fee |
    /// |-------------|-------------|
    /// | < 80%       | 0           |
    /// | 80-100%     | `virtual_cost * full_mempool_min_fpc_scaled / FPC_SCALE` |
    /// | >= 100%     | `virtual_cost * (lowest_fpc + 1) / FPC_SCALE` |
    ///
    /// `virtual_cost = cost + (num_spends * config.spend_penalty_cost)`
    ///
    /// See: [FEE-001](docs/requirements/domains/fee_estimation/specs/FEE-001.md)
    pub fn estimate_min_fee(&self, cost: u64, num_spends: usize) -> u64 {
        let virtual_cost = MempoolItem::compute_virtual_cost(cost, num_spends);
        if virtual_cost == 0 {
            return 0;
        }

        let pool = self.pool.read().unwrap();
        let total_cost = pool.total_cost;
        let max_cost = self.config.max_total_cost;

        if max_cost == 0 || pool.items.is_empty() {
            return 0;
        }

        // Tier 1: < 80% utilization — no minimum fee required.
        // Avoid floating point: total_cost / max_cost < 0.80
        //   ↔ total_cost * 100 < max_cost * 80
        //   ↔ total_cost * 10 < max_cost * 8
        // Use u128 arithmetic to prevent overflow.
        if (total_cost as u128) * 10 < (max_cost as u128) * 8 {
            return 0;
        }

        // Tier 3: >= 100% utilization — must beat the lowest-FPC item.
        if total_cost >= max_cost {
            let lowest_fpc = pool
                .items
                .values()
                .map(|i| i.fee_per_virtual_cost_scaled)
                .min()
                .unwrap_or(0);
            let fee = (virtual_cost as u128).saturating_mul(lowest_fpc.saturating_add(1))
                / FPC_SCALE;
            return fee.min(u64::MAX as u128) as u64;
        }

        // Tier 2: 80-100% utilization — apply minimum FPC threshold.
        let fee = (virtual_cost as u128)
            .saturating_mul(self.config.full_mempool_min_fpc_scaled)
            / FPC_SCALE;
        fee.min(u64::MAX as u128) as u64
    }

    /// Record a confirmed block's transaction data into the fee estimator.
    ///
    /// Feeds confirmed bundle metrics to the internal `FeeTracker`:
    /// 1. Applies 0.998 exponential decay to all existing bucket counters.
    /// 2. Places each bundle into its fee-rate bucket (total_observed++).
    /// 3. Appends `BlockFeeData` to the rolling window.
    ///
    /// Called automatically by `on_new_block()` (step 5). May also be called
    /// directly for historical data seeding on startup.
    ///
    /// # Arguments
    ///
    /// - `height`: confirmed block height.
    /// - `bundles`: slice of per-bundle metrics from the confirmed block.
    ///
    /// See: [FEE-004](docs/requirements/domains/fee_estimation/specs/FEE-004.md)
    pub fn record_confirmed_block(&self, height: u64, bundles: &[ConfirmedBundleInfo]) {
        self.fee_tracker.write().unwrap().record_block(height, bundles);
    }

    /// Estimate the fee rate required for confirmation within `target_blocks`.
    ///
    /// Scans fee-rate buckets from highest to lowest and returns the first
    /// bucket whose success rate ≥ 85% for the given confirmation target.
    ///
    /// Returns `None` when:
    /// - Fewer than `fee_estimator_window / 2` blocks have been recorded.
    /// - No bucket meets the 85% confidence threshold (e.g., all empty).
    ///
    /// # Arguments
    ///
    /// - `target_blocks`: desired number of blocks within which to confirm.
    ///   `0` is treated as `1`. Values > 10 use the `confirmed_in_10` counter.
    ///
    /// See: [FEE-003](docs/requirements/domains/fee_estimation/specs/FEE-003.md)
    pub fn estimate_fee_rate(&self, target_blocks: u32) -> Option<FeeRate> {
        let tracker = self.fee_tracker.read().unwrap();
        tracker
            .estimate_fee_rate(target_blocks)
            .map(FeeRate::new)
    }

    /// Return a snapshot of the fee tracker's internal state.
    ///
    /// Exposes bucket counts, window size, history length, and per-bucket
    /// counters for external inspection and testing. Read-only; does not
    /// mutate the tracker.
    ///
    /// See: [FEE-002](docs/requirements/domains/fee_estimation/specs/FEE-002.md)
    pub fn fee_tracker_stats(&self) -> FeeTrackerStats {
        let tracker = self.fee_tracker.read().unwrap();
        FeeTrackerStats {
            bucket_count: tracker.bucket_count(),
            window: tracker.window,
            history_len: tracker.block_history.len(),
            bucket_ranges: tracker.bucket_ranges(),
            bucket_totals: tracker.bucket_totals(),
            bucket_confirmed_in_1: tracker.bucket_confirmed_in_1(),
        }
    }

    /// Extract the current fee estimator state for persistence.
    ///
    /// Returns a serializable `FeeEstimatorState` that captures the complete
    /// `FeeTracker` state: all bucket statistics and block history.
    /// Use `restore_fee_state()` to reload this state after a restart.
    ///
    /// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
    pub fn snapshot_fee_state(&self) -> FeeEstimatorState {
        self.fee_tracker.read().unwrap().to_state()
    }

    /// Restore the fee estimator state from a persisted snapshot.
    ///
    /// Replaces the current tracker contents with the provided state.
    /// After this call, `estimate_fee_rate()` produces the same results
    /// as the tracker that created the snapshot.
    ///
    /// See: [FEE-005](docs/requirements/domains/fee_estimation/specs/FEE-005.md)
    pub fn restore_fee_state(&self, state: FeeEstimatorState) {
        let window = self.config.fee_estimator_window;
        let new_tracker = FeeTracker::from_state(state, window);
        *self.fee_tracker.write().unwrap() = new_tracker;
    }

    // ── Block Candidate Selection ────────────────────────────────────────

    /// Select an ordered set of active items for block inclusion.
    ///
    /// Returns items in topological order (parents before children) with
    /// fee-density descending within each layer.  Only items from the active
    /// pool are considered; pending items are never returned.
    ///
    /// # Selection Algorithm
    ///
    /// 1. **SEL-002** Pre-filter: remove expired / future-timelocked items.
    /// 2. **SEL-003..006** Run four greedy strategies (density, whale, compact, age).
    /// 3. **SEL-007** Best-set comparator: highest fees → lowest cost → fewest bundles.
    /// 4. **SEL-008** Topological ordering: layer 0 first, FPC-desc within layer.
    ///
    /// # Arguments
    ///
    /// - `max_block_cost`: virtual-cost budget for the block.
    /// - `height`: current block height (for timelock evaluation).
    /// - `timestamp`: current block timestamp (for timelock evaluation).
    ///
    /// See: [SEL-001](docs/requirements/domains/selection/specs/SEL-001.md)
    pub fn select_for_block(
        &self,
        max_block_cost: u64,
        height: u64,
        timestamp: u64,
    ) -> Vec<Arc<MempoolItem>> {
        let pool = self.pool.read().unwrap();
        let max_spends = self.config.max_spends_per_block;

        // SEL-002: Pre-filter expired / future-timelocked items.
        let candidates: Vec<Arc<MempoolItem>> = pool
            .items
            .values()
            .filter(|item| sel_002_is_selectable(item, height, timestamp))
            .cloned()
            .collect();

        if candidates.is_empty() {
            return Vec::new();
        }

        let candidates_set: HashSet<Bytes32> =
            candidates.iter().map(|i| i.spend_bundle_id).collect();

        // Run all four strategies.
        let s1 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Density,
        );
        let s2 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Whale,
        );
        let s3 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Compact,
        );
        let s4 = sel_greedy(
            &candidates,
            &pool,
            &candidates_set,
            max_block_cost,
            max_spends,
            SortStrategy::Age,
        );

        // SEL-007: pick the best set.
        let best = sel_007_best([&s1, &s2, &s3, &s4]);

        // SEL-008: topological ordering.
        let result = sel_008_topological_order(best, &pool.dependencies);
        drop(pool);

        // LCY-005: Fire on_block_selected hook.
        self.fire_hooks(|h| h.on_block_selected(&result));

        result
    }

    /// Process a newly confirmed block: remove confirmed and expired items,
    /// collect pending promotions, and drain eligible conflict-cache retries.
    ///
    /// # Arguments
    ///
    /// - `height`: the new confirmed block height.
    /// - `timestamp`: the new confirmed block timestamp.
    /// - `spent_coin_ids`: coin IDs spent (confirmed) in this block.
    /// - `confirmed_bundles`: per-bundle metrics for the confirmed transactions
    ///   (forwarded to the fee estimator — currently a no-op until FEE-004).
    ///
    /// # Processing Order
    ///
    /// 1. Remove confirmed items (spending `spent_coin_ids`) + cascade-evict their dependents.
    /// 2. Remove expired items (`assert_before_height <= height` or `assert_before_seconds <= timestamp`) + cascade.
    /// 3. Collect pending promotions (`assert_height <= height`).
    /// 4. Collect conflict retries (bundles whose conflicting active items are gone).
    /// 5. Update fee estimator via `record_confirmed_block()` (FEE-004).
    ///
    /// See: [LCY-001](docs/requirements/domains/lifecycle/specs/LCY-001.md)
    pub fn on_new_block(
        &self,
        height: u64,
        timestamp: u64,
        spent_coin_ids: &[Bytes32],
        confirmed_bundles: &[ConfirmedBundleInfo],
    ) -> RetryBundles {
        let mut cascade_evicted: Vec<Bytes32> = Vec::new();

        // Accumulate (bundle_id, reason) pairs for hooks — fired after all locks released.
        let mut removal_events: Vec<(Bytes32, RemovalReason)> = Vec::new();

        // Step 0: Clear the seen cache so that promoted/retry bundles can be resubmitted.
        //
        // The seen cache prevents re-validation of bundles seen in the same block cycle.
        // On a new block boundary, all previously seen hashes are stale — bundles must
        // be re-evaluated against the new chain state. This matches Chia's behaviour in
        // `MempoolManager.new_peak()`, which clears `seen_bundle_hashes` at each peak.
        self.seen_cache.write().unwrap().clear();

        // Steps 1 + 2: Remove confirmed + expired items under a single write lock.
        {
            let mut pool = self.pool.write().unwrap();

            // Step 1: Confirmed items — bundles spending any of the confirmed coins.
            let confirmed_ids: Vec<Bytes32> = {
                let mut seen = HashSet::new();
                spent_coin_ids
                    .iter()
                    .filter_map(|coin_id| pool.coin_index.get(coin_id).copied())
                    .filter(|id| seen.insert(*id))
                    .collect()
            };

            for bundle_id in confirmed_ids {
                // Collect child→parent map BEFORE eviction (dependents map is cleared).
                let parent_map =
                    collect_descendants_parent_map(&pool.dependents, &bundle_id);

                // cascade_evict removes the root AND all dependents (children first).
                // The last element is the root (confirmed item); everything before it
                // is a cascade-evicted dependent.
                let evicted = pool.cascade_evict(&bundle_id);
                if let Some((root, dependents)) = evicted.split_last() {
                    removal_events.push((*root, RemovalReason::Confirmed));
                    for dep_id in dependents {
                        let parent_id =
                            parent_map.get(dep_id).copied().unwrap_or(*root);
                        removal_events
                            .push((*dep_id, RemovalReason::CascadeEvicted { parent_id }));
                    }
                    cascade_evicted.extend_from_slice(dependents);
                }
            }

            // Step 2: Expired items — past assert_before_height or assert_before_seconds.
            let expired_ids: Vec<Bytes32> = pool
                .items
                .values()
                .filter(|item| {
                    let h_expired = item.assert_before_height.map_or(false, |h| h <= height);
                    let s_expired = item.assert_before_seconds.map_or(false, |s| s <= timestamp);
                    h_expired || s_expired
                })
                .map(|item| item.spend_bundle_id)
                .collect();

            for bundle_id in expired_ids {
                let parent_map =
                    collect_descendants_parent_map(&pool.dependents, &bundle_id);

                let evicted = pool.cascade_evict(&bundle_id);
                if let Some((root, dependents)) = evicted.split_last() {
                    removal_events.push((*root, RemovalReason::Expired));
                    for dep_id in dependents {
                        let parent_id =
                            parent_map.get(dep_id).copied().unwrap_or(*root);
                        removal_events
                            .push((*dep_id, RemovalReason::CascadeEvicted { parent_id }));
                    }
                    cascade_evicted.extend_from_slice(dependents);
                }
            }
        }

        // Step 3: Pending promotions — timelocked items whose height is now satisfied.
        let pending_promotions = {
            let mut pending = self.pending.write().unwrap();
            pending.drain(height, timestamp)
        };

        // Step 4: Conflict retries — bundles whose conflicting active items are gone.
        //
        // A conflict-cache bundle is retryable when none of the coins it spends
        // are still claimed by an active pool item. If any conflicting coin is still
        // active, the bundle would fail RBF again immediately.
        let conflict_retries = {
            let pool = self.pool.read().unwrap();
            let mut conflict = self.conflict.write().unwrap();

            let retryable: Vec<Bytes32> = conflict
                .cache
                .iter()
                .filter(|(_, (bundle, _))| {
                    !bundle
                        .coin_spends
                        .iter()
                        .any(|cs| pool.coin_index.contains_key(&cs.coin.coin_id()))
                })
                .map(|(id, _)| *id)
                .collect();

            let mut bundles = Vec::with_capacity(retryable.len());
            for id in retryable {
                if let Some((bundle, cost)) = conflict.cache.remove(&id) {
                    conflict.total_cost = conflict.total_cost.saturating_sub(cost);
                    bundles.push(bundle);
                }
            }
            bundles
        };

        // Step 5: Update fee estimator with confirmed block data (FEE-004).
        self.record_confirmed_block(height, confirmed_bundles);

        // LCY-005: Fire removal hooks after all locks are released.
        for (id, reason) in removal_events {
            self.fire_hooks(|h| h.on_item_removed(&id, reason.clone()));
        }

        RetryBundles {
            conflict_retries,
            pending_promotions,
            cascade_evicted,
        }
    }

    // ── Snapshot / Restore (LCY-007) ─────────────────────────────────────

    /// Capture a serializable snapshot of the complete mempool state.
    ///
    /// Includes all active items, pending items, conflict-cache bundles,
    /// and fee estimator state. The seen-cache is intentionally excluded:
    /// after `restore()`, bundles that were previously seen can be resubmitted
    /// without being rejected as `AlreadySeen`.
    ///
    /// # Thread safety
    ///
    /// Acquires read locks on all four state components.
    ///
    /// See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)
    pub fn snapshot(&self) -> MempoolSnapshot {
        let active_items: Vec<MempoolItem> = self
            .pool
            .read()
            .unwrap()
            .items
            .values()
            .map(|arc| (**arc).clone())
            .collect();

        let pending_items: Vec<MempoolItem> = self
            .pending
            .read()
            .unwrap()
            .pending
            .values()
            .map(|arc| (**arc).clone())
            .collect();

        let conflict_bundles: Vec<(SpendBundle, u64)> = self
            .conflict
            .read()
            .unwrap()
            .cache
            .values()
            .map(|(bundle, cost)| (bundle.clone(), *cost))
            .collect();

        let fee_estimator_state = self.fee_tracker.read().unwrap().to_state();

        MempoolSnapshot {
            active_items,
            pending_items,
            conflict_bundles,
            fee_estimator_state,
        }
    }

    /// Restore mempool state from a snapshot.
    ///
    /// Clears the active pool, pending pool, and conflict cache (the seen-cache
    /// is intentionally left unchanged), then rebuilds all indexes from the
    /// snapshot data. The fee estimator is fully replaced.
    ///
    /// After this call:
    /// - `len()` / `stats()` / `active_items()` reflect the snapshot state.
    /// - `pending_len()` reflects the snapshot state.
    /// - `conflict_len()` reflects the snapshot state.
    /// - `estimate_fee_rate()` returns results from the restored fee tracker.
    ///
    /// # Invariants
    ///
    /// All derived indexes (`coin_index`, `mempool_coins`, `dedup_index`, etc.)
    /// are rebuilt from the stored items via `ActivePool::insert()`.
    ///
    /// # Thread safety
    ///
    /// Acquires write locks on all four state components sequentially.
    ///
    /// See: [LCY-007](docs/requirements/domains/lifecycle/specs/LCY-007.md)
    pub fn restore(&self, snap: MempoolSnapshot) {
        // ── Clear active pool ──
        {
            let mut pool = self.pool.write().unwrap();
            pool.items.clear();
            pool.coin_index.clear();
            pool.mempool_coins.clear();
            pool.dependencies.clear();
            pool.dependents.clear();
            pool.dedup_index.clear();
            pool.dedup_waiters.clear();
            pool.total_cost = 0;
            pool.total_fees = 0;
            pool.total_spends = 0;
        }

        // ── Clear pending pool ──
        {
            let mut pending = self.pending.write().unwrap();
            pending.pending.clear();
            pending.pending_coin_index.clear();
            pending.pending_cost = 0;
        }

        // ── Clear conflict cache ──
        {
            let mut conflict = self.conflict.write().unwrap();
            conflict.cache.clear();
            conflict.total_cost = 0;
        }

        // ── Restore active items — insert() rebuilds all indexes ──
        {
            let mut pool = self.pool.write().unwrap();
            for item in snap.active_items {
                pool.insert(Arc::new(item));
            }
        }

        // ── Restore pending items ──
        {
            let mut pending = self.pending.write().unwrap();
            for item in snap.pending_items {
                pending.insert(Arc::new(item));
            }
        }

        // ── Restore conflict cache ──
        {
            let mut conflict = self.conflict.write().unwrap();
            for (bundle, cost) in snap.conflict_bundles {
                let id = bundle.name();
                conflict.total_cost = conflict.total_cost.saturating_add(cost);
                conflict.cache.insert(id, (bundle, cost));
            }
        }

        // ── Restore fee estimator ──
        let window = self.config.fee_estimator_window;
        let new_tracker = FeeTracker::from_state(snap.fee_estimator_state, window);
        *self.fee_tracker.write().unwrap() = new_tracker;
    }
}
