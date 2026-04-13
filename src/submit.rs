//! Submission result types.
//!
//! # Overview
//!
//! `SubmitResult` is the success return type of `Mempool::submit()` and related
//! methods. It distinguishes between two successful outcomes:
//! - `Success`: admitted to the active pool, eligible for block selection
//! - `Pending`: valid but timelocked, stored in the pending pool
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
//! - [API-005](../docs/requirements/domains/crate_api/specs/API-005.md) — Requirement
//!
//! [mempool_manager.py:600-603]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L600

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
