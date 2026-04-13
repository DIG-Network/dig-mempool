//! Mempool error types.
//!
//! # Overview
//!
//! `MempoolError` is the unified error enum for all mempool operations.
//! It derives `Clone + PartialEq` for ergonomic test assertions:
//!
//! ```rust,ignore
//! assert_eq!(result, Err(MempoolError::FeeTooLow));
//! ```
//!
//! # Design Decision: ValidationError as String
//!
//! The `ValidationError` variant stores `dig-clvm::ValidationError` as a
//! `String` rather than the original type. This is because `ValidationError`
//! from dig-clvm does not implement `Clone + PartialEq`, and we need both
//! derives on `MempoolError` for testability (SPEC Section 1.3, Decision #14).
//!
//! The conversion happens via `e.to_string()` at the mempool boundary,
//! preserving the error message while satisfying trait requirements.
//!
//! # Chia L1 Correspondence
//!
//! Chia uses `chia.util.errors.Err` enum codes (e.g., `DOUBLE_SPEND = 5`,
//! `MEMPOOL_CONFLICT = 19`, `INVALID_FEE_LOW_FEE = 18`). dig-mempool uses
//! named variants with structured data for better error context.
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 4](docs/resources/SPEC.md) — Error Types
//! - [API-004](docs/requirements/domains/crate_api/specs/API-004.md) — Requirement

use dig_clvm::Bytes32;

/// Errors returned by mempool operations.
///
/// Organized by category: dedup, structural, CLVM, cost/fee, conflict/RBF,
/// capacity, timelocks, conservation, CPFP, spend count, policy, and upstream.
///
/// Derives `Clone + PartialEq` for testability. `thiserror` provides `Display`
/// and `Error` trait implementations.
///
/// # Variant Count: 24
///
/// Each variant corresponds to a specific failure mode documented in the
/// SPEC's admission pipeline (Section 5).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MempoolError {
    // ── Deduplication ──
    // See: [ADM-003](docs/requirements/domains/admission/specs/ADM-003.md)
    /// Bundle ID was recently seen (in seen-cache, active pool, pending pool,
    /// or conflict cache). The seen-cache is populated before CLVM validation
    /// to prevent DoS via repeated submission of expensive-to-validate bundles.
    #[error("bundle already seen: {0}")]
    AlreadySeen(Bytes32),

    // ── Structural Errors ──
    // These are caught by dig-clvm during validation, but the mempool
    // surfaces them as its own error types for a consistent API.
    /// The bundle spends the same coin more than once.
    /// Caught by dig-clvm validate.rs:39-45 (HashSet dedup check).
    #[error("duplicate spend of coin {0} within bundle")]
    DuplicateSpend(Bytes32),

    /// A coin referenced by the bundle was not found in the caller's
    /// `coin_records` AND was not found in `mempool_coins` (CPFP lookup).
    /// Caught by dig-clvm validate.rs:56-59 (coin existence check).
    #[error("coin not found: {0}")]
    CoinNotFound(Bytes32),

    /// A coin referenced by the bundle is already spent on-chain.
    /// Caught by dig-clvm validate.rs:52-53 (`record.spent` check).
    #[error("coin already spent: {0}")]
    CoinAlreadySpent(Bytes32),

    // ── CLVM / Signature Errors ──
    // Propagated from dig-clvm::validate_spend_bundle().
    // See: [ADM-002](docs/requirements/domains/admission/specs/ADM-002.md)
    /// CLVM execution failed (puzzle error, invalid solution, etc.).
    /// Converted from `dig_clvm::ValidationError::Clvm` via `.to_string()`.
    #[error("CLVM execution error: {0}")]
    ClvmError(String),

    /// BLS aggregate signature verification failed.
    /// The bundle is correctly formed but not properly signed.
    #[error("invalid aggregate signature")]
    InvalidSignature,

    // ── Cost / Fee Errors ──
    // See: [ADM-004](docs/requirements/domains/admission/specs/ADM-004.md),
    //      [ADM-005](docs/requirements/domains/admission/specs/ADM-005.md)
    /// Bundle CLVM cost exceeds `config.max_bundle_cost`.
    /// Chia equivalent: `BLOCK_COST_EXCEEDS_MAX` at
    /// [mempool_manager.py:733](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L733).
    #[error("cost {cost} exceeds maximum {max}")]
    CostExceeded { cost: u64, max: u64 },

    /// Outputs exceed inputs (negative fee). This should be caught by
    /// dig-clvm's conservation check, but is included for completeness.
    #[error("negative fee: outputs ({output}) exceed inputs ({input})")]
    NegativeFee { input: u64, output: u64 },

    /// The bundle's implicit fee is less than the sum of its `RESERVE_FEE`
    /// conditions. The fee from `SpendResult.fee` is checked against
    /// `conditions.reserve_fee` (pre-summed by chia-consensus).
    #[error("insufficient fee: required {required}, available {available}")]
    InsufficientFee { required: u64, available: u64 },

    /// Fee-per-virtual-cost is too low for the current mempool utilization.
    /// Returned when utilization >= 80% and the bundle doesn't meet the
    /// minimum fee threshold from `estimate_min_fee()`.
    /// See: [FEE-001](docs/requirements/domains/fee_estimation/specs/FEE-001.md)
    #[error("fee too low for current mempool utilization")]
    FeeTooLow,

    // ── Conflict / RBF Errors ──
    // See: [CFR-001](docs/requirements/domains/conflict_resolution/specs/CFR-001.md)
    //      through [CFR-005](docs/requirements/domains/conflict_resolution/specs/CFR-005.md)
    /// A coin spent by this bundle is already spent by an active mempool item,
    /// and the RBF conditions were not met. The bundle has been added to the
    /// conflict cache for retry after the conflicting item is confirmed.
    /// Chia: `MEMPOOL_CONFLICT` at [mempool_manager.py:816](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L816).
    #[error("conflicts with existing mempool item {0}")]
    Conflict(Bytes32),

    /// RBF attempted but new bundle does not spend a superset of all
    /// conflicting bundles' coins. Chia: `can_replace()` superset rule at
    /// [mempool_manager.py:1101](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L1101).
    #[error("RBF rejected: must spend superset of conflicting bundle's coins")]
    RbfNotSuperset,

    /// RBF attempted but new bundle's fee-per-virtual-cost is not strictly
    /// higher than the conflicting bundles' aggregate fee-per-cost.
    /// Chia: [mempool_manager.py:1119](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L1119).
    #[error("RBF rejected: fee-per-cost not higher than existing")]
    RbfFpcNotHigher,

    /// RBF attempted but absolute fee increase is below `config.min_rbf_fee_bump`.
    /// Default minimum: 10,000,000 mojos (Chia: `MEMPOOL_MIN_FEE_INCREASE`).
    #[error("RBF rejected: fee bump {provided} below minimum {required}")]
    RbfBumpTooLow { required: u64, provided: u64 },

    // ── Capacity Errors ──
    // See: [POL-002](docs/requirements/domains/pools/specs/POL-002.md)
    /// Active mempool is at capacity (`total_cost + bundle_cost > max_total_cost`)
    /// and this bundle cannot evict enough low-FPC items to make room.
    #[error("mempool full: cannot admit bundle")]
    MempoolFull,

    /// Pending pool is at capacity (either count or cost limit reached).
    /// See: [POL-004](docs/requirements/domains/pools/specs/POL-004.md)
    #[error("pending pool full")]
    PendingPoolFull,

    // ── Timelock Errors ──
    // See: [ADM-006](docs/requirements/domains/admission/specs/ADM-006.md)
    /// Bundle has contradictory timelocks: `assert_before_height <= assert_height`
    /// or `assert_before_seconds <= assert_seconds`. The bundle can never be valid.
    /// Chia: `IMPOSSIBLE_HEIGHT_ABSOLUTE_CONSTRAINTS` at
    /// [mempool_manager.py:791](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L791).
    #[error("impossible timelock constraints")]
    ImpossibleTimelocks,

    /// Bundle has already expired: `assert_before_height <= current_height`
    /// or `assert_before_seconds <= current_timestamp`.
    #[error("bundle has expired")]
    Expired,

    // ── Conservation ──
    /// Conservation violation caught independently of dig-clvm.
    /// Should be unreachable if dig-clvm validation is functioning correctly.
    #[error("conservation violation: input {input}, output {output}")]
    ConservationViolation { input: u64, output: u64 },

    // ── CPFP Dependency Errors ──
    // See: [CPF-003](docs/requirements/domains/cpfp/specs/CPF-003.md),
    //      [CPF-004](docs/requirements/domains/cpfp/specs/CPF-004.md)
    /// Bundle would exceed `config.max_dependency_depth` (default 25).
    /// A bundle spending a mempool-created coin has depth 1; each additional
    /// hop adds 1 to the depth.
    #[error("dependency depth {depth} exceeds maximum {max}")]
    DependencyTooDeep { depth: u32, max: u32 },

    /// A cycle was detected in the CPFP dependency graph.
    /// This should be structurally impossible in a UTXO model (a coin is
    /// created once and spent once) but is checked defensively.
    #[error("dependency cycle detected")]
    DependencyCycle,

    // ── Spend Count ──
    /// Total spends in the active pool would exceed `config.max_spends_per_block`
    /// (default 6,000). This is a soft check; block selection enforces the hard limit.
    #[error("spend count {count} would exceed block limit {max}")]
    TooManySpends { count: usize, max: usize },

    // ── Admission Policy ──
    // See: [API-007](docs/requirements/domains/crate_api/specs/API-007.md)
    /// Rejected by a caller-provided `AdmissionPolicy` implementation.
    /// The string contains the policy's rejection reason.
    #[error("admission policy rejected: {0}")]
    PolicyRejected(String),

    // ── Upstream Validation ──
    // Catch-all for dig-clvm errors that don't map to a specific variant above.
    /// Error propagated from `dig_clvm::validate_spend_bundle()`.
    /// Stored as `String` to satisfy `Clone + PartialEq` (Decision #14).
    /// The original `ValidationError` is converted via `.to_string()`.
    ///
    /// # Conversion
    ///
    /// Use the `From<dig_clvm::ValidationError>` impl (below) to convert:
    /// ```rust,ignore
    /// let mempool_err: MempoolError = clvm_err.into();
    /// ```
    #[error("validation error: {0}")]
    ValidationError(String),
}

/// Convert a `dig_clvm::ValidationError` into a `MempoolError`.
///
/// The upstream `ValidationError` is converted to a `String` via `.to_string()`
/// because `dig_clvm::ValidationError` does not derive `Clone + PartialEq`,
/// which are required by `MempoolError` (Design Decision #14).
///
/// This conversion is used in the admission pipeline when `dig_clvm::validate_spend_bundle()`
/// returns an error. The `?` operator automatically applies this conversion:
///
/// ```rust,ignore
/// let result = dig_clvm::validate_spend_bundle(&bundle, &ctx, &cfg, cache)?;
/// // If validate_spend_bundle returns Err(ValidationError), it becomes
/// // MempoolError::ValidationError(err.to_string())
/// ```
///
/// See: [ADM-002](docs/requirements/domains/admission/specs/ADM-002.md)
impl From<dig_clvm::ValidationError> for MempoolError {
    fn from(e: dig_clvm::ValidationError) -> Self {
        MempoolError::ValidationError(e.to_string())
    }
}
