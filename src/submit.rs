//! Submission result types and lifecycle return types.
//!
//! # Overview
//!
//! `SubmitResult` is the success return type of `Mempool::submit()` and related
//! methods. It distinguishes between two successful outcomes:
//! - `Success`: admitted to the active pool, eligible for block selection
//! - `Pending`: valid but timelocked, stored in the pending pool
//!
//! `RetryBundles` is the return type of `Mempool::on_new_block()`. It provides
//! the caller with bundles to resubmit after a block confirmation.
//!
//! `ConfirmedBundleInfo` carries per-bundle metrics about confirmed transactions
//! for the fee estimator.
//!
//! Validation failures are returned as `Err(MempoolError)`, not as `SubmitResult`
//! variants. This keeps the success and failure paths cleanly separated.
//!
//! # Chia L1 Correspondence
//!
//! Chia returns `MempoolInclusionStatus.SUCCESS` or `MempoolInclusionStatus.PENDING`
//! alongside an error code. Timelocked bundles are routed to `PendingTxCache` at
//! [mempool_manager.py:600-603].
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 3.2](../docs/resources/SPEC.md) — SubmitResult definition
//! - [SPEC.md Section 3.5](../docs/resources/SPEC.md) — RetryBundles definition
//! - [API-005](../docs/requirements/domains/crate_api/specs/API-005.md) — Requirement
//! - [LCY-002](../docs/requirements/domains/lifecycle/specs/LCY-002.md) — RetryBundles spec
//!
//! [mempool_manager.py:600-603]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L600

use dig_clvm::{Bytes32, SpendBundle};

/// Bundles returned by `on_new_block()` that the caller should resubmit.
///
/// `on_new_block()` removes confirmed and expired items from internal pools
/// and returns raw `SpendBundle`s for the caller to re-validate and resubmit
/// with fresh `CoinRecord`s. This preserves the "no I/O" principle — the
/// mempool does not look up current coin state itself.
///
/// # Field Semantics
///
/// | Field | Source | Action Required |
/// |-------|--------|-----------------|
/// | `conflict_retries` | Conflict cache | Resubmit via `submit()` with current coin records |
/// | `pending_promotions` | Pending pool | Resubmit via `submit()` with current coin records |
/// | `cascade_evicted` | Active pool dependents | Informational — caller may log or notify |
///
/// # Guarantees
///
/// - All items in `conflict_retries` have been removed from the conflict cache.
/// - All items in `pending_promotions` have been removed from the pending pool.
/// - All IDs in `cascade_evicted` have been removed from the active pool.
/// - The struct is always returned even if all fields are empty.
///
/// See: [LCY-002](docs/requirements/domains/lifecycle/specs/LCY-002.md)
///
/// # Chia L1 Equivalent
///
/// Chia re-validates drained pending and conflict items inline under the lock
/// ([`mempool_manager.py:900-918`](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L900)).
/// dig-mempool returns them for the caller to resubmit with fresh coin records.
pub struct RetryBundles {
    /// Bundles from the conflict cache whose conflicting item was removed.
    ///
    /// These lost a previous RBF check. Now that the conflicting item has been
    /// confirmed or evicted, the caller should resubmit them.
    pub conflict_retries: Vec<SpendBundle>,

    /// Bundles from the pending pool whose timelocks are now satisfied.
    ///
    /// These were timelocked and could not be included in earlier blocks.
    /// Resubmit them now that `height >= assert_height`.
    pub pending_promotions: Vec<SpendBundle>,

    /// Bundle IDs cascade-evicted because their parent was confirmed or expired.
    ///
    /// These spent coins created by an item that is no longer in the mempool.
    /// They cannot be retried — their input coins no longer exist. Provided
    /// for caller bookkeeping and user notification.
    pub cascade_evicted: Vec<Bytes32>,
}

/// Per-bundle metrics for confirmed transactions, used by the fee estimator.
///
/// Passed to `on_new_block()` so the `FeeTracker` can update its rolling
/// window with real confirmed data. See [FEE-004].
///
/// [FEE-004]: docs/requirements/domains/fee_estimation/specs/FEE-004.md
pub struct ConfirmedBundleInfo {
    /// CLVM execution cost of the confirmed bundle.
    pub cost: u64,
    /// Fee paid by the confirmed bundle (mojos).
    pub fee: u64,
    /// Number of coin spends in the confirmed bundle.
    pub num_spends: usize,
}

/// Result of a successful `submit()` call.
///
/// Both variants represent valid bundles that passed CLVM validation and
/// admission checks. The distinction is whether the bundle is immediately
/// eligible for block selection (`Success`) or must wait for a future
/// block height (`Pending`).
///
/// # Usage
///
/// ```rust
/// use dig_mempool::SubmitResult;
///
/// fn handle(result: SubmitResult) {
///     match result {
///         SubmitResult::Success => println!("In active pool"),
///         SubmitResult::Pending { assert_height } => {
///             println!("Timelocked until height {assert_height}");
///         }
///     }
/// }
/// ```
///
/// # Return Context
///
/// Returned inside `Result<SubmitResult, MempoolError>`:
/// - `Ok(Success)` — bundle is in the active pool
/// - `Ok(Pending { assert_height })` — bundle is in the pending pool
/// - `Err(MempoolError::*)` — bundle was rejected
///
/// See: [`crate::MempoolError`] for the failure path.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmitResult {
    /// Bundle was admitted to the **active** mempool.
    ///
    /// The item is immediately eligible for block candidate selection
    /// via `select_for_block()`. It has no unsatisfied timelocks.
    Success,

    /// Bundle is valid but **timelocked**; stored in the pending pool.
    ///
    /// The bundle passed all validation (CLVM, signature, fee, cost) but
    /// has an `ASSERT_HEIGHT_ABSOLUTE` or `ASSERT_HEIGHT_RELATIVE` condition
    /// that is not yet satisfiable at the current chain height.
    ///
    /// When the chain reaches `assert_height`, the bundle will be returned
    /// in `RetryBundles::pending_promotions` from `on_new_block()`, and the
    /// caller should resubmit it with fresh coin records.
    ///
    /// `assert_height` is the **resolved absolute height** — relative timelocks
    /// have been converted to absolute values using the coin's confirmation
    /// height during admission (see [ADM-006]).
    ///
    /// [ADM-006]: ../docs/requirements/domains/admission/specs/ADM-006.md
    Pending {
        /// The minimum block height at which this bundle becomes eligible.
        /// Resolved from per-spend relative and absolute height timelocks.
        assert_height: u64,
    },
}
