# dig-mempool Specification

**Version:** 0.1.0
**Status:** Draft
**Date:** 2026-04-12

## 1. Overview

`dig-mempool` is a self-contained Rust crate that implements a fee-prioritized, conflict-aware transaction mempool for the DIG Network L2 blockchain. It accepts raw `SpendBundle` submissions, **validates them internally via `dig-clvm`** (CLVM dry-run + BLS aggregate signature verification to ensure the bundle is valid and correctly signed), then manages their lifecycle (ordering, conflicts, timelocks, eviction, CPFP dependencies) and outputs selected `MempoolItem`s for block candidate production.

The mempool **does** perform:
- **CLVM validation** of submitted bundles (via `dig_clvm::validate_spend_bundle()`) to ensure puzzle execution succeeds, conditions are valid, signatures verify, and conservation holds. Invalid bundles are rejected before entering the pool.

The mempool does **not** perform:
- **Block building** (generator compression, signature aggregation, singleton lineage rebasing)
- **Block validation** (executing block generators, verifying block-level conditions)
- Any consensus logic beyond mempool admission

The design is derived from Chia's production mempool ([`chia/full_node/mempool.py`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py), [`chia/full_node/mempool_manager.py`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py)) with targeted improvements for L2 throughput and functionality.

**Hard boundary:** Inputs = raw `SpendBundle` + `CoinRecord`s. Outputs = `Vec<Arc<MempoolItem>>`. Block production and block validation are outside this crate.

The mempool operates on the **coinset model** (UTXO-like), where coins are created and destroyed atomically. It is designed for Chia-compatible coin semantics: a coin's identity is `sha256(parent_coin_id || puzzle_hash || amount)`, coins are spent by revealing the puzzle program and providing a solution, and conditions (outputs, signatures, timelocks) are produced by CLVM execution.

### 1.1 Design Principles

- **Chia parity**: Every Chia-valid SpendBundle that does not rely on L1-specific features (PoS, timelord) should be admissible. Condition opcodes, signature domains, cost accounting, and fee semantics match Chia's rules.
- **Validation on admission**: The mempool runs a CLVM dry-run and BLS signature verification on every submitted `SpendBundle` via `dig_clvm::validate_spend_bundle()`. This ensures only valid, correctly-signed bundles enter the pool. The resulting `SpendResult` (additions, removals, fee, parsed conditions) is used for all subsequent state management decisions.
- **No I/O**: The mempool is a pure in-memory data structure. Coin lookups, persistence, and networking are the caller's responsibility. The caller provides coin records at submission time. Persistence is supported through `snapshot()` / `restore()` serialization, not internal I/O.
- **Extensibility**: Domain-specific admission policies, custom block selection strategies, and event hooks are supported through traits, keeping the core mempool generic.
- **Determinism**: All ordering, selection, and eviction decisions are deterministic given the same inputs. No randomness, no wall-clock time dependencies (timestamps are caller-provided).
- **Minimal lock contention**: CLVM validation (the expensive part) runs before acquiring the mempool write lock. The write lock is only held for fast HashMap operations during insertion. Multiple submissions can validate concurrently.

### 1.2 Crate Dependencies

`dig-clvm` is the **single integration point** for all Chia ecosystem crates. The mempool depends on `dig-clvm` and `dig-constants` directly; all Chia types are accessed via `dig-clvm` re-exports (see `dig-clvm/src/lib.rs`).

| Crate | Purpose |
|-------|---------|
| `dig-clvm` | **Runtime dependency**. The mempool calls `dig_clvm::validate_spend_bundle()` on every submission to validate CLVM execution and BLS signatures. Also the source of all re-exported Chia types: `SpendBundle`, `Coin`, `Bytes32`, `CoinRecord`, `OwnedSpendBundleConditions`, `SpendResult`, `ValidationContext`, `ValidationConfig`, `BlsCache`, `Cost`, etc. (dig-clvm also provides `build_block_generator()` and `validate_block()` but these are outside the mempool's scope -- used by the block producer/validator.) |
| `dig-constants` | Network constants (`NetworkConstants`, `DIG_MAINNET`, `DIG_TESTNET`). Wraps `chia-consensus::ConsensusConstants` with DIG-specific genesis challenge, AGG_SIG domain data, cost limits. Also re-exported by `dig-clvm`. |
| `chia-consensus` | Used via dig-clvm. Provides `OwnedSpendBundleConditions` (the parsed CLVM output the mempool reads), flag constants (`ELIGIBLE_FOR_DEDUP`, `ELIGIBLE_FOR_FF`, `MEMPOOL_MODE`), and opcode constants (`chia_consensus::opcodes::*` -- all condition opcodes like `CREATE_COIN`, `RESERVE_FEE`, `AGG_SIG_ME`, etc. are defined here and should NOT be redefined). The mempool reads these after dig-clvm's validation; `fast_forward_singleton()` and `check_time_locks()` are used by the caller outside the mempool. |
| `chia-sdk-types` | Condition enum (`Condition<T>`) for type-safe condition handling. `announcement_id()` for cross-bundle announcement validation. Singleton/CAT/NFT puzzle type definitions. |
| `chia-sdk-driver` | **Types only** -- `SingletonLayer` types for singleton identification. The caller uses `SingletonLayer::parse_puzzle()` before submission to extract launcher_id/lineage info and includes it in the submission. The mempool does not parse puzzles. |
| `chia-sdk-coinset` | `CoinRecord` for coin state. `MempoolItem` (basic SpendBundle + fee). `MempoolMinFees`, `BlockchainState` for fee tracking types. |
| `chia-protocol` | Core types: `SpendBundle` (+ `::name()`, `::additions()`), `Coin` (+ `::coin_id()`), `CoinSpend`, `Program`, `Bytes32`, `Bytes`, `CoinState`. Fee types: `FeeRate`, `FeeEstimate`, `FeeEstimateGroup`. |
| `chia-bls` | `BlsCache` (maintained internally for cached signature verification during `submit()`), `Signature`, `PublicKey` types. BLS verification is performed by dig-clvm's `validate_spend_bundle()` which the mempool calls with a `BlsCache` for pairing reuse. |
| `chia-puzzles` | Compiled puzzle bytecode and hashes: `SINGLETON_TOP_LAYER_V1_1_HASH`, `SINGLETON_LAUNCHER_HASH`, `CAT_PUZZLE_HASH`, etc. |
| `chia-traits` | `Streamable` trait for canonical serialization (used in snapshot/restore). |
| `thiserror` | Error type derivation. |
| `serde` | Serialization for snapshot/restore persistence. |

**Key Chia types used by the mempool (all via dig-clvm re-exports):**

| Type | From Crate | Mempool Usage |
|------|-----------|---------------|
| `SpendBundle` | chia-protocol | Transaction input. `::name()` computes bundle ID. `::coin_spends` for iteration. |
| `Coin` | chia-protocol | UTXO identity. `::coin_id()` computes `sha256(parent \|\| puzzle_hash \|\| amount)`. |
| `CoinRecord` | chia-sdk-coinset | Coin state: `coin`, `confirmed_block_index`, `spent`, `timestamp`, `coinbase`. |
| `OwnedSpendBundleConditions` | chia-consensus | Full CLVM output: `cost`, `reserve_fee`, `removal_amount`, `addition_amount`, timelocks, per-spend conditions with `flags` (ELIGIBLE_FOR_DEDUP, ELIGIBLE_FOR_FF). |
| `BlsCache` | chia-bls | BLS pairing cache for `aggregate_verify()`. |
| `Bytes32` | chia-protocol | 32-byte hash for coin IDs, bundle IDs, puzzle hashes. Implements `Hash + Eq`. |
| `FeeRate` | chia-protocol | `mojos_per_clvm_cost: u64`. Used in fee estimation output. |
| `FeeEstimate` | chia-protocol | `error`, `time_target`, `estimated_fee_rate`. Used in fee estimation output. |
| `FeeEstimateGroup` | chia-protocol | Groups multiple `FeeEstimate`s for different time targets. |
| `Cost` (= `u64`) | clvmr | CLVM cost tracking. |
| `Condition<T>` | chia-sdk-types | Type-safe enum for all 40+ condition opcodes. Used for condition inspection. |
| `announcement_id()` | chia-sdk-types | Computes `sha256(coin_info \|\| message)` for cross-bundle announcement validation. |
| `SingletonLayer<I>` | chia-sdk-driver | Parses singleton puzzles: extracts `launcher_id`, validates structure. |
| `fast_forward_singleton()` | chia-consensus | Validates and rebases singleton lineage proofs for FF spends. |
| `supports_fast_forward()` | chia-consensus | Checks if a `CoinSpend` puzzle structure is FF-eligible. |
| `check_time_locks()` | chia-consensus | Validates timelock conditions against chain height/timestamp. |
| `MempoolMinFees` | chia-sdk-coinset | Fee threshold tracking by cost tier. |
| `Streamable` | chia-traits | Canonical serialization trait. Used for snapshot/restore. |
| `SINGLETON_TOP_LAYER_V1_1_HASH` | chia-puzzles | Singleton puzzle hash constant for detection. |
| `SINGLETON_LAUNCHER_HASH` | chia-puzzles | Singleton launcher puzzle hash constant. |

### 1.3 Design Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | Interior mutability (`&self` + internal `RwLock`) | Fine-grained locking: reads concurrent with writes. All ops are fast (no CLVM). |
| 2 | Pending items do NOT participate in active conflict detection | Matches Chia L1. |
| 3 | Seen-cache populated before validation | Matches Chia L1 DoS protection. |
| 4 | Admission cost = CLVM runner output only (no byte cost) | Matches Chia L1. |
| 5 | Four block selection strategies (density, whale, compact, age-weighted) + custom | Age-weighted prevents starvation; custom trait enables domain-specific priority. |
| 6 | Separate pending pool limits (count + cost) | Matches Chia L1. |
| 7 | Conflict retry cache included | Matches Chia L1. |
| 8 | `on_new_block()` returns bundles to retry; caller resubmits | Maintains "no I/O" principle. |
| 9 | Caller workflow: `on_new_block()` -> `submit()` retries -> `select_for_block()` | Matches Chia L1 sequencing. |
| 10 | `Arc<MempoolItem>` in public API | Immutable items; cheap to share. |
| 11 | `std` only | Server-side mempool. |
| 12 | Zero-fee allowed when utilization < 80% | Matches Chia L1. |
| 13 | `select_for_block` returns topologically sorted then FPC descending | Parents before children; highest density first within layers. |
| 14 | `MempoolError` derives `Clone + PartialEq` | Testability. `ValidationError` stored as `String`. |
| 15 | Bundle ID = `SpendBundle::name()` from chia-protocol | Uses Chia's built-in canonical hash. No custom serialization. |

### 1.4 Improvements Over Chia L1

| # | Improvement | Description |
|---|-------------|-------------|
| 1 | Two-phase validation | CLVM validation (Phase 1) runs lock-free, enabling concurrent `submit()`. Write lock (Phase 2) only held for fast HashMap ops. Chia holds the lock for the entire flow. |
| 2 | Virtual cost / spend penalty | `virtual_cost = cost + num_spends * SPEND_PENALTY_COST`. |
| 3 | Expiring transaction protection | Items near expiry are protected from eviction. |
| 4 | Pending pool deduplication | Pending-vs-pending RBF at submission time. |
| 5 | Batch submission | Single lock acquisition for all inserts. |
| 6 | CPFP dependency chains | Package fee rates; child-pays-for-parent; cascade eviction. |
| 7 | Identical spend deduplication | Same coin+puzzle+solution across bundles costs once (cost accounting). |
| 8 | Singleton fast-forward | Chain tracking for sequential singleton spends. |
| 9 | Bitcoin-style fee estimation | Historical fee tracker with time-target estimates. |
| 10 | Event hooks | Synchronous callbacks on mempool mutations. |
| 11 | Snapshot/restore persistence | Serializable state for durability across restarts. |
| 12 | Custom block selection strategies | Caller-provided priority via trait. |
| 13 | Cross-bundle announcement validation | CPFP chains validate announcements across dependent bundles. |

---

## 2. Data Model

### 2.1 Coin Identity

Coins follow the Chia coinset model:

```
CoinId = sha256(parent_coin_id || puzzle_hash || amount)
```

A `CoinId` is a `Bytes32` (32-byte hash). Throughout this spec, "coin ID" refers to this derived identifier.

**Bundle ID** = `SpendBundle::name()` from `chia-protocol`. This method computes the canonical hash of the bundle using Chia's Streamable serialization, ensuring all nodes produce the same ID. No custom serialization is needed.

### 2.2 MempoolItem

A `MempoolItem` represents a validated, admitted transaction in the mempool. Items are immutable once created and stored internally as `Arc<MempoolItem>`. Corresponds to Chia's `MempoolItem` ([`mempool_item.py:45-120`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L45)) and `BundleCoinSpend` ([`mempool_item.py:25-42`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L25)), flattened into a single struct with CPFP extensions.

```rust
pub struct MempoolItem {
    // -- Identity --

    /// The original spend bundle (chia-protocol::SpendBundle).
    pub spend_bundle: SpendBundle,
    /// Computed via `SpendBundle::name()` (chia-protocol). Canonical bundle hash.
    pub spend_bundle_id: Bytes32,

    // -- Cost & fee (individual) --

    /// Implicit fee: `conditions.removal_amount - conditions.addition_amount`.
    /// Already computed by chia-consensus in `OwnedSpendBundleConditions`.
    /// Also available as `SpendResult.fee` from dig-clvm (validate.rs:159).
    pub fee: u64,
    /// CLVM cost from `OwnedSpendBundleConditions.cost` (= execution_cost + condition_cost).
    /// No byte cost at this layer (matches Chia mempool_item.py:84-85).
    pub cost: u64,
    /// Virtual cost: cost + (num_spends * SPEND_PENALTY_COST).
    /// Matches Chia's `MempoolItem.virtual_cost` (mempool_item.py:92-93).
    pub virtual_cost: u64,
    /// fee * FPC_SCALE / virtual_cost. Integer-precision fee-rate.
    /// Chia uses `fee_per_virtual_cost` as a float (mempool_item.py:76-77);
    /// we use scaled integers to avoid floating-point non-determinism.
    pub fee_per_virtual_cost_scaled: u128,

    // -- Cost & fee (package / CPFP) --

    /// Sum of fees across this item and all its ancestors.
    pub package_fee: u64,
    /// Sum of virtual costs across this item and all its ancestors.
    pub package_virtual_cost: u64,
    /// package_fee * FPC_SCALE / package_virtual_cost.
    pub package_fee_per_virtual_cost_scaled: u128,

    // -- Descendant score (eviction) --

    /// Maximum of (own FPC, package FPC of any descendant chain).
    /// Used for eviction ordering: protects low-fee parents with valuable children.
    /// Updated when children are added or removed.
    pub descendant_score: u128,

    // -- State deltas --

    /// Coins created by this bundle (from CREATE_COIN conditions).
    pub additions: Vec<Coin>,
    /// Coin IDs consumed by this bundle (the spent inputs).
    pub removals: Vec<Bytes32>,

    // -- Metadata --

    /// L2 block height when this item was admitted.
    pub height_added: u64,
    /// The full validated conditions from CLVM execution.
    /// This is `chia_consensus::owned_conditions::OwnedSpendBundleConditions`,
    /// returned by dig-clvm as `SpendResult.conditions`. Contains:
    /// - `.cost`, `.execution_cost`, `.condition_cost` — cost breakdown
    /// - `.removal_amount`, `.addition_amount` — balance totals (u128)
    /// - `.reserve_fee` — sum of all RESERVE_FEE conditions (already computed)
    /// - `.height_absolute`, `.seconds_absolute` — bundle-level absolute timelocks
    /// - `.before_height_absolute`, `.before_seconds_absolute` — bundle-level expiry
    /// - `.validated_signature` — whether BLS sig was already verified
    /// - `.spends[]` — per-spend conditions including:
    ///   - `.flags` — `ELIGIBLE_FOR_DEDUP` (0x1), `ELIGIBLE_FOR_FF` (0x4),
    ///     set by chia-consensus MempoolVisitor during execution
    ///   - `.create_coin` — Vec<(puzzle_hash, amount, hint)>
    ///   - `.height_relative`, `.seconds_relative`, etc. — per-spend timelocks
    ///   - `.agg_sig_me`, `.agg_sig_parent`, etc. — per-spend signatures
    pub conditions: OwnedSpendBundleConditions,
    /// Number of coin spends in this bundle.
    pub num_spends: usize,

    // -- Timelocks --

    /// Resolved absolute assert-height (earliest valid height). None if no timelocks.
    pub assert_height: Option<u64>,
    /// Resolved absolute assert-before-height (latest valid height). None if no expiry.
    pub assert_before_height: Option<u64>,
    /// Resolved absolute assert-before-seconds. None if no time expiry.
    pub assert_before_seconds: Option<u64>,

    // -- Dependencies (CPFP) --

    /// Bundle IDs this item directly depends on (spends coins created by them).
    pub depends_on: HashSet<Bytes32>,
    /// Depth in the dependency chain. 0 = no dependencies.
    pub depth: u32,

    // -- Deduplication --

    /// Whether this item's coin spends are eligible for identical-spend deduplication.
    /// Read from `OwnedSpendConditions.flags & ELIGIBLE_FOR_DEDUP` -- this flag is
    /// set by chia-consensus's MempoolVisitor during CLVM execution when MEMPOOL_MODE
    /// is active. The canonical encoding check is performed by chia-consensus, not
    /// by the mempool. True if ALL spends in the bundle have the flag set.
    pub eligible_for_dedup: bool,

    // -- Singleton fast-forward --

    /// If this item spends a singleton, the lineage info for fast-forward rebasing.
    /// Detected when `OwnedSpendConditions.flags & ELIGIBLE_FOR_FF` is set by
    /// chia-consensus. The `supports_fast_forward()` function from chia-consensus
    /// confirms FF eligibility from the puzzle structure.
    pub singleton_lineage: Option<SingletonLineageInfo>,
}
```

### 2.3 SingletonLineageInfo

Tracks the state of a singleton coin for fast-forward optimization. Extends Chia's `UnspentLineageInfo` ([`mempool_item.py:18-22`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L18)) with `launcher_id` and `inner_puzzle_hash` for puzzle-level singleton identification.

```rust
pub struct SingletonLineageInfo {
    /// The current unspent singleton coin ID.
    pub coin_id: Bytes32,
    /// The parent of the current singleton.
    pub parent_id: Bytes32,
    /// The grandparent (parent's parent).
    pub parent_parent_id: Bytes32,
    /// The singleton's launcher ID (immutable identity).
    pub launcher_id: Bytes32,
    /// The singleton's inner puzzle hash.
    pub inner_puzzle_hash: Bytes32,
}
```

### 2.4 MempoolConfig

All tuneable parameters with sensible defaults. Builder pattern with `with_*` methods. Chia's equivalent is the `MempoolInfo` struct passed to the `Mempool` constructor ([`mempool.py:107`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L107)).

```rust
pub struct MempoolConfig {
    // -- Active pool limits --

    /// Maximum aggregate cost across all active items.
    /// Default: L2_MAX_COST_PER_BLOCK * MEMPOOL_BLOCK_BUFFER (15x block cost).
    /// Chia: `MempoolInfo.max_size_in_cost` checked in mempool.py:509-514.
    pub max_total_cost: u64,
    /// Maximum cost for a single spend bundle.
    /// Default: L1_MAX_COST_PER_SPEND (11,000,000,000) from dig-clvm config.rs:6.
    /// Chia: `max_tx_clvm_cost` checked in mempool_manager.py:733.
    pub max_bundle_cost: u64,
    /// Maximum number of coin spends per block.
    /// Default: 6,000 (matches Chia L1 MAX_SPENDS_PER_BLOCK).
    pub max_spends_per_block: usize,

    // -- Pending pool limits --

    /// Maximum timelocked items. Default: 3,000.
    /// Chia: `PendingTxCache._cache_max_size = 3000` (pending_tx_cache.py:53).
    pub max_pending_count: usize,
    /// Maximum aggregate cost of timelocked items. Default: 1x block cost.
    /// Chia: `PendingTxCache._cache_max_total_cost` (pending_tx_cache.py:52).
    pub max_pending_cost: u64,

    // -- Conflict cache limits --

    /// Maximum conflict cache items. Default: 1,000.
    /// Chia: `ConflictTxCache._cache_max_size = 1000` (pending_tx_cache.py:15).
    pub max_conflict_count: usize,
    /// Maximum aggregate cost of conflict cache items. Default: 1x block cost.
    /// Chia: `ConflictTxCache._cache_max_total_cost` (pending_tx_cache.py:14).
    pub max_conflict_cost: u64,

    // -- Fee / RBF --

    /// Minimum absolute fee increase for RBF. Default: 10,000,000 (10M mojos).
    /// Chia: `MEMPOOL_MIN_FEE_INCREASE = 10000000` (mempool_manager.py:52).
    pub min_rbf_fee_bump: u64,
    /// Minimum FPC (scaled) when mempool is 80-100% full. Default: 5 * FPC_SCALE.
    pub full_mempool_min_fpc_scaled: u128,

    // -- Virtual cost --

    /// Cost penalty per coin spend. Default: 500,000.
    /// Chia: `SPEND_PENALTY_COST = 500_000` (mempool_item.py:14).
    pub spend_penalty_cost: u64,

    // -- Expiry protection --

    /// Blocks before expiry within which items are eviction-protected. Default: 100.
    pub expiry_protection_blocks: u64,

    // -- CPFP --

    /// Maximum dependency chain depth. Default: 25.
    pub max_dependency_depth: u32,

    // -- Deduplication --

    /// Seen-cache capacity. Default: 10,000.
    pub max_seen_cache_size: usize,

    // -- Fee estimation --

    /// Number of recent blocks tracked by the fee estimator. Default: 100.
    pub fee_estimator_window: usize,
    /// Number of fee-rate buckets for the fee estimator. Default: 50.
    pub fee_estimator_buckets: usize,

    // -- Singleton fast-forward --

    /// Enable singleton fast-forward optimization. Default: true.
    pub enable_singleton_ff: bool,

    // -- Identical spend dedup --

    /// Enable identical spend deduplication. Default: true.
    pub enable_identical_spend_dedup: bool,
}
```

### 2.5 Constants

```rust
/// Integer scaling for fee-per-cost. Avoids Chia's float-based fee_per_cost (mempool_item.py:72-73).
pub const FPC_SCALE: u128 = 1_000_000_000_000;
/// Chia's `ConsensusConstants.mempool_block_buffer` is 10 (available via
/// `dig_constants::NetworkConstants::consensus().mempool_block_buffer`).
/// DIG L2 overrides to 15 to account for higher block cost (550B vs 11B).
pub const MEMPOOL_BLOCK_BUFFER: u64 = 15;
/// Chia: `MEMPOOL_MIN_FEE_INCREASE = 10000000` (mempool_manager.py:52).
pub const MIN_RBF_FEE_BUMP: u64 = 10_000_000;
/// Chia: `nonzero_fee_minimum_fpc = 5` (mempool_manager.py:746).
pub const FULL_MEMPOOL_MIN_FPC_SCALED: u128 = 5 * FPC_SCALE;
pub const DEFAULT_SEEN_CACHE_SIZE: usize = 10_000;
/// Chia: PendingTxCache._cache_max_size (pending_tx_cache.py:53).
pub const DEFAULT_MAX_PENDING_COUNT: usize = 3_000;
/// Chia: ConflictTxCache._cache_max_size (pending_tx_cache.py:15).
pub const DEFAULT_MAX_CONFLICT_COUNT: usize = 1_000;
/// Chia: SPEND_PENALTY_COST (mempool_item.py:14).
pub const SPEND_PENALTY_COST: u64 = 500_000;
/// Chia uses 48 blocks / 900 seconds (~15 min) (mempool.py:408-409).
/// DIG L2 uses 100 blocks (configurable).
pub const DEFAULT_EXPIRY_PROTECTION_BLOCKS: u64 = 100;
pub const DEFAULT_MAX_DEPENDENCY_DEPTH: u32 = 25;
pub const DEFAULT_MAX_SPENDS_PER_BLOCK: usize = 6_000;
pub const DEFAULT_FEE_ESTIMATOR_WINDOW: usize = 100;
pub const DEFAULT_FEE_ESTIMATOR_BUCKETS: usize = 50;
```

---

## 3. Public API

### 3.1 Construction

```rust
impl Mempool {
    /// Create a mempool with default configuration for the given network.
    pub fn new(constants: NetworkConstants) -> Self;

    /// Create a mempool with custom configuration.
    pub fn with_config(constants: NetworkConstants, config: MempoolConfig) -> Self;
}
```

### 3.2 Transaction Submission

The mempool validates every submitted bundle internally via `dig_clvm::validate_spend_bundle()` (CLVM dry-run + BLS aggregate signature verification). Invalid or incorrectly-signed bundles are rejected.

```rust
pub enum SubmitResult {
    /// Bundle was admitted to the active mempool.
    Success,
    /// Bundle is valid but timelocked; stored in the pending pool.
    Pending { assert_height: u64 },
}

impl Mempool {
    /// Validate and submit a spend bundle to the mempool.
    ///
    /// Internally calls `dig_clvm::validate_spend_bundle()` to:
    /// - Execute CLVM puzzles with solutions (dry-run)
    /// - Verify the BLS aggregate signature (using internal BlsCache)
    /// - Parse conditions, compute cost, check conservation
    ///
    /// If validation passes, the mempool then performs:
    /// 1. Deduplication (seen-cache)
    /// 2. Timelock resolution
    /// 3. Conflict detection + RBF
    /// 4. CPFP dependency resolution
    /// 5. Capacity management / eviction
    /// 6. Optional admission policy
    ///
    /// The caller supplies `coin_records` for every on-chain coin spent by
    /// the bundle. For CPFP, use `get_mempool_coin_record()` for mempool coins.
    pub fn submit(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
    ) -> Result<SubmitResult, MempoolError>;

    /// Submit with a custom admission policy.
    pub fn submit_with_policy(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
        policy: &dyn AdmissionPolicy,
    ) -> Result<SubmitResult, MempoolError>;

    /// Validate and submit multiple bundles in batch.
    ///
    /// CLVM validation runs concurrently across bundles (parallel validation).
    /// State mutation acquires the write lock once for all insertions.
    /// Earlier entries can create coins that later entries depend on (CPFP).
    ///
    /// Returns one result per input, in the same order.
    pub fn submit_batch(
        &self,
        bundles: Vec<SpendBundle>,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
    ) -> Vec<Result<SubmitResult, MempoolError>>;

    /// Batch submit with a custom admission policy.
    pub fn submit_batch_with_policy(
        &self,
        bundles: Vec<SpendBundle>,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
        policy: &dyn AdmissionPolicy,
    ) -> Vec<Result<SubmitResult, MempoolError>>;
}
```

### 3.3 CPFP Coin Queries

```rust
impl Mempool {
    /// Look up a coin created by an active mempool item and return a synthetic
    /// CoinRecord for use in a subsequent submit() call.
    ///
    /// The synthetic record uses:
    /// - `confirmed_block_index` = parent item's `height_added`
    /// - `timestamp` = the timestamp when the parent was admitted
    /// - `spent` = false
    /// - `coinbase` = false
    ///
    /// Returns None if the coin was not created by any active mempool item.
    /// Note: TOCTOU safe -- if the parent is evicted between this call and
    /// submit(), Phase 2 will reject the bundle with CoinNotFound.
    pub fn get_mempool_coin_record(&self, coin_id: &Bytes32) -> Option<CoinRecord>;

    /// Look up which active mempool item created a given coin.
    /// Returns the creating bundle's ID, or None.
    pub fn get_mempool_coin_creator(&self, coin_id: &Bytes32) -> Option<Bytes32>;
}
```

### 3.4 Block Candidate Selection

The mempool selects which items to propose for block inclusion. It does **not** build the block itself (no generator compression, signature aggregation, or lineage rebasing). The caller takes the returned items and passes them to the block producer.

```rust
impl Mempool {
    /// Select an optimal set of non-conflicting mempool items for block inclusion.
    ///
    /// Returns items in **topological order** (parents before children), with
    /// items at the same depth sorted by fee-per-virtual-cost descending.
    ///
    /// Total cost will not exceed `max_block_cost`. Total spend count will
    /// not exceed `config.max_spends_per_block`. Uses CPFP-aware multi-strategy
    /// greedy selection.
    ///
    /// The caller is responsible for taking these items and building the actual
    /// block (e.g., via `dig_clvm::build_block_generator()`, singleton FF
    /// rebasing via `fast_forward_singleton()`, etc.).
    ///
    /// **Caller workflow**: `on_new_block()` -> resubmit retries -> `select_for_block()`.
    pub fn select_for_block(
        &self,
        max_block_cost: u64,
        height: u64,
        timestamp: u64,
    ) -> Vec<Arc<MempoolItem>>;

    /// Same as `select_for_block` but returns `BundleSelection` with metadata.
    pub fn select_for_block_with_costs(
        &self,
        max_block_cost: u64,
        height: u64,
        timestamp: u64,
    ) -> Vec<BundleSelection>;

    /// Select using a custom strategy instead of the built-in 4-way greedy.
    /// The custom strategy receives all eligible items and returns the selected set.
    pub fn select_for_block_with_strategy(
        &self,
        max_block_cost: u64,
        height: u64,
        timestamp: u64,
        strategy: &dyn BlockSelectionStrategy,
    ) -> Vec<Arc<MempoolItem>>;
}

pub struct BundleSelection {
    pub item: Arc<MempoolItem>,
    pub cost: u64,
    pub fee: u64,
}
```

### 3.5 Block Confirmation

```rust
pub struct RetryBundles {
    /// Bundles from the conflict cache whose conflicting item was removed.
    pub conflict_retries: Vec<SpendBundle>,
    /// Bundles from the pending pool whose timelocks are now satisfied.
    pub pending_promotions: Vec<SpendBundle>,
    /// Bundle IDs that were cascade-evicted because their parent was
    /// confirmed or expired. These cannot be retried (their input coins
    /// no longer exist in the mempool). Provided for caller bookkeeping.
    pub cascade_evicted: Vec<Bytes32>,
}

impl Mempool {
    /// Notify the mempool that a new block has been confirmed.
    ///
    /// # Effects
    ///
    /// 1. Removes active items whose inputs overlap with `spent_coin_ids`,
    ///    plus all their dependents (cascade eviction).
    /// 2. Removes expired active items plus dependents.
    /// 3. Extracts promotable pending items.
    /// 4. Extracts retryable conflict cache items.
    /// 5. Updates the fee estimator with confirmed transaction data.
    /// 6. Fires event hooks.
    /// 7. Updates internal height/timestamp tracking.
    ///
    /// The `confirmed_bundles` parameter provides the fee/cost data of transactions
    /// confirmed in this block, used to update the fee estimation tracker.
    pub fn on_new_block(
        &self,
        height: u64,
        timestamp: u64,
        spent_coin_ids: &[Bytes32],
        confirmed_bundles: &[ConfirmedBundleInfo],
    ) -> RetryBundles;

    /// Clear the entire mempool. Used for reorg recovery.
    pub fn clear(&self);
}

/// Information about a confirmed transaction, fed to the fee estimator.
pub struct ConfirmedBundleInfo {
    pub cost: u64,
    pub fee: u64,
    pub num_spends: usize,
}
```

### 3.6 Fee Estimation

```rust
impl Mempool {
    /// Estimate the minimum fee for admission given current utilization.
    ///
    /// | Utilization | Minimum fee |
    /// |-------------|-------------|
    /// | < 80%       | 0           |
    /// | 80% - 100%  | `virtual_cost * full_mempool_min_fpc_scaled / FPC_SCALE` |
    /// | >= 100%     | `virtual_cost * (lowest_fpc + 1) / FPC_SCALE` |
    pub fn estimate_min_fee(&self, cost: u64, num_spends: usize) -> u64;

    /// Estimate the fee-per-cost needed for confirmation within `target_blocks`.
    ///
    /// Uses the Bitcoin-style fee tracker based on recent confirmed block data.
    /// Returns the estimated fee rate as a scaled FPC value, or None if
    /// insufficient data (fewer than `fee_estimator_window / 2` blocks tracked).
    /// Returns a `chia_protocol::FeeRate` (mojos_per_clvm_cost) or None.
    pub fn estimate_fee_rate(&self, target_blocks: u32) -> Option<FeeRate>;

    /// Feed confirmed block data into the fee estimator.
    /// Called internally by `on_new_block()`, but also available for manual seeding
    /// (e.g., loading historical data on startup).
    pub fn record_confirmed_block(&self, height: u64, bundles: &[ConfirmedBundleInfo]);
}
```

### 3.7 Queries

```rust
impl Mempool {
    pub fn get(&self, bundle_id: &Bytes32) -> Option<Arc<MempoolItem>>;
    pub fn contains(&self, bundle_id: &Bytes32) -> bool;
    pub fn active_bundle_ids(&self) -> Vec<Bytes32>;
    pub fn pending_bundle_ids(&self) -> Vec<Bytes32>;
    pub fn active_items(&self) -> Vec<Arc<MempoolItem>>;
    pub fn dependents_of(&self, bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>>;
    pub fn ancestors_of(&self, bundle_id: &Bytes32) -> Vec<Arc<MempoolItem>>;
    pub fn len(&self) -> usize;
    pub fn pending_len(&self) -> usize;
    pub fn conflict_len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn stats(&self) -> MempoolStats;
}

pub struct MempoolStats {
    pub active_count: usize,
    pub pending_count: usize,
    pub conflict_count: usize,
    pub total_cost: u64,
    pub total_fees: u64,
    pub max_cost: u64,
    pub utilization: f64,
    pub min_fpc_scaled: u128,
    pub max_fpc_scaled: u128,
    pub items_with_dependencies: usize,
    pub max_current_depth: u32,
    pub total_spend_count: usize,
    pub dedup_eligible_count: usize,
    pub singleton_ff_count: usize,
}
```

### 3.8 Eviction

```rust
impl Mempool {
    /// Evict lowest fee-rate items from the active pool.
    /// Uses `descendant_score` for ordering (protects parents with valuable children).
    /// Respects expiry protection. Cascade-evicts dependents of removed parents.
    pub fn evict_lowest_percent(&self, percent: u8);

    /// Remove a specific bundle. Cascade-evicts dependents.
    /// Returns all removed bundle IDs.
    pub fn remove(&self, bundle_id: &Bytes32) -> Vec<Bytes32>;
}
```

### 3.9 Persistence

```rust
impl Mempool {
    /// Serialize the full mempool state for persistence.
    /// Includes active pool, pending pool, conflict cache, fee estimator state,
    /// and singleton lineage tracking.
    /// Does NOT include the seen-cache or BLS cache (ephemeral).
    pub fn snapshot(&self) -> MempoolSnapshot;

    /// Restore mempool state from a snapshot.
    /// Replaces all current state. The caller should re-validate items
    /// against current chain state after restore if the snapshot may be stale.
    pub fn restore(&self, snapshot: MempoolSnapshot);
}

/// Serializable mempool state. `SpendBundle`, `Coin`, and other chia-protocol types
/// implement the `Streamable` trait from `chia-traits` for canonical serialization.
/// The snapshot uses serde for the outer structure wrapping these types.
#[derive(Serialize, Deserialize)]
pub struct MempoolSnapshot {
    pub active_items: Vec<MempoolItem>,
    pub pending_items: Vec<MempoolItem>,
    pub conflict_bundles: Vec<SpendBundle>,
    pub fee_estimator_state: FeeEstimatorState,
    pub height: u64,
    pub timestamp: u64,
}
```

### 3.10 Extension Traits

```rust
/// Admission policy: domain-specific validation after standard checks pass.
pub trait AdmissionPolicy {
    fn check(
        &self,
        item: &MempoolItem,
        existing_items: &[Arc<MempoolItem>],
    ) -> Result<(), String>;
}

/// Custom block selection strategy.
pub trait BlockSelectionStrategy {
    /// Given eligible items, return the selected set.
    /// The implementation must:
    /// - Not exceed `max_block_cost` total cost.
    /// - Not exceed `max_spends` total spend count.
    /// - Not include conflicting items (shared spent coins).
    /// - Include all ancestors of any selected CPFP item.
    /// - Return items in topological order (parents before children).
    fn select(
        &self,
        eligible_items: &[Arc<MempoolItem>],
        max_block_cost: u64,
        max_spends: usize,
    ) -> Vec<Arc<MempoolItem>>;
}

/// Event hook: synchronous callbacks on mempool mutations.
/// Called under the write lock -- implementations must be fast and non-blocking.
pub trait MempoolEventHook: Send + Sync {
    fn on_item_added(&self, _item: &MempoolItem) {}
    fn on_item_removed(&self, _bundle_id: &Bytes32, _reason: RemovalReason) {}
    fn on_block_selected(&self, _items: &[Arc<MempoolItem>]) {}
    fn on_conflict_cached(&self, _bundle_id: &Bytes32) {}
    fn on_pending_added(&self, _item: &MempoolItem) {}
}

/// Reason an item was removed from the mempool.
#[derive(Debug, Clone, PartialEq)]
pub enum RemovalReason {
    /// Included in a confirmed block.
    Confirmed,
    /// Replaced by a higher-fee bundle (RBF).
    ReplacedByFee { replacement_id: Bytes32 },
    /// Parent was removed; this item's inputs no longer exist.
    CascadeEvicted { parent_id: Bytes32 },
    /// Expired (assert_before_height or assert_before_seconds passed).
    Expired,
    /// Evicted due to capacity pressure (low fee-per-cost).
    CapacityEviction,
    /// Explicitly removed by caller via remove().
    ExplicitRemoval,
    /// Mempool cleared (reorg recovery).
    Cleared,
}

impl Mempool {
    /// Register an event hook. Multiple hooks can be registered.
    /// Hooks are called synchronously under the write lock.
    pub fn add_event_hook(&self, hook: Arc<dyn MempoolEventHook>);
}
```

---

## 4. Error Types

Chia uses `chia.util.errors.Err` enum codes (e.g., `DOUBLE_SPEND = 5`, `UNKNOWN_UNSPENT = 6`, `MEMPOOL_CONFLICT = 19`, `INVALID_FEE_LOW_FEE = 18`). dig-clvm uses `ValidationError` (`dig-clvm/src/consensus/error.rs:9-36`). dig-mempool defines its own `MempoolError` that wraps both, converting dig-clvm errors to strings for `Clone + PartialEq` derive support.

```rust
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MempoolError {
    // -- Deduplication --
    #[error("bundle already seen: {0}")]
    AlreadySeen(Bytes32),

    // -- Structural --
    #[error("duplicate spend of coin {0} within bundle")]
    DuplicateSpend(Bytes32),

    #[error("coin not found: {0}")]
    CoinNotFound(Bytes32),

    #[error("coin already spent: {0}")]
    CoinAlreadySpent(Bytes32),

    // -- CLVM / signature --
    #[error("CLVM execution error: {0}")]
    ClvmError(String),

    #[error("invalid aggregate signature")]
    InvalidSignature,

    // -- Cost / fee --
    #[error("cost {cost} exceeds maximum {max}")]
    CostExceeded { cost: u64, max: u64 },

    #[error("negative fee: outputs ({output}) exceed inputs ({input})")]
    NegativeFee { input: u64, output: u64 },

    #[error("insufficient fee: required {required}, available {available}")]
    InsufficientFee { required: u64, available: u64 },

    #[error("fee too low for current mempool utilization")]
    FeeTooLow,

    // -- Conflict / RBF --
    #[error("conflicts with existing mempool item {0}")]
    Conflict(Bytes32),

    #[error("RBF rejected: must spend superset of conflicting bundle's coins")]
    RbfNotSuperset,

    #[error("RBF rejected: fee-per-cost not higher than existing")]
    RbfFpcNotHigher,

    #[error("RBF rejected: fee bump {provided} below minimum {required}")]
    RbfBumpTooLow { required: u64, provided: u64 },

    // -- Capacity --
    #[error("mempool full: cannot admit bundle")]
    MempoolFull,

    #[error("pending pool full")]
    PendingPoolFull,

    // -- Timelocks --
    #[error("impossible timelock constraints")]
    ImpossibleTimelocks,

    #[error("bundle has expired")]
    Expired,

    // -- Conservation --
    #[error("conservation violation: input {input}, output {output}")]
    ConservationViolation { input: u64, output: u64 },

    // -- Dependency chains --
    #[error("dependency depth {depth} exceeds maximum {max}")]
    DependencyTooDeep { depth: u32, max: u32 },

    #[error("dependency cycle detected")]
    DependencyCycle,

    // -- Spend count --
    #[error("spend count {count} would exceed block limit {max}")]
    TooManySpends { count: usize, max: usize },

    // -- Policy --
    #[error("admission policy rejected: {0}")]
    PolicyRejected(String),

    // -- Upstream --
    #[error("validation error: {0}")]
    ValidationError(String),
}
```

---

## 5. Admission Pipeline

The admission pipeline corresponds to Chia's `MempoolManager.add_spend_bundle()` ([`mempool_manager.py:538-607`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L538)) and `validate_spend_bundle()` ([`mempool_manager.py:609-833`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L609)).

The pipeline is split into two phases for concurrency:
- **Phase 1 (lock-free)**: CLVM validation via dig-clvm. Multiple submissions validate concurrently.
- **Phase 2 (write lock)**: State management (dedup, conflicts, insertion). Only fast HashMap operations.

### 5.1 Pipeline Steps

```
submit(bundle, coin_records, height, timestamp)
   │
   ├─ Phase 1: Validation (lock-free, concurrent)
   │   ├─ 1. Deduplication check (seen-cache, read lock only)
   │   ├─ 2. CLVM dry-run + BLS signature verification (via dig-clvm)
   │   ├─ 3. Extract SpendResult: additions, removals, fee, conditions
   │   ├─ 4. Cost and virtual cost computation
   │   └─ 5. Timelock resolution (relative -> absolute)
   │
   └─ Phase 2: State Mutation (write lock, fast)
       ├─ 6. Re-check dedup (race between concurrent Phase 1s)
       ├─ 7. Dedup / FF flag extraction (from conditions.flags)
       ├─ 8. CPFP dependency resolution (mempool_coins lookup)
       ├─ 9. Cross-bundle announcement validation (CPFP only)
       ├─ 10. Conflict detection (coin_index lookup)
       ├─ 11. Replace-by-fee evaluation
       ├─ 12. Singleton fast-forward chain handling
       ├─ 13. Capacity management / eviction
       ├─ 14. Identical spend dedup cost adjustment
       ├─ 15. Pending pool dedup (if timelocked)
       ├─ 16. Admission policy (optional)
       ├─ 17. Descendant score update
       └─ 18. Insertion + event hooks
```

### 5.2 Deduplication (Phase 1)

Compute bundle ID via `SpendBundle::name()` (chia-protocol). Check against seen-cache, active items, pending items, and conflict cache. If found, return `AlreadySeen`. Otherwise add to seen-cache **immediately** (DoS protection -- prevents repeated expensive CLVM validation of the same invalid bundle). Chia stores seen hashes in `MempoolManager.seen_bundle_hashes` ([`mempool_manager.py:298`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L298)) with the same pre-validation semantics.

### 5.3 CLVM Validation (Phase 1, lock-free)

Call `dig_clvm::validate_spend_bundle()` (`dig-clvm/src/consensus/validate.rs:30-167`) to validate the bundle. This is the expensive step and runs **without holding the mempool write lock**, enabling concurrent validation of multiple submissions.

```rust
let ctx = ValidationContext {
    height: current_height as u32,
    timestamp: current_timestamp,
    constants: self.constants.clone(),
    coin_records: coin_records.clone(),
    ephemeral_coins: mempool_coin_candidate_ids,  // CPFP coins
};
let config = ValidationConfig {
    flags: MEMPOOL_MODE,  // strict validation (from chia_consensus::flags)
    ..Default::default()
};
let spend_result = dig_clvm::validate_spend_bundle(
    &bundle, &ctx, &config, Some(&mut self.bls_cache)
)?;
```

dig-clvm internally:
1. Checks for duplicate spends and coin existence (validate.rs:39-62)
2. Executes CLVM puzzles with solutions via `chia_consensus::run_spendbundle()` (validate.rs:79-102)
3. Verifies BLS aggregate signature via `BlsCache::aggregate_verify()` (validate.rs:107-113)
4. Enforces cost limits (validate.rs:126-131)
5. Extracts additions from `CREATE_COIN` conditions and removals (validate.rs:134-146)
6. Checks conservation: `sum(inputs) >= sum(outputs)` (validate.rs:148-159)

On failure, `dig_clvm::ValidationError` is converted to `MempoolError::ValidationError(e.to_string())`.

### 5.4 Extract Fields from SpendResult (Phase 1)

Read values from the validated `SpendResult`:
- `fee = spend_result.fee` (= `conditions.removal_amount - conditions.addition_amount`, already computed by chia-consensus).
- `cost = spend_result.conditions.cost` (= `execution_cost + condition_cost`). Condition costs use chia-consensus opcode constants (`chia_consensus::opcodes::AGG_SIG_COST = 1,200,000`, `CREATE_COIN_COST = 1,800,000`, etc. from [`condition_costs.py:6-15`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/consensus/condition_costs.py#L6)). These constants are NOT redefined by the mempool.
- `additions = spend_result.additions` (coins created by `CREATE_COIN` conditions).
- `removals` = coin IDs from `spend_result.removals` (via `Coin::coin_id()`).
- `reserve_fee = spend_result.conditions.reserve_fee` (pre-summed by chia-consensus).
- If `fee < reserve_fee` -> `InsufficientFee`. Chia: [`mempool_manager.py:728`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L728).

### 5.5 Cost and Virtual Cost (Phase 1)

```
virtual_cost = cost + (num_spends * spend_penalty_cost)  // Chia: mempool_item.py:92-93
fee_per_virtual_cost_scaled = (fee * FPC_SCALE) / virtual_cost
```
If `cost > config.max_bundle_cost` -> `CostExceeded`. Chia: [`mempool_manager.py:733-734`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L733).

### 5.6 Timelock Resolution (Phase 1)

Resolve relative timelocks to absolute values for storage, mirroring Chia's `compute_assert_height()` ([`mempool_manager.py:81-126`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L81)). The mempool reads per-spend timelock conditions from `spend_result.conditions.spends[*]`:

- `height_relative`: resolved via `coin_record.confirmed_block_index + n` ([`mempool_manager.py:103`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L103)).
- `seconds_relative`: resolved via `coin_record.timestamp + n` ([`mempool_manager.py:107`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L107)).
- `before_height_relative` / `before_seconds_relative`: resolved and min'd ([`mempool_manager.py:110-124`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L110)).
- Bundle-level absolutes: `conditions.height_absolute`, `conditions.before_height_absolute`, etc.

For CPFP mempool coin candidates (not in coin_records), use `confirmed_block_index = current_height` and `timestamp = current_timestamp`, matching Chia's ephemeral coin handling ([`mempool_manager.py:716-722`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L716)).

- Impossible constraints (`assert_before_height <= assert_height`) -> `ImpossibleTimelocks`. Chia: [`mempool_manager.py:791-796`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L791).
- Already expired -> `Expired`.
- Future timelocked -> marked as pending. Chia routes to pending cache ([`mempool_manager.py:600-603`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L600)).

### 5.7 Dedup and Fast-Forward Flag Extraction (Phase 2)

Both flags are **pre-computed by chia-consensus during the caller's CLVM execution** (via `MempoolVisitor` when `MEMPOOL_MODE` is set). The mempool reads them from `OwnedSpendConditions.flags` in the `SpendResult.conditions`; it does **not** run CLVM, check canonical encoding, or parse puzzle structures.

**Identical Spend Dedup** (if `config.enable_identical_spend_dedup`):
- Read `ELIGIBLE_FOR_DEDUP` (0x1) from `conditions.spends[*].flags`. Chia reads this at [`mempool_manager.py:662-663`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L662).
- Set `eligible_for_dedup = true` if ALL spends in the bundle have the flag.

**Singleton Fast-Forward** (if `config.enable_singleton_ff`):
- Read `ELIGIBLE_FOR_FF` (0x4) from `conditions.spends[*].flags`.
- The caller is responsible for having already called `supports_fast_forward()` and `SingletonLayer::parse_puzzle()` before submission, and for providing `SingletonLineageInfo` in the submission if applicable. The mempool does not parse puzzles.

### 5.8 CPFP Dependency Resolution (Phase 2)

For each coin in `removals` that is NOT in the caller's `coin_records`:
1. Look up in `mempool_coins` -> if found, record dependency on the creating bundle.
2. If not found -> `CoinNotFound`.

Compute depth. If > `max_dependency_depth` -> `DependencyTooDeep`. Defensive cycle check -> `DependencyCycle`.

Compute package fee rates:
```
package_fee = fee + sum(ancestor.fee)
package_virtual_cost = virtual_cost + sum(ancestor.virtual_cost)
package_fpc_scaled = (package_fee * FPC_SCALE) / package_virtual_cost
```

### 5.9 Cross-Bundle Announcement Validation (Phase 2, CPFP)

For CPFP items (non-empty `depends_on`), validate cross-bundle announcements using `chia_sdk_types::announcement_id()`:
- `ASSERT_COIN_ANNOUNCEMENT(hash)`: check if any ancestor's conditions include a matching `CREATE_COIN_ANNOUNCEMENT`.
- `ASSERT_PUZZLE_ANNOUNCEMENT(hash)`: check against ancestors' `CREATE_PUZZLE_ANNOUNCEMENT`.
- `RECEIVE_MESSAGE(msg)`: check against ancestors' `SEND_MESSAGE`.

Intra-bundle announcements were already validated by dig-clvm during the caller's validation. Only cross-bundle (ancestor-to-descendant) assertions are checked here.

**Note**: Assertions referencing non-ancestor bundles are not rejected -- they may be satisfied by other bundles in the same block during block validation (outside mempool scope).

### 5.10 Conflict Detection (Phase 2)

Active pool only. Check `coin_index` for each removal. Collect conflicting bundle IDs. Mirrors Chia's `check_removals()` ([`mempool_manager.py:229-292`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L229)) which queries the `spends` table ([`mempool.py:290-299`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L290)).

### 5.11 Replace-By-Fee (Phase 2)

Three conditions, matching Chia's `can_replace()` ([`mempool_manager.py:1077-1126`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1077)):

1. **Superset rule** ([`mempool_manager.py:1101-1109`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1101)): Every coin in conflicting items must be in new item.
2. **Higher FPC** ([`mempool_manager.py:1119-1126`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1119)): `new_item.fee_per_cost > conflicting_fees / conflicting_cost`.
3. **Minimum fee bump**: `new_item.fee >= conflicting_fees + MIN_RBF_FEE_BUMP`. Chia: `MEMPOOL_MIN_FEE_INCREASE` ([`mempool_manager.py:52`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L52)).

**RBF + CPFP interaction** (dig-mempool extension, not in Chia): When RBF replaces a parent, all its dependents are cascade-evicted. Cascade-evicted children do NOT go to the conflict cache.

On RBF failure, add bundle to conflict cache (matching Chia's [`mempool_manager.py:595-599`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L595)) and return error.

### 5.12 Singleton Fast-Forward Chain Handling (Phase 2)

If `config.enable_singleton_ff` and `singleton_lineage` is provided:
1. Check `singleton_spends[launcher_id]` for existing items.
2. **Sequential update**: append to chain if spending the latest version's output.
3. **Conflicting update**: apply RBF rules if spending an older version.
4. **Fresh singleton**: add normally.

### 5.13 Capacity Management (Phase 2)

Corresponds to Chia's `add_to_pool()` ([`mempool.py:395-502`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L395)).

**Active pool**:
- Sort by `descendant_score` ascending (improvement over Chia's raw `fee_per_cost` at [`mempool.py:448-458`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L448)).
- Skip expiry-protected items. Chia: 48 blocks/900 seconds ([`mempool.py:406-442`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L406)).
- Cascade-evict dependents of removed parents (CPFP extension).

**Pending pool**:
- Check count and cost limits -> `PendingPoolFull`. Chia: [`pending_tx_cache.py:77-89`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/pending_tx_cache.py#L77).

### 5.14 Identical Spend Dedup Cost Adjustment (Phase 2)

If `eligible_for_dedup`: check `dedup_index` for matching `(coin_id, solution_hash)`. If found, reduce this item's effective cost. The mempool tracks cost savings for selection decisions; actual dedup execution is outside scope.

### 5.15 Pending Pool Deduplication (Phase 2)

If timelocked, check `pending_coin_index` for conflicts. Apply RBF rules for pending-vs-pending.

### 5.16 Admission Policy (Phase 2)

If a policy was provided, invoke `check()`.

### 5.17 Descendant Score Update and Insertion (Phase 2)

After inserting:
- Walk ancestor chain, update `descendant_score` to max of current and new item's `package_fpc_scaled`.
- Add to `items`, `coin_index`, `mempool_coins`, dependency graph.
- Fire event hooks.

#### 5.4.2 CPFP Dependency Resolution
For each mempool coin candidate:
1. Look up in `mempool_coins` -> if found, record dependency.
2. If not found -> `CoinNotFound`.

Compute depth. If > `max_dependency_depth` -> `DependencyTooDeep`.
Defensive cycle check -> `DependencyCycle`.

Compute package fee rates:
```
package_fee = fee + sum(ancestor.fee)
package_virtual_cost = virtual_cost + sum(ancestor.virtual_cost)
package_fpc_scaled = (package_fee * FPC_SCALE) / package_virtual_cost
```

#### 5.4.3 Cross-Bundle Announcement Validation (CPFP)

For CPFP items (non-empty `depends_on`), validate cross-bundle announcements:
- `ASSERT_COIN_ANNOUNCEMENT(hash)`: check if any ancestor bundle's conditions include a matching `CREATE_COIN_ANNOUNCEMENT`.
- `ASSERT_PUZZLE_ANNOUNCEMENT(hash)`: check against ancestors' `CREATE_PUZZLE_ANNOUNCEMENT`.
- `RECEIVE_MESSAGE(msg)`: check against ancestors' `SEND_MESSAGE`.

Announcements that are satisfied intra-bundle (within the same SpendBundle) are already validated by dig-clvm. This step validates cross-bundle assertions that rely on CPFP parent bundles.

**Note**: Cross-bundle announcements that reference non-ancestor bundles are not validated (they must be satisfied within the same block during block validation, not mempool admission). Only ancestor-to-descendant assertions are checked here as a correctness guarantee for the dependency chain.

#### 5.4.4 Conflict Detection
Active pool only. Check `coin_index` for each removal. Collect conflicting bundle IDs. This mirrors Chia's `check_removals()` ([`mempool_manager.py:229-292`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L229)) which queries the `spends` table ([`mempool.py:290-299`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L290)). Chia also handles FF and dedup special cases for conflicts ([`mempool_manager.py:270-288`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L270)).

#### 5.4.5 Replace-By-Fee (RBF)

Three conditions, matching Chia's `can_replace()` ([`mempool_manager.py:1077-1126`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1077)):

1. **Superset rule** ([`mempool_manager.py:1101-1109`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1101)): Every coin in conflicting items must be in new item.
2. **Higher FPC** ([`mempool_manager.py:1119-1126`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1119)): `new_item.fee_per_cost > conflicting_fees / conflicting_cost`.
3. **Minimum fee bump**: `new_item.fee >= conflicting_fees + MIN_RBF_FEE_BUMP`. Chia: `MEMPOOL_MIN_FEE_INCREASE` ([`mempool_manager.py:52`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L52)).

**RBF + CPFP interaction** (dig-mempool extension, not in Chia): When RBF replaces a parent, all its dependents are cascade-evicted. The superset rule checks only the conflicting item's own removals, not its dependents' removals. Cascade-evicted children are irrecoverably invalid (their input coins no longer exist) and do NOT go to the conflict cache.

On RBF failure, add bundle to conflict cache (if space permits, matching Chia's [`mempool_manager.py:595-599`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L595)) and return error.

#### 5.4.6 Singleton Fast-Forward

If `config.enable_singleton_ff` and the item has `singleton_lineage`:
1. Check if another active item already spends the same singleton (same `launcher_id`).
2. If so, check if the existing item can be fast-forwarded:
   - The existing item must also have `singleton_lineage`.
   - The new item must spend the coin created by the existing item (sequential singleton update).
3. If fast-forward is possible: update the existing item's lineage to point to the new item's output. Both items remain in the mempool. During block selection, the items are ordered correctly (old spend before new spend) and the block builder rebases the lineage proof.
4. If the same singleton is spent with incompatible solutions (not sequential), treat as a conflict and apply RBF rules.

#### 5.4.7 Capacity Management

Corresponds to Chia's `add_to_pool()` ([`mempool.py:395-502`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L395)).

**Active pool**:
- Sort by `descendant_score` ascending (improvement over Chia's raw `fee_per_cost` sort at [`mempool.py:448-458`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L448)).
- Skip expiry-protected items. Chia implements this as a separate pass for items expiring within 48 blocks/900 seconds ([`mempool.py:406-442`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L406)). dig-mempool uses a configurable `expiry_protection_blocks` window.
- When admitting an expiring item, prefer evicting other expiring items with lower FPC. Matches Chia's expiring-item-specific eviction logic at [`mempool.py:416-442`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L416).
- Cascade-evict dependents of removed parents (dig-mempool CPFP extension, not in Chia).
- Check `max_spends_per_block`: if total active spends + new spends > limit, reject with `TooManySpends`.

**Pending pool**:
- Check count and cost limits -> `PendingPoolFull`. Chia: `PendingTxCache` evicts highest-assert_height items first ([`pending_tx_cache.py:77-89`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/pending_tx_cache.py#L77)).

#### 5.4.8 Identical Spend Deduplication

If `config.enable_identical_spend_dedup` and `eligible_for_dedup`:
- For each `CoinSpend` in the bundle, compute `(coin_id, puzzle_hash, solution_hash)`.
- Check if any active item has an identical spend (same coin_id, same puzzle, same solution).
- If found, the new item records a **cost saving**: the CLVM execution cost for the duplicated spend is not counted toward the new item's admission cost. Instead, the shared cost is attributed once (to the existing item).
- This allows a bundle that re-spends the same coin with the same solution to enter the mempool at lower effective cost, encouraging deduplication.

**Note**: The actual deduplication (running CLVM once per unique spend) is a block production optimization, outside the mempool's scope. The mempool's role is limited to tracking dedup eligibility and adjusting cost accounting so that block candidate selection makes better decisions.

#### 5.4.9 Pending Pool Deduplication

If timelocked, check `pending_coin_index` for conflicts with other pending items. Apply RBF rules for pending-vs-pending.

#### 5.4.10 Admission Policy
If a policy was provided, invoke `check()`.

#### 5.4.11 Descendant Score Update

After inserting the new item:
- If it has parents, walk up the ancestor chain and update each ancestor's `descendant_score` to the max of their current score and the new item's `package_fee_per_virtual_cost_scaled`.
- This ensures parents with newly valuable children are protected from eviction.

#### 5.4.12 Insertion

**Active pool**: Add to `items`, `coin_index`, `mempool_coins`, dependency graph, accumulators.
**Pending pool**: Add to `pending`, `pending_coin_index`, pending cost.

Fire `on_item_added` / `on_pending_added` event hooks.

### 5.5 Batch Submission

`submit_batch()` uses a two-pass approach for intra-batch CPFP:

1. **Dependency scan**: For each bundle, identify which coins it spends. Partition into:
   - **Independent**: all coins are in caller's `coin_records`.
   - **Dependent**: one or more coins are not in `coin_records` (potential intra-batch CPFP).

2. **Parallel Phase 1**: Validate all independent bundles concurrently. For dependent bundles, identify their parent within the batch (the bundle whose additions include the needed coin). Validate dependent bundles sequentially after their parent's Phase 1 completes, synthesizing CoinRecords from the parent's additions.

3. **Single Phase 2**: Acquire write lock once. Insert all bundles in batch order. Earlier insertions create `mempool_coins` entries that later insertions can reference.

---

## 6. Block Candidate Selection Algorithm

Chia selects bundles via `create_bundle_from_mempool_items()` ([`mempool.py:583-615`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L583)) using a single `ORDER BY priority DESC, seq ASC` query ([`mempool.py:605`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L605)). dig-mempool uses a multi-strategy greedy approach with CPFP awareness for better optimization. The caller then passes selected bundles to `dig_clvm::build_block_generator()` (`dig-clvm/src/consensus/block.rs:29-115`), which calls `chia_consensus::solution_generator_backrefs()` for CLVM back-reference compression (matching Chia's [`mempool.py:540`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L540)).

### 6.1 Pre-Selection Filtering

Exclude:
- Items with `assert_height > height`.
- Items with `assert_before_height <= height`.
- Items with `assert_before_seconds <= timestamp`.

### 6.2 CPFP-Aware Multi-Strategy Greedy Selection

Four built-in strategies, all using `package_fee_per_virtual_cost_scaled` for items with dependencies:

**Strategy 1: Fee-per-cost (density)** -- Sort by package FPC desc, fee desc, virtual_cost asc, height_added asc, bundle_id asc.

**Strategy 2: Absolute fee (whale)** -- Sort by package_fee desc, package FPC desc, virtual_cost asc, height_added asc, bundle_id asc.

**Strategy 3: Compact high-value** -- Sort by package FPC desc, virtual_cost asc, fee desc, height_added asc, bundle_id asc.

**Strategy 4: Age-weighted** -- Sort by height_added asc, package FPC desc, fee desc, bundle_id asc.

### 6.3 CPFP-Aware Greedy Accumulation

For each sorted item:
1. Compute unselected ancestors.
2. Total cost = item + unselected ancestors.
3. Total spends = item.num_spends + sum(ancestor.num_spends for unselected).
4. Skip if total cost > remaining block cost.
5. Skip if total spends would exceed `max_spends_per_block`.
6. Skip if any conflict with already-selected items.
7. Otherwise include item + all unselected ancestors.

### 6.4 Singleton Chain Ordering

After greedy selection, for any singleton chain (multiple items spending sequential versions of the same singleton):
- Order them by lineage (oldest first).
- Verify the chain is unbroken.
- The mempool returns them in this order. Lineage rebasing is the caller's responsibility (outside mempool scope).

### 6.5 Identical Spend Cost Adjustment

During selection, for items marked `eligible_for_dedup`:
- Identify duplicate spends across candidate items.
- Reduce the effective cost budget consumed by duplicated CLVM cost (each unique spend is counted once).
- This may free budget for additional items. Re-run the greedy scan on remaining items with the freed cost.
- The actual dedup execution is the block producer's responsibility; the mempool only accounts for the cost savings during selection.

### 6.6 Best Selection

Compare four candidate sets by: highest total fees, then lowest total virtual cost, then fewest bundles.

### 6.7 Custom Strategy

`select_for_block_with_strategy()` bypasses the 4-way selection and delegates to the caller's `BlockSelectionStrategy` implementation. The implementation must respect cost limits, spend count limits, conflict-freeness, and topological ordering.

### 6.8 Final Ordering

Output is sorted:
1. Topological order (parents before children).
2. Within each topological layer: fee-per-virtual-cost descending.
3. Tiebreaker: `height_added` ascending, then `bundle_id` ascending.

### 6.9 Determinism

All sort orders include `height_added` and `bundle_id` as final tiebreakers.

---

## 7. Condition Handling

All conditions are parsed by `dig-clvm` (which delegates to `chia-consensus`); the mempool reads the parsed results from `OwnedSpendBundleConditions`. Condition opcodes are defined in Chia's [`condition_opcodes.py`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/condition_opcodes.py). Condition costs are applied during CLVM execution per [`condition_costs.py:6-15`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/consensus/condition_costs.py#L6): `AGG_SIG = 1,200,000`, `CREATE_COIN = 1,800,000`, `MESSAGE_CONDITION_COST = 700`, `GENERIC_CONDITION_COST = 200`.

### 7.1 Coin Conditions

| Opcode | Name | Mempool Behavior |
|--------|------|-----------------|
| 51 | `CREATE_COIN` | Extract additions. Register in `mempool_coins` for CPFP. |
| 52 | `RESERVE_FEE` | Verify implicit fee >= sum of all RESERVE_FEE values. |

### 7.2 Signature Conditions

Verified by dig-clvm with MEMPOOL_MODE. Domain separation per dig-constants.

| Opcode | Name | Domain Separation |
|--------|------|-------------------|
| 43-48 | `AGG_SIG_PARENT` through `AGG_SIG_PARENT_PUZZLE` | `genesis_challenge \|\| 0x2b` through `0x30` |
| 49 | `AGG_SIG_UNSAFE` | none |
| 50 | `AGG_SIG_ME` | `genesis_challenge` |

### 7.3 Timelock Conditions

| Opcode | Name | Mempool Behavior |
|--------|------|-----------------|
| 80-83 | `ASSERT_SECONDS/HEIGHT_RELATIVE/ABSOLUTE` | Resolve; may cause pending status. |
| 84-87 | `ASSERT_BEFORE_SECONDS/HEIGHT_RELATIVE/ABSOLUTE` | Resolve; sets expiry + protection. |

CPFP coins use `confirmed_block_index = current_height`, `timestamp = current_timestamp` for relative timelock resolution.

### 7.4 Announcement / Message Conditions

| Opcode | Name | Mempool Behavior |
|--------|------|-----------------|
| 60-63 | `CREATE/ASSERT_COIN/PUZZLE_ANNOUNCEMENT` | Intra-bundle: dig-clvm. Cross-bundle (CPFP): Phase 2. |
| 64-65 | `ASSERT_CONCURRENT_SPEND/PUZZLE` | Intra-bundle: dig-clvm. |
| 66-67 | `SEND/RECEIVE_MESSAGE` | Intra-bundle: dig-clvm. Cross-bundle (CPFP): Phase 2. |

### 7.5 Self-Assertion Conditions

| Opcode | Name | Mempool Behavior |
|--------|------|-----------------|
| 70-76 | `ASSERT_MY_*`, `ASSERT_EPHEMERAL` | Validated by dig-clvm. |

### 7.6 Other Conditions

| Opcode | Name | Mempool Behavior |
|--------|------|-----------------|
| 1 | `REMARK` | Ignored. Inspectable by `AdmissionPolicy`. |
| 90 | `SOFTFORK` | Passed to dig-clvm. |

---

## 8. Internal Data Structures

Chia uses SQLite in-memory tables ([`mempool.py:108`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L108)) for the `tx` table ([`mempool.py:124-133`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L124)) and `spends` table ([`mempool.py:146-149`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L146)), plus a Python dict `_items` for heavy objects. dig-mempool uses HashMaps with `Arc` for the same purpose, avoiding SQLite overhead.

### 8.1 Active Pool

```
items:           HashMap<Bytes32, Arc<MempoolItem>>   // Chia: _items dict + tx table
coin_index:      HashMap<Bytes32, Bytes32>           // coin_id -> spending bundle (Chia: spends table)
mempool_coins:   HashMap<Bytes32, Bytes32>           // coin_id -> creating bundle (CPFP, not in Chia)
```

### 8.2 Dependency Graph

```
dependencies:    HashMap<Bytes32, HashSet<Bytes32>>  // bundle -> parents
dependents:      HashMap<Bytes32, HashSet<Bytes32>>  // bundle -> children
```

### 8.3 Singleton Tracking

```
singleton_spends: HashMap<Bytes32, Vec<Bytes32>>     // launcher_id -> [bundle_ids] in lineage order
```

### 8.4 Identical Spend Index

```
dedup_index:     HashMap<(Bytes32, Bytes32), Bytes32> // (coin_id, solution_hash) -> first bundle_id
```

### 8.5 Pending Pool

```
pending:             HashMap<Bytes32, Arc<MempoolItem>>
pending_coin_index:  HashMap<Bytes32, Bytes32>
```

### 8.6 Conflict Cache

```
conflict_cache:  HashMap<Bytes32, SpendBundle>
```

### 8.7 Fee Estimator

```
fee_tracker:     FeeTracker
```

See Section 10 for `FeeTracker` internals.

### 8.8 Event Hooks

```
hooks:           Vec<Arc<dyn MempoolEventHook>>
```

### 8.9 Concurrency

CLVM validation (Phase 1) runs without holding `pool_lock`, enabling concurrent validation. Phase 2 only holds the write lock for fast HashMap operations.

| Lock | Protects | Type | Hot path |
|------|----------|------|----------|
| `pool_lock` | items, coin_index, mempool_coins, dependencies, dependents, singleton_spends, dedup_index, accumulators | `RwLock` | Phase 2 (write), `select_for_block` (read) |
| `pending_lock` | pending, pending_coin_index, pending_cost | `RwLock` | Phase 2 for timelocked items |
| `conflict_lock` | conflict_cache, conflict_cost | `RwLock` | Phase 2 on conflict |
| `seen_lock` | seen_cache | `RwLock` | Phase 1 (dedup check) |
| `bls_lock` | bls_cache | `Mutex` | Phase 1 (sig verification via dig-clvm) |
| `fee_lock` | fee_tracker | `RwLock` | `on_new_block`, `estimate_fee_rate` |
| `hooks_lock` | hooks | `RwLock` | Under parent write lock |

---

## 9. Lifecycle Events

### 9.1 New Block

Corresponds to Chia's `MempoolManager.new_peak()` ([`mempool_manager.py:858-937`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L858)) and `Mempool.new_tx_block()` ([`mempool.py:329-345`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L329)).

`on_new_block(height, timestamp, spent_coin_ids, confirmed_bundles)`:

1. **Remove confirmed + cascade**: Remove items whose coins are in `spent_coin_ids`. Chia: iterates spent coins and evicts via `get_items_by_coin_id()` ([`mempool_manager.py:900-918`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L900)). dig-mempool additionally cascade-evicts CPFP dependents. Fire `on_item_removed(Confirmed)` and `on_item_removed(CascadeEvicted)`.
2. **Remove expired + cascade**: Remove items past expiry. Chia: `new_tx_block()` SQL query `WHERE assert_before_seconds <= ? OR assert_before_height <= ?` ([`mempool.py:336-340`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L336)). Cascade-evict dependents.
3. **Collect pending promotions**: Extract timelocked items now satisfiable. Chia: `_pending_cache.drain(new_peak.height)` ([`pending_tx_cache.py:91-108`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/pending_tx_cache.py#L91)).
4. **Collect conflict retries**: Extract retryable conflict items. Chia: `_conflict_cache.drain()` ([`pending_tx_cache.py:40-44`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/pending_tx_cache.py#L40)).
5. **Update fee estimator**: Feed `confirmed_bundles` to `FeeTracker`. Chia: `fee_estimator.new_block()` ([`bitcoin_fee_estimator.py:34-36`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/bitcoin_fee_estimator.py#L34)).
6. **Update tracking**: Set height and timestamp.
7. **Return `RetryBundles`** with `conflict_retries`, `pending_promotions`, `cascade_evicted`. Chia re-validates drained items inline via `add_spend_bundle()` (under the lock); dig-mempool returns them for the caller to resubmit with fresh coin records.

### 9.2 Cascade Eviction

When an item is removed:
1. Remove from `items`, `coin_index`, `mempool_coins`, `singleton_spends`, `dedup_index`.
2. Look up `dependents[bundle_id]`.
3. Recursively remove each dependent (depth-first).
4. Clean up `dependencies` and `dependents`.
5. Walk ancestor chain and recompute `descendant_score` for affected ancestors.
6. Fire `on_item_removed` with appropriate `RemovalReason`.

**RBF + CPFP**: When RBF replaces a parent:
- Children are cascade-evicted with `RemovalReason::CascadeEvicted`.
- They do NOT go to the conflict cache (their coins no longer exist).
- They ARE reported in future `RetryBundles::cascade_evicted` or via event hooks.

### 9.3 Reorg Recovery

1. `clear()` -- clears all state except event hooks and config.
2. Caller re-fetches chain state and re-submits.

### 9.4 Memory Pressure

| Severity | Action |
|----------|--------|
| Warning  | `evict_lowest_percent(25)` |
| Critical | `evict_lowest_percent(50)` |

---

## 10. Fee Estimation Tracker

### 10.1 Architecture

The fee estimator uses a **bucket-based tracker** inspired by Bitcoin Core's `CBlockPolicyEstimator`, following Chia's implementation in `BitcoinFeeEstimator` ([`bitcoin_fee_estimator.py:14-78`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/bitcoin_fee_estimator.py#L14)) which wraps `FeeTracker` and `SmartFeeEstimator`. Chia's `FeeEstimatorInterface` protocol ([`fee_estimator_interface.py:11-33`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/fee_estimator_interface.py#L11)) defines the methods: `new_block`, `add_mempool_item`, `remove_mempool_item`, `estimate_fee_rate`. dig-mempool internalizes these as methods on the `FeeTracker` struct.

### 10.2 FeeTracker

```rust
struct FeeTracker {
    /// Number of recent blocks to track.
    window: usize,
    /// Fee-rate buckets (logarithmically spaced from min to max FPC).
    buckets: Vec<FeeBucket>,
    /// Circular buffer of per-block confirmed transaction data.
    block_history: VecDeque<BlockFeeData>,
    /// Current height.
    current_height: u64,
}

struct FeeBucket {
    /// Lower bound of this bucket's fee rate (scaled FPC).
    fee_rate_lower: u128,
    /// Upper bound.
    fee_rate_upper: u128,
    /// Number of transactions confirmed within 1 block at this rate.
    confirmed_in_1: f64,
    /// Within 2 blocks.
    confirmed_in_2: f64,
    /// Within 5 blocks.
    confirmed_in_5: f64,
    /// Within 10 blocks.
    confirmed_in_10: f64,
    /// Total transactions observed at this rate.
    total_observed: f64,
}

struct BlockFeeData {
    pub height: u64,
    pub min_fpc_included: u128,
    pub max_fpc_included: u128,
    pub median_fpc: u128,
    pub num_transactions: usize,
}
```

### 10.3 Estimation Algorithm

When `estimate_fee_rate(target_blocks)` is called:

1. For each bucket (from highest to lowest fee rate):
   - Compute `success_rate = confirmed_in_N / total_observed` where N is the target.
   - If `success_rate >= 0.85` (85% confidence), this bucket's fee rate is sufficient.
2. Return the lower bound of the first sufficient bucket.
3. If no bucket has 85% confidence, return `None` (insufficient data).

### 10.4 Decay

Older observations are decayed exponentially to weight recent blocks more heavily. The decay factor is `0.998` per block, meaning data from 100 blocks ago has ~82% weight, and from 500 blocks ago has ~37% weight.

### 10.5 Serialization

`FeeTracker` state is included in `MempoolSnapshot` via `FeeEstimatorState`, allowing the fee estimator to survive restarts.

```rust
#[derive(Serialize, Deserialize)]
pub struct FeeEstimatorState {
    pub buckets: Vec<SerializedBucket>,
    pub block_history: Vec<BlockFeeData>,
    pub current_height: u64,
}
```

---

## 11. Singleton Fast-Forward

### 11.1 Overview

A **singleton** is a coin that, when spent, creates exactly one child coin with a specific puzzle structure (singleton top-layer puzzle wrapping an inner puzzle). Singletons are identified by a `launcher_id` that remains constant across all versions. Chia implements this via `BundleCoinSpend.latest_singleton_lineage` ([`mempool_item.py:35-42`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L35)) and `SingletonFastForward` ([`eligible_coin_spends.py`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/eligible_coin_spends.py)).

Multiple sequential spends of the same singleton can be admitted to the mempool. The fast-forward optimization allows the block builder to include all of them in a single block by rebasing each spend's lineage proof to reference the previous spend's output. Chia handles FF rebasing during `new_peak()` ([`mempool_manager.py:921-937`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L921)) and block generation ([`mempool.py:596-600`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L596)).

### 11.2 Detection (Delegated to Chia Crates)

Detection uses existing Chia crate functions -- the mempool does NOT implement singleton puzzle parsing itself.

During Phase 1, for each `CoinSpend`:
1. Check `OwnedSpendConditions.flags & ELIGIBLE_FOR_FF` (set by `chia-consensus::MempoolVisitor`).
2. If flagged, call `chia_consensus::fast_forward::supports_fast_forward(&coin_spend)` to confirm puzzle structure. This function validates against `SINGLETON_TOP_LAYER_V1_1_HASH` from `chia-puzzles`.
3. If confirmed, use `chia_sdk_driver::SingletonLayer::parse_puzzle()` to extract:
   - `launcher_id` from the curried singleton args.
   - Inner puzzle hash.
   - Lineage proof from the solution via `SingletonLayer::parse_solution()`.
4. Construct `SingletonLineageInfo` from the parsed data.

No custom singleton puzzle parsing or hash comparison is implemented by the mempool.

### 11.3 Admission Rules

During Phase 2, if `singleton_lineage` is present:
1. Check `singleton_spends[launcher_id]` for existing items.
2. **Sequential update**: If the new item spends the coin created by the latest existing item for this launcher_id, append to the chain. This is not a conflict -- it's a sequential singleton update. Chia handles this via `check_removals()` FF special case ([`mempool_manager.py:274-276`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L274)).
3. **Conflicting update**: If the new item spends an older version of the singleton (not the latest), treat as a conflict. Apply RBF rules. Only the latest chain is kept.
4. **Fresh singleton**: If no existing items for this launcher_id, add normally.

### 11.4 Block Selection Integration

During block selection, singleton chains are treated as dependency chains:
- All items in a singleton chain must be included together or not at all.
- They are ordered by lineage (oldest first) in the output.
- The total cost is the sum of all items in the chain.
- Chia uses `SingletonFastForward` tracker during block building ([`mempool.py:600`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L600)).

### 11.5 Lineage Rebasing (Outside Mempool Scope)

The mempool does NOT perform lineage rebasing. This is block production logic, outside the mempool's boundary. The mempool's responsibility ends at returning singleton chain items in correct lineage order via `select_for_block()`.

The **caller** (block producer) then:
1. Uses `chia_consensus::fast_forward::fast_forward_singleton()` to rebase each singleton spend's lineage proof.
2. Passes the rebased spends to `dig_clvm::build_block_generator()` for compression and aggregation.

This separation ensures the mempool remains a pure state manager with no block construction logic.

---

## 12. Identical Spend Deduplication

### 12.1 Overview

When multiple bundles contain a `CoinSpend` for the same coin with the same puzzle and solution, the CLVM result is identical. The block builder can run the puzzle once and reuse the result, reducing effective block cost. Chia implements this via `IdenticalSpendDedup` ([`eligible_coin_spends.py`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/eligible_coin_spends.py)) and the `ELIGIBLE_FOR_DEDUP` flag. Per-spend cost is tracked in `BundleCoinSpend.cost` ([`mempool_item.py:32`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L32)).

### 12.2 Canonical Encoding Requirement

For dedup eligibility, the solution must use canonical CLVM encoding. **This check is performed by chia-consensus** (via `MempoolVisitor`) during CLVM execution when `MEMPOOL_MODE` is set. The mempool does not implement its own canonical encoding check -- it reads the `ELIGIBLE_FOR_DEDUP` flag from `OwnedSpendConditions.flags`.

The underlying canonical encoding rules enforced by chia-consensus (Chia reference: `is_clvm_canonical()` at [`mempool_manager.py:185-226`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L185)):
- No back-references (`0xFE` byte) in the serialized CLVM tree ([`mempool_manager.py:208-209`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L208)).
- All atoms use minimal-length encoding ([`mempool_manager.py:143-182`](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L143) `is_atom_canonical()`).

Solutions with back-references could have different serialized forms that produce the same tree, making dedup unreliable. Canonical encoding guarantees byte-level equality implies semantic equality.

### 12.3 Dedup Index

```
dedup_index: HashMap<(Bytes32, Bytes32), Bytes32>
// (coin_id, sha256(solution)) -> first bundle_id that established the cost
```

### 12.4 Cost Adjustment

When a new item has a dedup-eligible spend that matches an existing entry in `dedup_index`:
- The CLVM cost for that specific spend is not added to the new item's `cost`.
- The new item's `virtual_cost` and `fee_per_virtual_cost_scaled` reflect the reduced cost.
- The cost saving is tracked: `cost_saving: u64` field on `MempoolItem`.

This incentivizes bundles to use standard puzzle/solution patterns and allows more transactions per block.

### 12.5 Dedup Invalidation

When the "first bundle" (the one that established the cost) is removed:
- Find the next item with the same dedup key and transfer the cost establishment.
- Re-compute affected items' costs and fee rates.
- Update the dedup_index to point to the new cost-bearer.

---

## 13. Cross-Bundle Announcement Validation

### 13.1 Scope

When a CPFP child depends on a parent, the child's CLVM conditions may include assertions that can only be satisfied by the parent's conditions:
- `ASSERT_COIN_ANNOUNCEMENT(hash)` expecting parent's `CREATE_COIN_ANNOUNCEMENT`.
- `ASSERT_PUZZLE_ANNOUNCEMENT(hash)` expecting parent's `CREATE_PUZZLE_ANNOUNCEMENT`.
- `RECEIVE_MESSAGE(mode, msg)` expecting parent's `SEND_MESSAGE(mode, msg)`.

### 13.2 Validation

Uses `chia_sdk_types::announcement_id(coin_info, message)` to compute announcement hashes for matching. This function computes `sha256(coin_info || message)` which is the standard Chia announcement ID format.

During Phase 2, for each CPFP item:
1. Collect all unsatisfied assertion conditions from the item's CLVM output (assertions not satisfied intra-bundle). Use the `Condition<T>` enum from `chia-sdk-types` for type-safe condition matching (`AssertCoinAnnouncement`, `AssertPuzzleAnnouncement`, `ReceiveMessage`).
2. For each unsatisfied assertion, compute the expected announcement ID via `announcement_id()` and search the ancestor chain's conditions for a matching creation (`CreateCoinAnnouncement`, `CreatePuzzleAnnouncement`, `SendMessage`).
3. If an assertion cannot be satisfied by any ancestor, it is **not rejected** at the mempool level -- it may be satisfied by another bundle in the same block. A warning is logged but admission proceeds.
4. If an assertion IS satisfied by an ancestor, record the cross-bundle binding for debugging and bookkeeping.

**Rationale**: The mempool cannot know the full block contents. Rejecting unsatisfied cross-bundle assertions would be too strict. However, for CPFP chains where the dependency is explicit, verifying ancestor-to-descendant assertions provides an additional correctness guarantee.

---

## 14. Compatibility Notes

### 14.1 Chia Compatibility

Preserved behaviors: coin identity, implicit fees, CLVM conditions (51-90), BLS signatures with 8 domain variants, RBF rules, seen-cache DoS protection, pending/conflict separation.

### 14.2 DIG L2 vs Chia L1

| Aspect | Chia L1 | DIG L2 |
|--------|---------|--------|
| Block cost limit | 11B | 550B (configurable) |
| Per-spend cost | 11B | 11B (same) |
| Spends per block | 6,000 | 6,000 (configurable) |
| Mempool capacity | 15x block | 15x block (same ratio) |
| Genesis challenge | Chia mainnet | DIG-specific |
| Storage | SQLite | HashMap + Arc |
| Concurrency | Single-threaded | Two-phase lock-free |
| Virtual cost | 500K/spend | 500K/spend (same) |
| CPFP | **Not supported** | Depth-25 chains, package fee rates |
| Batch submit | **Not supported** | Parallel Phase 1 |
| Expiry protection | 15-min window | Configurable block window |
| Pending dedup | **Not implemented** | Pending-vs-pending RBF |
| Block selection | Single greedy | 4-way greedy + custom strategy |
| Singleton FF | Supported | Supported |
| Spend dedup | Supported | Supported |
| Fee estimator | Bitcoin-style | Bitcoin-style (bucket tracker) |
| Event hooks | **None** | Synchronous callback trait |
| Persistence | **None** | Snapshot/restore serialization |
| Custom selection | **None** | `BlockSelectionStrategy` trait |
| Cross-bundle announcements | **Not validated** | Validated for CPFP chains |
| Admission policies | **None** | `AdmissionPolicy` trait |

### 14.3 Crate Boundary

`dig-mempool` is a **library crate** (`lib`). It is strictly a **mempool state manager**: inputs are transaction submissions, outputs are selected mempool items. It does **not** include:
- **Block production** (compression, generator serialization, signature aggregation). The caller uses `dig-clvm::build_block_generator()` with the items returned by `select_for_block()`.
- **Block validation** (executing block generators, verifying block-level conditions). The caller uses `dig-clvm::validate_block()`.
- **Singleton lineage rebasing** (rewriting FF proofs for block inclusion). The caller uses `chia_consensus::fast_forward_singleton()`.
- **Networking** (gossip, relay, peer management).
- **Persistent storage I/O** (caller uses `snapshot()` / `restore()` for serialization, handles I/O).
- **RPC endpoints**.
- **Consensus rules** beyond mempool admission (fork choice, finality, chain selection).

The mempool's contract is:
- **Input**: `SpendBundle` + `CoinRecord`s (via `submit()` / `submit_batch()`)
- **Output**: `Vec<Arc<MempoolItem>>` (via `select_for_block()`) -- ordered, non-conflicting items ready for the block producer to consume

---

## 15. Thread Safety

`Mempool` is `Send + Sync`. CLVM validation (Phase 1) runs lock-free, enabling concurrent `submit()` calls. The write lock (Phase 2) is only held for fast HashMap operations.

| Operation | Locks | Contention |
|-----------|-------|------------|
| `submit` Phase 1 | `seen_lock` (write), `bls_lock` (Mutex) | Concurrent CLVM; serialized BLS cache |
| `submit` Phase 2 | `pool_lock` or `pending_lock` (write) | Fast HashMap ops only |
| `submit_batch` Phase 1 | `bls_lock` per bundle | Parallel CLVM validation |
| `submit_batch` Phase 2 | `pool_lock` (write, once) | Single lock acquisition for all |
| `select_for_block` | `pool_lock` (read) | Concurrent with Phase 1 |
| `on_new_block` | All locks (write) | Brief, infrequent |
| `get` / `stats` | `pool_lock` (read) | Concurrent |
| `estimate_fee_rate` | `fee_lock` (read) | Concurrent |
| Event hooks | `hooks_lock` (read) | Under parent write lock |

---

## 16. Testing Strategy

### 16.1 Unit Tests

- **Deduplication**: seen cache, active/pending/conflict hit, concurrent race.
- **Structural**: duplicate spends.
- **Coin records**: missing, spent, mempool coin (CPFP), ephemeral coins.
- **CLVM**: valid, invalid puzzle, bad signature.
- **Fee**: positive, zero, negative, RESERVE_FEE.
- **Virtual cost**: penalty calculation, FPC uses virtual cost.
- **Timelocks**: relative->absolute, pending, expired, impossible, CPFP coin timelocks.
- **Conflicts**: single, multi, active-only scope.
- **RBF**: superset/FPC/bump pass+fail, conflict cache on fail, RBF+CPFP cascade.
- **CPFP**: single dep, chain, depth limit, cycle, package fees, cascade eviction, cross-bundle announcements.
- **Singleton FF**: detection, sequential admission, conflicting update, block selection ordering.
- **Identical dedup**: canonical check, cost adjustment, dedup invalidation on removal.
- **Capacity**: eviction by descendant_score, expiry protection, expiring item preference.
- **Pending**: limits, deduplication.
- **Conflict cache**: limits, drain.
- **Fee estimator**: bucket placement, estimation, decay, insufficient data.
- **Admission policy**: accept, reject.
- **Batch**: parallel validation, sequential insert, intra-batch CPFP.
- **Events**: hook firing, removal reasons.
- **Persistence**: snapshot round-trip, restore correctness.
- **Custom strategy**: delegated selection, constraint enforcement.
- **Spend count**: per-bundle, per-block limit.

### 16.2 Integration Tests

- Full lifecycle: submit -> select -> on_new_block -> removal.
- RBF + cascade eviction of CPFP chain.
- Conflict cache retry flow.
- Pending promotion flow.
- CPFP: low-fee parent + high-fee child -> both selected.
- CPFP cascade: parent removal -> child removal.
- Singleton FF: sequential updates -> all in one block.
- Reorg recovery: clear + resubmit.
- Multi-strategy selection optimality.
- Concurrent submit + select_for_block.
- Batch with intra-batch CPFP.
- Caller workflow: on_new_block -> resubmit -> select.
- Expiry protection under eviction pressure.
- Fee estimation accuracy over simulated block history.
- Snapshot -> clear -> restore -> verify state equality.
- Custom BlockSelectionStrategy invocation.
- Event hook ordering and completeness.

### 16.3 Property Tests

- Conservation: total_fees + total output == total input.
- Determinism: identical inputs -> identical outputs.
- No conflicts: selected bundles never share spent coins.
- Topological order: parents before children in selection output.
- Monotonic eviction: lowest descendant_score first (skipping protected).
- Capacity invariant: total_cost <= max_total_cost after any operation.
- Dependency invariant: if B depends on A, A present whenever B present.
- Package fee correctness: package_fee == sum(ancestor fees) + self fee.
- Descendant score: >= own FPC and >= any descendant's package FPC.
- Fee-rate ordering: within each topological layer, FPC is non-increasing.
- Singleton lineage: items for same launcher_id are in valid chain order.

---

## 17. Performance Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| `submit` Phase 1 | < 50ms p99 | CLVM execution + BLS verification (via dig-clvm) |
| `submit` Phase 2 | < 1ms p99 | HashMap operations only |
| `submit_batch` (100) | < 200ms p99 | Parallel Phase 1, single Phase 2 |
| `select_for_block` | < 100ms p99 | 4 sorts + greedy + CPFP walks |
| `on_new_block` | < 10ms p99 | Index lookups + cascade removals |
| `get` / `contains` | < 1us | HashMap + Arc clone |
| `stats` | < 1us | Cached accumulators |
| `estimate_fee_rate` | < 100us | Bucket scan |
| `snapshot` | < 50ms | Serialization of full state |
| `restore` | < 100ms | Deserialization + index rebuild |
| Memory per item | ~2-5 KB | SpendBundle + metadata |
| Max active items | ~50,000 | Bounded by max_total_cost |
| Max dependency depth | 25 | Configurable |

---

## Appendix A: Chia L1 Reference Index

Consolidated index of Chia source references. Citations are also inlined throughout the spec body. All paths are relative to the repo root at [`github.com/Chia-Network/chia-blockchain`](https://github.com/Chia-Network/chia-blockchain). Line numbers reference the `main` branch as of 2026-04-12.

### A.1 Core Mempool (chia/full_node/mempool.py)

**Class and storage:**
- `Mempool` class definition: [mempool.py:94](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L94). Uses SQLite in-memory database for indexed queries and a Python dict `_items: dict[bytes32, InternalMempoolItem]` for fast access to heavy objects (G2Element signatures).
- Constructor: [mempool.py:107-113](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L107). Initializes `_db_conn`, `_items`, `_block_height`, `_timestamp`, `_total_fee`, `_total_cost`.
- TX table schema: [mempool.py:124-133](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L124). Columns: `name BLOB, cost INT, fee INT, assert_height INT, assert_before_height INT, assert_before_seconds INT, fee_per_cost REAL, priority REAL, seq INTEGER PRIMARY KEY AUTOINCREMENT`.
- Spends table: [mempool.py:146-149](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L146). Maps `coin_id BLOB` to `tx BLOB` (bundle hash). This is the **coin index** used for conflict detection. dig-mempool mirrors this with `coin_index: HashMap<Bytes32, Bytes32>`.
- Indexes: [mempool.py:136-153](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L136). `name_idx`, `feerate`, `priority_idx`, `assert_before` (partial index on expiring items), `spend_by_coin`, `spend_by_bundle`.

**Conflict detection:**
- `get_items_by_coin_ids()`: [mempool.py:290-299](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L290). Queries the spends table: `SELECT * FROM tx WHERE name IN (SELECT tx FROM spends WHERE coin_id IN (...))`. This only searches **active** items, not pending or conflict caches. dig-mempool mirrors this: conflict detection is active-pool-only (Section 5.4.4).

**Fee rate minimum:**
- `get_min_fee_rate()`: [mempool.py:301-327](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L301). Returns 0 if not at capacity. Otherwise iterates ascending `fee_per_cost` removing items until the new tx fits. dig-mempool's `estimate_min_fee()` uses the same concept with three tiers (Section 3.6).
- `at_full_capacity()`: [mempool.py:509-514](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L509). `total_cost + cost > max_size_in_cost`.

**Block expiry removal:**
- `new_tx_block()`: [mempool.py:329-345](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L329). SQL: `SELECT name FROM tx WHERE assert_before_seconds <= ? OR assert_before_height <= ?`. Then calls `remove_from_pool()`. dig-mempool's `on_new_block()` step 2 mirrors this (Section 9.1).

**Eviction on capacity:**
- `add_to_pool()`: [mempool.py:395-502](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L395).
  - Expiring item special handling: [mempool.py:406-442](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L406). Items expiring within 48 blocks (~15 min) or 900 seconds are evicted first, only if their `priority < new_item.fee_per_virtual_cost`. dig-mempool implements this as expiry protection (Section 5.4.7).
  - General capacity eviction: [mempool.py:446-460](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L446). Uses SQL window function `SUM(cost) OVER (ORDER BY priority DESC, seq ASC)` to find lowest-priority items. dig-mempool sorts by `descendant_score` ascending (improvement over Chia).
  - TX insertion: [mempool.py:462-492](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L462). Inserts into `tx` table and `spends` table. Note singleton FF spends are indexed by `latest_singleton_lineage.coin_id` ([mempool.py:488-489](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L488)).

**Block generator construction:**
- `create_bundle_from_mempool_items()`: [mempool.py:583-615](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L583). Iterates `SELECT name, fee FROM tx ORDER BY priority DESC, seq ASC`, collecting items until cost or spend count limits are reached. Uses `IdenticalSpendDedup` ([mempool.py:596](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L596)) and `SingletonFastForward` ([mempool.py:600](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L600)).
- `create_block_generator()`: [mempool.py:516-581](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L516). Calls `solution_generator_backrefs()` ([mempool.py:540](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L540)) for CLVM back-reference compression, then validates with `run_block_generator2()` ([mempool.py:551](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L551)). dig-mempool delegates this to `dig_clvm::build_block_generator()` which calls the same underlying functions.

**Constants:**
- `MAX_SKIPPED_ITEMS = 10`: [mempool.py:49](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L49).
- `PRIORITY_TX_THRESHOLD = 3`: [mempool.py:54](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L54).
- `MempoolRemoveReason` enum: [mempool.py:87-91](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool.py#L87). `CONFLICT`, `BLOCK_INCLUSION`, `POOL_FULL`, `EXPIRED`. dig-mempool's `RemovalReason` enum (Section 3.10) extends this with `ReplacedByFee`, `CascadeEvicted`, `ExplicitRemoval`, `Cleared`.

### A.2 MempoolItem (chia/types/mempool_item.py)

- `SPEND_PENALTY_COST = 500_000`: [mempool_item.py:14](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L14). dig-mempool uses the same constant (Section 2.5).
- `UnspentLineageInfo`: [mempool_item.py:18-22](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L18). Fields: `coin_id`, `parent_id`, `parent_parent_id`. dig-mempool's `SingletonLineageInfo` (Section 2.3) extends this with `launcher_id` and `inner_puzzle_hash`.
- `BundleCoinSpend`: [mempool_item.py:25-42](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L25). Per-spend data: `coin_spend`, `eligible_for_dedup`, `additions`, `cost`, `latest_singleton_lineage`. dig-mempool tracks dedup and FF eligibility as top-level flags on `MempoolItem`.
- `MempoolItem`: [mempool_item.py:45-120](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L45). Frozen dataclass with `aggregated_signature`, `fee`, `conds`, `spend_bundle_name`, `height_added_to_mempool`, `assert_height`, `assert_before_height`, `assert_before_seconds`, `bundle_coin_spends`.
- `virtual_cost` property: [mempool_item.py:92-93](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L92). `cost + num_spends * SPEND_PENALTY_COST`. dig-mempool computes this identically (Section 5.3.5).
- `fee_per_virtual_cost` property: [mempool_item.py:76-77](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/types/mempool_item.py#L76). `fee / virtual_cost`. Used as `priority` in the SQL table. dig-mempool uses `fee_per_virtual_cost_scaled` (integer arithmetic with `FPC_SCALE`).

### A.3 Mempool Manager (chia/full_node/mempool_manager.py)

**RBF constant:**
- `MEMPOOL_MIN_FEE_INCREASE = 10_000_000`: [mempool_manager.py:52](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L52). 0.00001 XCH. dig-mempool uses `MIN_RBF_FEE_BUMP = 10_000_000` (Section 2.5).

**Timelock resolution:**
- `compute_assert_height()`: [mempool_manager.py:81-126](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L81). Resolves relative timelocks to absolute using `removal_coin_records[coin_id].confirmed_block_index` and `.timestamp`. Returns `TimelockConditions` with `assert_height`, `assert_seconds`, `assert_before_height`, `assert_before_seconds`. dig-mempool implements identical resolution logic (Section 5.3.6).

**Canonical encoding check:**
- `is_clvm_canonical()`: [mempool_manager.py:185-226](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L185). Verifies no back-references (byte `0xFE`) and all atoms use minimal-length encoding. This check is performed by chia-consensus's MempoolVisitor during CLVM execution; dig-mempool reads the resulting `ELIGIBLE_FOR_DEDUP` flag (Section 5.3.7).

**Conflict detection:**
- `check_removals()`: [mempool_manager.py:229-292](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L229). For each coin, checks if `spent` (DOUBLE_SPEND), then queries mempool for conflicts. Handles FF and dedup special cases ([mempool_manager.py:270-288](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L270)). dig-mempool uses `coin_index` HashMap lookups for the same purpose.

**Validation flow:**
- `validate_spend_bundle()`: [mempool_manager.py:609-833](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L609). The main validation method. Key steps:
  - Cost check: [mempool_manager.py:733-734](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L733). `cost > self.max_tx_clvm_cost`.
  - Fee limit: [mempool_manager.py:740](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L740). `MEMPOOL_ITEM_FEE_LIMIT = 2^50`.
  - Fee rate check at capacity: [mempool_manager.py:745-752](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L745). Only checked when `at_full_capacity()`.
  - Puzzle hash verification: [mempool_manager.py:765-771](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L765).
  - Timelock check via Rust: [mempool_manager.py:779-784](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L779). `check_time_locks()`.
  - Impossible constraint detection: [mempool_manager.py:791-796](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L791). `assert_before_height <= assert_height`.
  - Dedup eligibility + canonical check: [mempool_manager.py:662-663](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L662).
  - FF eligibility + lineage lookup: [mempool_manager.py:666-675](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L666).
  - Ephemeral coin handling: [mempool_manager.py:704-723](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L704). Creates synthetic `CoinRecord` with `confirmed_block_index = peak.height + 1` and `timestamp = peak.timestamp`. dig-mempool uses `current_height` and `current_timestamp` for the same purpose (Section 5.3.6).

**Submission result routing:**
- [mempool_manager.py:587-607](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L587). `MEMPOOL_CONFLICT` -> conflict cache; timelock failure with valid item -> pending cache; otherwise -> failed.

**RBF rules:**
- `can_replace()`: [mempool_manager.py:1077-1126](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1077).
  - Superset rule: [mempool_manager.py:1101-1109](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1101). Every coin in conflicting items must be in new item.
  - Fee-per-cost comparison: [mempool_manager.py:1119-1126](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1119). `new_item.fee_per_cost <= conflicting_fees / conflicting_cost`.
  - Minimum fee increase: `new_item.fee < conflicting_fees + MEMPOOL_MIN_FEE_INCREASE`.
  - FF/dedup protection: [mempool_manager.py:1090-1113](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L1090). Cannot remove dedup/FF eligibility via RBF.

**New peak (block confirmation):**
- `new_peak()`: [mempool_manager.py:858-937](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L858).
  - Expiry removal: [mempool_manager.py:879](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L879). `mempool.new_tx_block()`.
  - Fast path (optimization): [mempool_manager.py:887-937](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L887). Used when the new peak is a direct successor. Iterates `spent_coins`, evicts regular spends, defers FF spends for rebasing.
  - Pending/conflict drain: drain pending cache items whose height is now satisfied and all conflict cache items, then re-validate via `add_spend_bundle()`.

### A.4 Pending and Conflict Caches (chia/full_node/pending_tx_cache.py)

- `ConflictTxCache`: [pending_tx_cache.py:13-47](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/pending_tx_cache.py#L13). `_cache_max_size = 1000`, `_cache_max_total_cost`. FIFO eviction. `drain()` returns all items and clears. dig-mempool uses the same limits (Section 2.4).
- `PendingTxCache`: [pending_tx_cache.py:50-111](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/pending_tx_cache.py#L50). `_cache_max_size = 3000`, organized by `assert_height` in `SortedDict`. `drain(up_to_height)` returns items whose height is satisfied. Eviction removes **highest** assert_height first (furthest future items are least useful). dig-mempool mirrors these limits (Section 2.4) and adds pending-vs-pending deduplication (Section 5.4.9).

### A.5 Condition Costs (chia/consensus/condition_costs.py)

- [condition_costs.py:1-15](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/consensus/condition_costs.py#L1):
  - `AGG_SIG = 1,200,000` -- G1 subgroup check + aggregated signature validation.
  - `CREATE_COIN = 1,800,000` -- per output coin.
  - `MESSAGE_CONDITION_COST = 700` -- SEND/RECEIVE_MESSAGE and CREATE/ASSERT_ANNOUNCEMENT.
  - `GENERIC_CONDITION_COST = 200` -- all other conditions.

These costs are applied during CLVM execution by `chia-consensus` and are included in the `cost` field returned by `dig-clvm::validate_spend_bundle()`.

### A.6 Fee Estimation (chia/full_node/bitcoin_fee_estimator.py)

- `BitcoinFeeEstimator`: [bitcoin_fee_estimator.py:14-78](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/bitcoin_fee_estimator.py#L14). Wraps `FeeTracker` and `SmartFeeEstimator`.
  - `new_block()`: [bitcoin_fee_estimator.py:34-36](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/bitcoin_fee_estimator.py#L34). Calls `tracker.process_block()`.
  - `add_mempool_item()` / `remove_mempool_item()`: [bitcoin_fee_estimator.py:38-44](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/bitcoin_fee_estimator.py#L38).
  - `estimate_fee_rate()`: [bitcoin_fee_estimator.py:46-53](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/bitcoin_fee_estimator.py#L46). Delegates to `SmartFeeEstimator.get_estimate(time_offset_seconds)`.
- `FeeEstimatorInterface` protocol: [fee_estimator_interface.py:11-33](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/fee_estimator_interface.py#L11). Methods: `new_block_height`, `new_block`, `add_mempool_item`, `remove_mempool_item`, `estimate_fee_rate`, `mempool_size`, `mempool_max_size`. dig-mempool internalizes this as the `FeeTracker` struct (Section 10).

---

## Appendix B: dig-clvm Interaction Reference

This appendix documents how `dig-mempool` interacts with `dig-clvm` at the function and type level. All paths reference the `dig-clvm` crate at `github.com/DIG-Network/dig-clvm`.

### B.1 Crate Structure

**Cargo.toml dependencies (dig-clvm v0.1.0):**
- `clvmr = "0.14"` -- CLVM runtime (allocator, cost type, node pointer)
- `chia-consensus = "0.26"` -- `run_spendbundle`, `validate_clvm_and_signature`, `run_block_generator2`, `solution_generator_backrefs`, flags, `OwnedSpendBundleConditions`
- `chia-bls = "0.26"` -- `BlsCache`, `Signature`, `PublicKey`, `aggregate_verify`
- `chia-protocol = "0.26"` -- `SpendBundle`, `Coin`, `CoinSpend`, `Program`, `Bytes32`
- `chia-sdk-coinset = "0.30"` -- `CoinRecord`
- `chia-sdk-types = "0.30"` -- `Condition`, `Conditions`
- `chia-sdk-driver = "0.30"` -- `SpendContext`, `SpendWithConditions`
- `chia-puzzles = "0.20"` -- Singleton puzzle definitions
- `dig-constants = "0.1.0"` -- `NetworkConstants`, `DIG_MAINNET`, `DIG_TESTNET`

**Re-exports (dig-clvm/src/lib.rs:1-44):**

dig-clvm re-exports all key types so that `dig-mempool` only needs to depend on `dig-clvm` (and `dig-constants`), not on the individual Chia crates:

```rust
// Types the mempool uses directly:
pub use chia_protocol::{Bytes, Bytes32, Coin, CoinSpend, SpendBundle};
pub use chia_bls::{BlsCache, Signature};
pub use chia_sdk_coinset::CoinRecord;
pub use chia_consensus::owned_conditions::OwnedSpendBundleConditions;  // via consensus module
pub use dig_constants::{NetworkConstants, DIG_MAINNET, DIG_TESTNET};
pub use clvmr::cost::Cost;
```

### B.2 ValidationConfig (dig-clvm/src/consensus/config.rs:1-29)

```rust
pub const L1_MAX_COST_PER_SPEND: Cost = 11_000_000_000;
pub const L2_MAX_COST_PER_BLOCK: Cost = 550_000_000_000;

pub struct ValidationConfig {
    pub max_cost_per_spend: Cost,   // Default: 11B
    pub max_cost_per_block: Cost,   // Default: 550B
    pub flags: u32,                 // chia-consensus flags
}
```

**Mempool usage:** The mempool constructs a `ValidationConfig` with `flags: MEMPOOL_MODE` for strict validation during `submit()`. The `MEMPOOL_MODE` flag (from `chia_consensus::flags`) enables stricter condition checking that rejects certain edge cases allowed in block validation. The `max_cost_per_block` is used as the CLVM cost budget during execution.

**Flag constants available from `chia_consensus::flags`:**
- `MEMPOOL_MODE` -- Strict validation: reject non-standard conditions. Used during mempool submission.
- `DONT_VALIDATE_SIGNATURE` -- Skip BLS aggregate signature verification. Used by `build_block_generator()` since signatures are verified separately.
- `0` (no flags) -- Standard block validation.

### B.3 ValidationContext (dig-clvm/src/consensus/context.rs:1-26)

```rust
pub struct ValidationContext {
    pub height: u32,
    pub timestamp: u64,
    pub constants: NetworkConstants,
    pub coin_records: HashMap<Bytes32, CoinRecord>,
    pub ephemeral_coins: HashSet<Bytes32>,
}
```

**Mempool construction during submit():**

```rust
// Phase 1: Build context from caller-provided data
let mut ephemeral = HashSet::new();
for coin_id in mempool_coin_candidates {
    ephemeral.insert(coin_id);  // Coins created by other mempool items (CPFP)
}

let ctx = ValidationContext {
    height: current_height as u32,
    timestamp: current_timestamp,
    constants: self.network_constants.clone(),
    coin_records: coin_records.clone(),  // Caller-provided on-chain records
    ephemeral_coins: ephemeral,
};
```

**Key field semantics:**
- `coin_records`: Must contain a `CoinRecord` for every on-chain coin being spent. The mempool caller is responsible for loading these from the chain database. For CPFP coins, the caller uses `get_mempool_coin_record()` to synthesize records.
- `ephemeral_coins`: Coin IDs that exist in the mempool but not on-chain. `validate_spend_bundle()` accepts these for existence checks ([validate.rs:57](https://github.com/DIG-Network/dig-clvm)) but does NOT create CoinRecords for them -- the caller must include synthetic records in `coin_records` if relative timelocks are present. Matches Chia's ephemeral coin handling at [mempool_manager.py:707-723](https://github.com/Chia-Network/chia-blockchain/blob/main/chia/full_node/mempool_manager.py#L707).
- `height` / `timestamp`: Passed to `chia-consensus` for condition evaluation. The mempool passes the current L2 block height and timestamp as received from the caller.

### B.4 validate_spend_bundle() (dig-clvm/src/consensus/validate.rs:30-167)

This is the **primary function** the mempool calls during Phase 1 validation. Full signature:

```rust
pub fn validate_spend_bundle(
    bundle: &SpendBundle,
    context: &ValidationContext,
    config: &ValidationConfig,
    _bls_cache: Option<&mut BlsCache>,
) -> Result<SpendResult, ValidationError>
```

**Step-by-step execution:**

1. **Duplicate spend check** (lines 39-45): Iterates `bundle.coin_spends`, inserts each `coin_id` into a `HashSet`. If insertion returns `false`, returns `ValidationError::DoubleSpend(coin_id)`. The mempool also checks this in Phase 1 structural validation (Section 5.3.1), providing an earlier exit before the more expensive CLVM execution.

2. **Coin existence and spent-status check** (lines 48-62): For each coin spend:
   - If `coin_id` is in `context.coin_records` and `record.spent == true` -> `AlreadySpent(coin_id)`.
   - If `coin_id` is NOT in `coin_records` AND NOT in `ephemeral_coins` -> `CoinNotFound(coin_id)`.
   - This is where CPFP works: mempool-created coin IDs placed in `ephemeral_coins` pass this check.

3. **CLVM execution + BLS verification** (lines 64-123): Three paths based on flags and cache:

   **Path A: Skip signatures** (`DONT_VALIDATE_SIGNATURE` flag set, line 76-88):
   ```rust
   let mut a = make_allocator(LIMIT_HEAP);
   let (sbc, _pkm_pairs) = run_spendbundle(&mut a, bundle, max_cost, height, flags, consensus)?;
   OwnedSpendBundleConditions::from(&a, sbc)
   ```
   Calls `chia_consensus::spendbundle_conditions::run_spendbundle()` which executes all puzzles with solutions, parses conditions, computes cost, but skips BLS aggregate signature verification. Used by `build_block_generator()` for cost estimation.

   **Path B: With BLS cache** (cache provided, lines 89-115):
   ```rust
   let (sbc, pkm_pairs) = run_spendbundle(&mut a, bundle, max_cost, height, flags, consensus)?;
   let sig_valid = cache.aggregate_verify(
       pks_msgs.iter().map(|(pk, msg)| (pk, msg.as_ref())),
       &bundle.aggregated_signature,
   );
   ```
   Runs CLVM first, then uses `BlsCache::aggregate_verify()` which checks cached pairings before computing expensive new ones. **This is the path the mempool uses for `submit()`** because it provides both CLVM execution and cached signature verification.

   **Path C: Full validation without cache** (no cache, lines 116-123):
   ```rust
   let (owned_conditions, _validation_pairs, _duration) =
       validate_clvm_and_signature(bundle, max_cost, consensus, height)?;
   ```
   Calls `chia_consensus::spendbundle_validation::validate_clvm_and_signature()` which handles everything in one call. Slower for repeated calls because it can't reuse pairing results.

4. **Cost enforcement** (lines 125-131): `conditions.cost > config.max_cost_per_block` -> `CostExceeded`. The mempool checks `cost > config.max_bundle_cost` separately (Section 5.3.5) since the per-bundle limit may differ from the per-block limit.

5. **Extract additions and removals** (lines 133-146):
   ```rust
   let removals: Vec<Coin> = bundle.coin_spends.iter().map(|cs| cs.coin).collect();
   let additions: Vec<Coin> = conditions.spends.iter().flat_map(|spend| {
       spend.create_coin.iter().map(|cc| Coin::new(parent_id, cc.0, cc.1))
   }).collect();
   ```
   Additions are derived from `CREATE_COIN` conditions. Each `create_coin` tuple is `(puzzle_hash, amount, memo)`. The mempool stores both `additions` and `removals` on the `MempoolItem`.

6. **Conservation check** (lines 148-159): `total_input < total_output` -> `ConservationViolation`. Fee = `total_input - total_output`.

**Return value:** `SpendResult { additions, removals, fee, conditions }`.

### B.5 SpendResult (dig-clvm/src/consensus/result.rs:8-21)

```rust
pub struct SpendResult {
    pub additions: Vec<Coin>,
    pub removals: Vec<Coin>,
    pub fee: u64,
    pub conditions: OwnedSpendBundleConditions,
}
```

**Mempool usage of each field:**
- `additions` -> stored as `MempoolItem.additions`, registered in `mempool_coins` for CPFP.
- `removals` -> coin IDs extracted (via `Coin::coin_id()`) and stored as `MempoolItem.removals`, registered in `coin_index`.
- `fee` -> stored as `MempoolItem.fee`, used for FPC calculation and RBF comparison. Note: this equals `conditions.removal_amount - conditions.addition_amount` (already computed by chia-consensus as u128, truncated to u64 by dig-clvm after conservation check).
- `conditions` -> stored as `MempoolItem.conditions`. The mempool reads:
  - `conditions.cost` -> admission cost (= `execution_cost + condition_cost`, both also available separately).
  - `conditions.reserve_fee` -> already the sum of all RESERVE_FEE conditions. No manual iteration needed.
  - `conditions.removal_amount`, `conditions.addition_amount` -> u128 totals for balance validation.
  - `conditions.validated_signature` -> whether BLS sig was already verified in this call.
  - `conditions.spends[*].create_coin` -> per-spend additions breakdown.
  - `conditions.spends[*].height_relative`, `seconds_relative`, etc. -> per-spend timelock resolution.
  - `conditions.spends[*].flags` -> `ELIGIBLE_FOR_DEDUP` (0x1), `ELIGIBLE_FOR_FF` (0x4). Set by chia-consensus MempoolVisitor. The mempool reads these, it does not compute them.
  - `conditions.height_absolute`, `seconds_absolute`, `before_height_absolute`, `before_seconds_absolute` -> bundle-level absolute timelocks.

**Additionally, `SpendBundle::name()`** (from chia-protocol) computes the canonical bundle hash used as `MempoolItem.spend_bundle_id`. The mempool calls this directly -- no custom serialization needed.

### B.6 OwnedSpendBundleConditions

This type comes from `chia_consensus::owned_conditions::OwnedSpendBundleConditions`. It is the primary output of CLVM execution. Key fields the mempool accesses:

```
.cost: u64                          // Total CLVM execution cost
.spends: Vec<OwnedSpendConditions>  // Per-spend condition data
  .coin_id: Bytes32
  .puzzle_hash: Bytes32
  .coin_amount: u64
  .flags: u32                       // ELIGIBLE_FOR_DEDUP | ELIGIBLE_FOR_FF
  .create_coin: Vec<(Bytes32, u64, Bytes)>  // (puzzle_hash, amount, memo)
  .agg_sig_me: Vec<(PublicKey, Bytes)>
  .height_relative: Option<u32>
  .height_absolute: u32
  .seconds_relative: Option<u64>
  .seconds_absolute: u64
  .before_height_relative: Option<u32>
  .before_height_absolute: Option<u32>
  .before_seconds_relative: Option<u64>
  .before_seconds_absolute: Option<u64>
.height_absolute: u32               // Bundle-level absolute height assertion
.seconds_absolute: u64              // Bundle-level absolute seconds assertion
.before_height_absolute: Option<u32>
.before_seconds_absolute: Option<u64>
.agg_sig_unsafe: Vec<(PublicKey, Bytes)>
```

### B.7 build_block_generator() (dig-clvm/src/consensus/block.rs:29-115) -- OUTSIDE MEMPOOL SCOPE

```rust
pub fn build_block_generator(
    bundles: &[SpendBundle],
    context: &ValidationContext,
    max_cost: Cost,
) -> Result<BlockGeneratorResult, ValidationError>
```

**Called by the block producer** (not the mempool) after `select_for_block()` returns the selected items. Documented here for context only -- this function is outside the mempool's boundary.

**Step-by-step:**
1. Iterates bundles in order (lines 43-85). For each:
   - Runs `run_spendbundle()` with `DONT_VALIDATE_SIGNATURE` to compute cost.
   - Skips bundles that fail or exceed remaining budget.
   - Collects `(coin, puzzle_reveal, solution)` tuples and signatures.
2. Builds compressed generator (line 92): `solution_generator_backrefs(spends_iter)` -- uses CLVM back-references to deduplicate repeated puzzle bytes.
3. Aggregates signatures (lines 96-104): Sums all `G2Element` signatures.
4. Returns `BlockGeneratorResult { generator, block_refs, aggregated_signature, additions, removals, cost, bundles_included }`.

**Interaction with mempool block selection:**
The mempool's `select_for_block()` returns items in topological order (parents before children) with FPC descending within layers. The caller passes these to `build_block_generator()` which processes them in that order, ensuring CPFP parent coins exist before children are validated.

### B.8 validate_block() (dig-clvm/src/consensus/block.rs:123-197) -- OUTSIDE MEMPOOL SCOPE

```rust
pub fn validate_block(
    generator: &[u8],
    generator_refs: &[Vec<u8>],
    context: &ValidationContext,
    config: &ValidationConfig,
    bls_cache: Option<&mut BlsCache>,
    aggregated_signature: &Signature,
) -> Result<SpendResult, ValidationError>
```

Validates a complete block generator. Calls `chia_consensus::run_block_generator::run_block_generator2()` (line 136). **Not called by the mempool.** Documented here for context -- used by the block validator, which is a separate component.

### B.9 ValidationError (dig-clvm/src/consensus/error.rs:1-36)

```rust
pub enum ValidationError {
    Clvm(String),
    CoinNotFound(Bytes32),
    AlreadySpent(Bytes32),
    DoubleSpend(Bytes32),
    PuzzleHashMismatch(Bytes32),
    SignatureFailed,
    ConservationViolation { input: u64, output: u64 },
    CostExceeded { limit: Cost, consumed: Cost },
    Driver(DriverError),
}
```

**Mempool error mapping:**
dig-mempool converts these to `MempoolError` variants:

| `dig_clvm::ValidationError` | `MempoolError` |
|---|---|
| `DoubleSpend(id)` | `DuplicateSpend(id)` |
| `CoinNotFound(id)` | `CoinNotFound(id)` (after CPFP resolution fails) |
| `AlreadySpent(id)` | `CoinAlreadySpent(id)` |
| `Clvm(msg)` | `ClvmError(msg)` |
| `SignatureFailed` | `InvalidSignature` |
| `ConservationViolation { .. }` | `ConservationViolation { .. }` |
| `CostExceeded { .. }` | `CostExceeded { .. }` |
| `PuzzleHashMismatch(id)` | `ValidationError(e.to_string())` |
| `Driver(e)` | `ValidationError(e.to_string())` |

All conversions use `e.to_string()` for the catch-all `MempoolError::ValidationError(String)` variant, preserving `Clone + PartialEq` on the error type.

### B.10 BlsCache Usage

The mempool maintains an internal `BlsCache` (from `chia_bls`) protected by a `Mutex`. This cache stores BLS pairing results to avoid recomputing expensive elliptic curve operations when the same public key / message pairs are encountered.

**Lifecycle:**
1. Created once during `Mempool::new()`.
2. Passed as `Some(&mut cache)` to `validate_spend_bundle()` during each `submit()` call.
3. `validate_spend_bundle()` uses Path B (lines 89-115): runs CLVM, then calls `cache.aggregate_verify()` which checks the cache before computing new pairings.
4. The cache grows over time. It is NOT included in `snapshot()` (ephemeral, rebuild on restore).
5. On `clear()`, the cache is optionally reset to free memory.

### B.11 dig-constants Usage

The mempool uses `dig-constants` for:

1. **Network selection**: `DIG_MAINNET` or `DIG_TESTNET` constant, passed to `Mempool::new()`.
2. **Consensus constants**: `constants.consensus()` returns `&ConsensusConstants` which is passed through `ValidationContext` to all `chia-consensus` functions.
3. **Cost limits**: `constants.max_block_cost_clvm()` returns `11,000,000,000` (used as `L1_MAX_COST_PER_SPEND` default for `config.max_bundle_cost`). The L2 block cost (`L2_MAX_COST_PER_BLOCK = 550B`) is defined in `dig-clvm/src/consensus/config.rs`.
4. **Genesis challenge**: `constants.genesis_challenge()` returns the network-specific genesis hash. Used implicitly by `chia-consensus` during AGG_SIG domain separation.
5. **AGG_SIG additional data**: Seven pre-computed hash constants ([dig-constants/src/lib.rs:112-130](https://github.com/DIG-Network/dig-constants)) derived as `sha256(genesis_challenge || opcode_byte)`. These are embedded in `ConsensusConstants` and used by `chia-consensus` during BLS signature verification. The mempool does not interact with these directly -- they flow through `ValidationContext.constants.consensus()`.

### B.12 Complete Mempool -> Chia Crate Call Graph

```
Mempool::submit()
  │
  ├─ Phase 1: Validation (lock-free, concurrent)
  │   ├─ SpendBundle::name()                         [chia-protocol] dedup check
  │   ├─ dig_clvm::validate_spend_bundle()           [dig-clvm] CLVM dry-run + BLS sig
  │   │    ├─ Coin::coin_id()                        [chia-protocol] compute coin hashes
  │   │    ├─ chia_consensus::run_spendbundle()       [chia-consensus] CLVM execution
  │   │    │    └─ MempoolVisitor                     [chia-consensus] sets ELIGIBLE_FOR_DEDUP/FF
  │   │    ├─ BlsCache::aggregate_verify()            [chia-bls] cached BLS sig verify
  │   │    ├─ Cost enforcement
  │   │    ├─ Extract additions (CREATE_COIN) and removals
  │   │    └─ Conservation check (removal_amount >= addition_amount)
  │   │
  │   ├─ Read OwnedSpendBundleConditions:             [chia-consensus]
  │   │    ├─ .cost, .reserve_fee, .removal_amount, .addition_amount
  │   │    ├─ .spends[*].flags (ELIGIBLE_FOR_DEDUP, ELIGIBLE_FOR_FF)
  │   │    └─ .spends[*].height_relative, .seconds_relative, etc.
  │   │
  │   └─ Compute virtual cost, resolve timelocks
  │
  └─ Phase 2: State Mutation (write lock, fast)
      ├─ Re-check dedup (race condition guard)
      ├─ CPFP dependency resolution (mempool_coins lookup)
      ├─ announcement_id()                            [chia-sdk-types] cross-bundle validation
      ├─ Conflict detection + RBF (coin_index)
      ├─ Capacity management / eviction
      └─ Insertion + event hooks

Mempool::select_for_block()
  └─ Returns Vec<Arc<MempoolItem>>     ◄── MEMPOOL OUTPUT BOUNDARY
      (ordered, non-conflicting, topologically sorted)

═══════════════════════════════════════ OUTSIDE MEMPOOL SCOPE ═══════

Caller (block producer) uses selected items:
  ├─ fast_forward_singleton(...)               [chia-consensus] rebase FF lineage proofs
  └─ dig_clvm::build_block_generator()         [dig-clvm] block construction
       ├─ chia_consensus::run_spendbundle()    (cost estimation, skip sig)
       ├─ chia_consensus::solution_generator_backrefs()  (CLVM compression)
       └─ Aggregate BLS signatures             [chia-bls]

Caller (block validator):
  └─ dig_clvm::validate_block()                [dig-clvm]
      └─ chia_consensus::run_block_generator2() [chia-consensus]
```
