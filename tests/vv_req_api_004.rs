//! REQUIREMENT: API-004 — MempoolError Enum
//!
//! Test-driven verification that `MempoolError` has all 24 specified variants,
//! derives `Clone + PartialEq`, provides readable `Display` formatting via
//! `thiserror`, and supports conversion from `dig_clvm::ValidationError`.
//!
//! ## What this proves
//!
//! These tests verify every acceptance criterion from the API-004 spec:
//! - All 24 variants are present and constructible
//! - `Clone` works on all variants (cloned == original)
//! - `PartialEq` works for both equality and inequality comparisons
//! - `Display` (via thiserror) produces readable error messages
//! - `From<dig_clvm::ValidationError>` converts to `MempoolError::ValidationError(String)`
//! - Structured variants contain the correct associated data types
//!
//! ## Design Decision (SPEC Section 1.3, Decision #14)
//!
//! `MempoolError` derives `Clone + PartialEq` for test ergonomics:
//! ```rust,ignore
//! assert_eq!(result, Err(MempoolError::FeeTooLow));
//! ```
//! The `ValidationError` variant stores `String` instead of the original
//! `dig_clvm::ValidationError` because the upstream type doesn't derive
//! `Clone + PartialEq`. The conversion happens via `.to_string()`.
//!
//! ## Chia L1 Correspondence
//!
//! Chia uses integer error codes (`Err` enum in `chia.util.errors`):
//! - `DOUBLE_SPEND = 5` → `MempoolError::DuplicateSpend`
//! - `UNKNOWN_UNSPENT = 6` → `MempoolError::CoinNotFound`
//! - `MEMPOOL_CONFLICT = 19` → `MempoolError::Conflict`
//! - `INVALID_FEE_LOW_FEE = 18` → `MempoolError::FeeTooLow`
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-004.md

use dig_mempool::{Bytes32, MempoolError};

/// Test: All 24 variants are constructible.
///
/// Proves API-004 acceptance criterion: "All 24 variants are present as specified."
/// Each variant is constructed with appropriate data to verify it compiles.
/// If any variant is missing or has wrong types, this test won't compile.
#[test]
fn vv_req_api_004_all_variants_constructible() {
    let id = Bytes32::default();

    // Dedup (1)
    let _ = MempoolError::AlreadySeen(id);

    // Structural (3)
    let _ = MempoolError::DuplicateSpend(id);
    let _ = MempoolError::CoinNotFound(id);
    let _ = MempoolError::CoinAlreadySpent(id);

    // CLVM / signature (2)
    let _ = MempoolError::ClvmError("test".into());
    let _ = MempoolError::InvalidSignature;

    // Cost / fee (4)
    let _ = MempoolError::CostExceeded { cost: 1, max: 2 };
    let _ = MempoolError::NegativeFee {
        input: 1,
        output: 2,
    };
    let _ = MempoolError::InsufficientFee {
        required: 1,
        available: 0,
    };
    let _ = MempoolError::FeeTooLow;

    // Conflict / RBF (4)
    let _ = MempoolError::Conflict(id);
    let _ = MempoolError::RbfNotSuperset;
    let _ = MempoolError::RbfFpcNotHigher;
    let _ = MempoolError::RbfBumpTooLow {
        required: 10,
        provided: 5,
    };

    // Capacity (2)
    let _ = MempoolError::MempoolFull;
    let _ = MempoolError::PendingPoolFull;

    // Timelocks (2)
    let _ = MempoolError::ImpossibleTimelocks;
    let _ = MempoolError::Expired;

    // Conservation (1)
    let _ = MempoolError::ConservationViolation {
        input: 100,
        output: 200,
    };

    // CPFP (2)
    let _ = MempoolError::DependencyTooDeep { depth: 30, max: 25 };
    let _ = MempoolError::DependencyCycle;

    // Spend count (1)
    let _ = MempoolError::TooManySpends {
        count: 7000,
        max: 6000,
    };

    // Policy (1)
    let _ = MempoolError::PolicyRejected("reason".into());

    // Upstream (1)
    let _ = MempoolError::ValidationError("upstream error".into());

    // Total: 1+3+2+4+4+2+2+1+2+1+1+1 = 24 variants
}

/// Test: Clone works on all variants (cloned value equals original).
///
/// Proves API-004 acceptance criterion: "Derives Clone."
/// Clone is needed for returning errors from batch operations
/// where the same error may need to be reported multiple times.
#[test]
fn vv_req_api_004_clone_works() {
    let errors = make_all_variants();
    for err in &errors {
        let cloned = err.clone();
        // Clone must produce an equal value (requires PartialEq)
        assert_eq!(&cloned, err, "Clone should produce equal value for {err}");
    }
}

/// Test: PartialEq correctly distinguishes between variants.
///
/// Proves API-004 acceptance criterion: "Derives PartialEq."
/// Tests both equality (same variant, same data) and inequality
/// (different variant or different data).
#[test]
fn vv_req_api_004_partial_eq_works() {
    // Same variant, same data → equal
    assert_eq!(MempoolError::FeeTooLow, MempoolError::FeeTooLow);
    assert_eq!(
        MempoolError::CostExceeded { cost: 1, max: 2 },
        MempoolError::CostExceeded { cost: 1, max: 2 }
    );

    // Same variant, different data → not equal
    assert_ne!(
        MempoolError::CostExceeded { cost: 1, max: 2 },
        MempoolError::CostExceeded { cost: 3, max: 4 }
    );

    // Different variants → not equal
    assert_ne!(MempoolError::FeeTooLow, MempoolError::MempoolFull);
    assert_ne!(
        MempoolError::AlreadySeen(Bytes32::default()),
        MempoolError::CoinNotFound(Bytes32::default())
    );
}

/// Test: Display formatting produces readable error messages.
///
/// Proves API-004 acceptance criterion: "Derives thiserror::Error for
/// Display and Error trait." Each variant's `#[error("...")]` attribute
/// produces a human-readable message when formatted with `{}`.
#[test]
fn vv_req_api_004_display_formatting() {
    // Parameterized messages include field values
    let err = MempoolError::CostExceeded {
        cost: 15_000_000_000,
        max: 11_000_000_000,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("15000000000"),
        "Display should include cost value"
    );
    assert!(
        msg.contains("11000000000"),
        "Display should include max value"
    );

    // Simple messages are readable
    assert_eq!(
        format!("{}", MempoolError::FeeTooLow),
        "fee too low for current mempool utilization"
    );
    assert_eq!(
        format!("{}", MempoolError::MempoolFull),
        "mempool full: cannot admit bundle"
    );
    assert_eq!(
        format!("{}", MempoolError::InvalidSignature),
        "invalid aggregate signature"
    );
}

/// Test: From<dig_clvm::ValidationError> conversion works.
///
/// Proves API-004 acceptance criterion: "From<dig_clvm::ValidationError>
/// conversion is implemented." This is how CLVM validation errors flow
/// through to the mempool's error type.
///
/// The upstream `ValidationError` is converted to a String via `.to_string()`
/// because `ValidationError` doesn't derive `Clone + PartialEq`.
#[test]
fn vv_req_api_004_from_validation_error() {
    // Create a dig-clvm ValidationError (CoinNotFound variant)
    let clvm_err = dig_clvm::ValidationError::CoinNotFound(Bytes32::default());
    let expected_msg = clvm_err.to_string();

    // Convert to MempoolError using From trait
    let mempool_err: MempoolError = clvm_err.into();

    // Should be the ValidationError variant with the stringified message
    match &mempool_err {
        MempoolError::ValidationError(msg) => {
            assert_eq!(msg, &expected_msg);
        }
        other => panic!("Expected ValidationError, got {other}"),
    }
}

/// Test: MempoolError implements std::error::Error trait.
///
/// Proves the thiserror derive provides the Error trait, which is required
/// for interop with the broader Rust error handling ecosystem (?, anyhow, etc.).
#[test]
fn vv_req_api_004_implements_error_trait() {
    fn assert_error<T: std::error::Error>() {}
    assert_error::<MempoolError>();
}

/// Test: Structured variants have correct associated data types.
///
/// Proves API-004 acceptance criterion: "Structured variants contain the
/// correct associated data types." Verifies u64, u32, usize, String,
/// and Bytes32 are used in the right places.
#[test]
fn vv_req_api_004_structured_variant_types() {
    // u64 fields
    let _ = MempoolError::CostExceeded {
        cost: u64::MAX,
        max: u64::MAX,
    };
    let _ = MempoolError::InsufficientFee {
        required: u64::MAX,
        available: u64::MAX,
    };
    let _ = MempoolError::RbfBumpTooLow {
        required: u64::MAX,
        provided: u64::MAX,
    };

    // u32 fields
    let _ = MempoolError::DependencyTooDeep {
        depth: u32::MAX,
        max: u32::MAX,
    };

    // usize fields
    let _ = MempoolError::TooManySpends {
        count: usize::MAX,
        max: usize::MAX,
    };

    // String fields
    let _ = MempoolError::ClvmError(String::new());
    let _ = MempoolError::PolicyRejected(String::new());
    let _ = MempoolError::ValidationError(String::new());

    // Bytes32 fields
    let _ = MempoolError::AlreadySeen(Bytes32::default());
    let _ = MempoolError::Conflict(Bytes32::default());
}

// ── Helper ──

/// Construct one instance of each variant for bulk testing.
fn make_all_variants() -> Vec<MempoolError> {
    let id = Bytes32::default();
    vec![
        MempoolError::AlreadySeen(id),
        MempoolError::DuplicateSpend(id),
        MempoolError::CoinNotFound(id),
        MempoolError::CoinAlreadySpent(id),
        MempoolError::ClvmError("test".into()),
        MempoolError::InvalidSignature,
        MempoolError::CostExceeded { cost: 1, max: 2 },
        MempoolError::NegativeFee {
            input: 1,
            output: 2,
        },
        MempoolError::InsufficientFee {
            required: 1,
            available: 0,
        },
        MempoolError::FeeTooLow,
        MempoolError::Conflict(id),
        MempoolError::RbfNotSuperset,
        MempoolError::RbfFpcNotHigher,
        MempoolError::RbfBumpTooLow {
            required: 10,
            provided: 5,
        },
        MempoolError::MempoolFull,
        MempoolError::PendingPoolFull,
        MempoolError::ImpossibleTimelocks,
        MempoolError::Expired,
        MempoolError::ConservationViolation {
            input: 100,
            output: 200,
        },
        MempoolError::DependencyTooDeep { depth: 30, max: 25 },
        MempoolError::DependencyCycle,
        MempoolError::TooManySpends {
            count: 7000,
            max: 6000,
        },
        MempoolError::PolicyRejected("reason".into()),
        MempoolError::ValidationError("upstream error".into()),
    ]
}
