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
            active_count: RwLock::new(0),
            bls_cache: Mutex::new(BlsCache::default()),
            seen_cache: RwLock::new(SeenCache::new(seen_cache_size)),
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
        let _spend_result =
            dig_clvm::validate_spend_bundle(&bundle, &ctx, &config, Some(&mut bls_cache))?;

        // Release the BLS cache before Phase 2
        drop(bls_cache);

        // TODO(ADM-003): Dedup check via seen-cache
        // TODO(ADM-004): Fee extraction + RESERVE_FEE check from _spend_result
        // TODO(ADM-005): Virtual cost computation from _spend_result
        // TODO(ADM-006): Timelock resolution from _spend_result.conditions
        // TODO(ADM-007): Dedup/FF flag extraction from _spend_result.conditions
        // TODO(CFR-001): Conflict detection against coin_index
        // TODO(POL-002): Capacity management / eviction
        //
        // ── Phase 2: State mutation (write lock) ──
        // For now, after successful validation, we return Success.
        // The actual insertion into pools will be implemented in POL-001.

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
