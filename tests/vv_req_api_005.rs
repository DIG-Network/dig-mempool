//! REQUIREMENT: API-005 — SubmitResult Enum
//!
//! Test-driven verification that `SubmitResult` has `Success` and `Pending`
//! variants, is publicly exported, and supports pattern matching.
//!
//! ## What this proves
//!
//! - `Success` variant exists with no associated data
//! - `Pending` variant exists with `assert_height: u64`
//! - Both variants are pattern-matchable
//! - Derives Debug, Clone, PartialEq for testability
//! - Works inside `Result<SubmitResult, MempoolError>`
//!
//! ## Chia L1 Correspondence
//!
//! Chia routes timelocked bundles to `PendingTxCache` at:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L600
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-005.md

use dig_mempool::{MempoolError, SubmitResult};

/// Test: Success variant exists with no data.
///
/// Proves API-005 acceptance criterion: "Contains Success variant with no data."
/// Success means the bundle was admitted to the active pool.
#[test]
fn vv_req_api_005_success_variant() {
    let result = SubmitResult::Success;
    assert_eq!(result, SubmitResult::Success);
}

/// Test: Pending variant exists with assert_height field.
///
/// Proves API-005 acceptance criterion: "Contains Pending variant with
/// assert_height: u64." Pending means the bundle is valid but timelocked.
/// The assert_height is the resolved absolute height for promotion.
#[test]
fn vv_req_api_005_pending_variant() {
    let result = SubmitResult::Pending {
        assert_height: 1000,
    };
    match result {
        SubmitResult::Pending { assert_height } => {
            assert_eq!(assert_height, 1000);
        }
        _ => panic!("Expected Pending variant"),
    }
}

/// Test: Pattern matching works on both variants.
///
/// Proves both arms are reachable, which is essential for callers
/// to handle the two successful outcomes differently.
#[test]
fn vv_req_api_005_pattern_matching() {
    fn describe(r: &SubmitResult) -> &str {
        match r {
            SubmitResult::Success => "active",
            SubmitResult::Pending { .. } => "pending",
        }
    }
    assert_eq!(describe(&SubmitResult::Success), "active");
    assert_eq!(
        describe(&SubmitResult::Pending { assert_height: 500 }),
        "pending"
    );
}

/// Test: SubmitResult works inside Result<SubmitResult, MempoolError>.
///
/// Proves API-005 spec's "Return Context" — the full return type is
/// `Result<SubmitResult, MempoolError>`, where Ok is success/pending
/// and Err is rejection.
#[test]
fn vv_req_api_005_inside_result() {
    let ok_active: Result<SubmitResult, MempoolError> = Ok(SubmitResult::Success);
    let ok_pending: Result<SubmitResult, MempoolError> =
        Ok(SubmitResult::Pending { assert_height: 42 });
    let err: Result<SubmitResult, MempoolError> = Err(MempoolError::FeeTooLow);

    assert!(ok_active.is_ok());
    assert!(ok_pending.is_ok());
    assert!(err.is_err());
}

/// Test: SubmitResult is Clone.
///
/// Clone is needed for batch operations where results may be copied.
#[test]
fn vv_req_api_005_clone() {
    let original = SubmitResult::Pending { assert_height: 999 };
    let cloned = original.clone();
    assert_eq!(original, cloned);
}

/// Test: SubmitResult is Debug.
///
/// Debug formatting is needed for logging and error messages.
#[test]
fn vv_req_api_005_debug() {
    let s = format!("{:?}", SubmitResult::Success);
    assert!(s.contains("Success"));

    let p = format!("{:?}", SubmitResult::Pending { assert_height: 123 });
    assert!(p.contains("Pending"));
    assert!(p.contains("123"));
}
