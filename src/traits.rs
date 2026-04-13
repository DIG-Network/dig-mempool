//! Extension traits for mempool customization.
//!
//! # Overview
//!
//! These traits allow callers to inject domain-specific logic into the mempool
//! without modifying the core crate:
//!
//! - [`AdmissionPolicy`] — custom validation after standard checks pass
//! - [`BlockSelectionStrategy`] — custom block candidate selection algorithm
//! - [`MempoolEventHook`] — synchronous callbacks on mempool mutations
//! - [`RemovalReason`] — why an item was removed (used by event hooks)
//!
//! All traits are **object-safe** (usable as `dyn Trait`) and designed for
//! use with dynamic dispatch.
//!
//! # Chia L1 Correspondence
//!
//! These are dig-mempool extensions not present in Chia L1. Chia has hardcoded
//! admission rules and a single selection algorithm. dig-mempool provides
//! these hooks for DIG L2-specific requirements.
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 3.10](../docs/resources/SPEC.md) — Extension traits
//! - [API-007](../docs/requirements/domains/crate_api/specs/API-007.md) — Requirement
//! - [LCY-005](../docs/requirements/domains/lifecycle/specs/LCY-005.md) — Event hooks
//! - [LCY-006](../docs/requirements/domains/lifecycle/specs/LCY-006.md) — RemovalReason

use std::sync::Arc;

use dig_clvm::Bytes32;

use crate::item::MempoolItem;

/// Optional admission policy applied after all standard validation passes.
///
/// Called during Phase 2 of the admission pipeline (SPEC Section 5.16),
/// after dedup, CLVM validation, conflict detection, RBF, and capacity checks.
///
/// # Contract
///
/// - `item`: the candidate `MempoolItem` about to be admitted (fully validated).
/// - `existing_items`: snapshot of all current active items (for cross-item decisions).
/// - Return `Ok(())` to allow admission.
/// - Return `Err(reason)` to reject with `MempoolError::PolicyRejected(reason)`.
/// - The policy MUST NOT mutate the mempool. It is a pure decision function.
/// - The policy is called under the write lock — keep it fast and non-blocking.
///
/// # Object Safety
///
/// This trait is object-safe: `submit_with_policy()` accepts `&dyn AdmissionPolicy`.
///
/// # Use Cases
///
/// - Reject bundles creating too many coins
/// - Blocklist specific puzzle hashes
/// - Rate-limit by puzzle type
/// - Enforce DIG-specific registry rules
pub trait AdmissionPolicy {
    /// Inspect a fully validated item before admission.
    fn check(&self, item: &MempoolItem, existing_items: &[Arc<MempoolItem>]) -> Result<(), String>;
}

/// Custom block selection strategy, replacing the built-in 4-way greedy.
///
/// Called by `select_for_block_with_strategy()` instead of the default algorithm.
///
/// # Contract
///
/// The implementation MUST:
/// - Not exceed `max_block_cost` total cost
/// - Not exceed `max_spends` total spend count
/// - Not include conflicting items (items sharing a spent coin)
/// - Include all CPFP ancestors of any selected item
/// - Return items in topological order (parents before children)
///
/// The implementation SHOULD optimize for total fees.
///
/// # Object Safety
///
/// This trait is object-safe: usable as `&dyn BlockSelectionStrategy`.
///
/// # Note
///
/// The mempool does NOT validate the strategy's output. Invalid output
/// (e.g., conflicting items, missing ancestors) may cause downstream
/// block validation failures. The strategy is trusted.
///
/// # Use Cases
///
/// - Prioritize governance transactions regardless of fee rate
/// - MEV-resistant ordering
/// - Custom knapsack packing algorithms
pub trait BlockSelectionStrategy {
    /// Select items for block inclusion from the eligible candidates.
    fn select(
        &self,
        eligible_items: &[Arc<MempoolItem>],
        max_block_cost: u64,
        max_spends: usize,
    ) -> Vec<Arc<MempoolItem>>;
}

/// Synchronous event hook for mempool mutations.
///
/// Called under the write lock during state changes. All methods have
/// default no-op implementations, so implementors only override the
/// events they care about.
///
/// # Warning
///
/// Implementations MUST be fast and non-blocking. The write lock is held
/// during these callbacks. Blocking operations will stall all mempool
/// access (submissions, queries, block selection).
///
/// # Use Cases
///
/// - Logging item additions/removals
/// - Metrics collection (counters, histograms)
/// - External notification systems
/// - Audit trail recording
pub trait MempoolEventHook: Send + Sync {
    /// Called when a new item is admitted to the active pool.
    fn on_item_added(&self, _item: &MempoolItem) {}

    /// Called when an item is removed from the active pool.
    /// The `reason` indicates why (confirmed, evicted, RBF, cascade, etc.).
    fn on_item_removed(&self, _bundle_id: &Bytes32, _reason: RemovalReason) {}

    /// Called after block candidate selection completes.
    fn on_block_selected(&self, _items: &[Arc<MempoolItem>]) {}

    /// Called when a bundle is added to the conflict cache (failed RBF).
    fn on_conflict_cached(&self, _bundle_id: &Bytes32) {}

    /// Called when a timelocked item is added to the pending pool.
    fn on_pending_added(&self, _item: &MempoolItem) {}
}

/// Reason an item was removed from the mempool.
///
/// Used by [`MempoolEventHook::on_item_removed()`] and included in
/// [`RetryBundles::cascade_evicted`] reporting.
///
/// # Variants (7 total)
///
/// Each variant maps to a specific removal trigger in the lifecycle:
///
/// | Variant | Trigger | Recoverable? |
/// |---------|---------|-------------|
/// | `Confirmed` | Block confirmed spending the item's coins | No (on-chain) |
/// | `ReplacedByFee` | Higher-fee bundle replaced via RBF | No (superseded) |
/// | `CascadeEvicted` | Parent removed; this item's coins gone | No (orphaned) |
/// | `Expired` | `assert_before_height` or `assert_before_seconds` passed | No (expired) |
/// | `CapacityEviction` | Low-fee eviction to make room | Retry possible |
/// | `ExplicitRemoval` | Caller called `remove()` | Caller-decided |
/// | `Cleared` | Mempool cleared for reorg recovery | Retry after reorg |
///
/// See: [LCY-006](../docs/requirements/domains/lifecycle/specs/LCY-006.md)
///
/// Chia equivalent: `MempoolRemoveReason` at
/// [mempool.py:87-91](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L87)
/// (only 4 variants: CONFLICT, BLOCK_INCLUSION, POOL_FULL, EXPIRED).
/// dig-mempool extends this with ReplacedByFee, CascadeEvicted, ExplicitRemoval, Cleared.
#[derive(Debug, Clone, PartialEq)]
pub enum RemovalReason {
    /// Item's coins were spent in a confirmed block.
    Confirmed,
    /// A higher-fee bundle replaced this item via RBF.
    ReplacedByFee {
        /// The bundle ID of the replacement.
        replacement_id: Bytes32,
    },
    /// Parent item was removed; this item's input coins no longer exist.
    CascadeEvicted {
        /// The bundle ID of the removed parent.
        parent_id: Bytes32,
    },
    /// Item expired (`assert_before_height` or `assert_before_seconds` passed).
    Expired,
    /// Item was evicted due to mempool capacity pressure (low fee rate).
    CapacityEviction,
    /// Caller explicitly removed via `Mempool::remove()`.
    ExplicitRemoval,
    /// Mempool was cleared for reorg recovery via `Mempool::clear()`.
    Cleared,
}
