//! REQUIREMENT: API-007 — Extension Traits
//!
//! Test-driven verification that `AdmissionPolicy`, `BlockSelectionStrategy`,
//! and `MempoolEventHook` traits are publicly exported, implementable,
//! and object-safe (usable as `dyn Trait`).
//!
//! ## What this proves
//!
//! - `AdmissionPolicy::check()` has the correct signature
//! - `BlockSelectionStrategy::select()` has the correct signature
//! - `MempoolEventHook` has default method implementations
//! - `RemovalReason` enum has all 7 variants
//! - All traits are object-safe (work with dynamic dispatch)
//! - Custom implementations compile and produce expected results
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-007.md

use std::sync::Arc;

use dig_mempool::item::MempoolItem;
use dig_mempool::traits::{
    AdmissionPolicy, BlockSelectionStrategy, MempoolEventHook, RemovalReason,
};
use dig_mempool::Bytes32;

// ── Test Implementations ──

/// A policy that always accepts. Used to prove the trait is implementable.
struct AcceptAllPolicy;

impl AdmissionPolicy for AcceptAllPolicy {
    fn check(
        &self,
        _item: &MempoolItem,
        _existing_items: &[Arc<MempoolItem>],
    ) -> Result<(), String> {
        Ok(())
    }
}

/// A policy that always rejects with a message.
struct RejectAllPolicy;

impl AdmissionPolicy for RejectAllPolicy {
    fn check(
        &self,
        _item: &MempoolItem,
        _existing_items: &[Arc<MempoolItem>],
    ) -> Result<(), String> {
        Err("rejected by test policy".into())
    }
}

/// A selection strategy that returns nothing (empty block).
struct EmptyStrategy;

impl BlockSelectionStrategy for EmptyStrategy {
    fn select(
        &self,
        _eligible: &[Arc<MempoolItem>],
        _max_cost: u64,
        _max_spends: usize,
    ) -> Vec<Arc<MempoolItem>> {
        vec![]
    }
}

/// A no-op event hook (uses default implementations).
struct NoOpHook;
impl MempoolEventHook for NoOpHook {}

/// Test: AdmissionPolicy is implementable and accept path works.
///
/// Proves API-007 acceptance criterion: "AdmissionPolicy trait is publicly
/// exported" and "AdmissionPolicy::check() has the specified signature."
#[test]
fn vv_req_api_007_admission_policy_accept() {
    let policy = AcceptAllPolicy;
    let item = MempoolItem::new_for_test(100, 1_000_000, 1);
    let existing: Vec<Arc<MempoolItem>> = vec![];
    let result = policy.check(&item, &existing);
    assert!(result.is_ok());
}

/// Test: AdmissionPolicy rejection returns Err(String).
///
/// Proves the rejection path produces a String message that will be
/// wrapped in MempoolError::PolicyRejected by the admission pipeline.
#[test]
fn vv_req_api_007_admission_policy_reject() {
    let policy = RejectAllPolicy;
    let item = MempoolItem::new_for_test(100, 1_000_000, 1);
    let existing: Vec<Arc<MempoolItem>> = vec![];
    let result = policy.check(&item, &existing);
    assert_eq!(result, Err("rejected by test policy".to_string()));
}

/// Test: AdmissionPolicy is object-safe (usable as dyn trait).
///
/// Proves it can be used with dynamic dispatch: `&dyn AdmissionPolicy`.
/// This is required for `submit_with_policy()` which accepts `&dyn AdmissionPolicy`.
#[test]
fn vv_req_api_007_admission_policy_object_safe() {
    let policy: &dyn AdmissionPolicy = &AcceptAllPolicy;
    let item = MempoolItem::new_for_test(50, 500_000, 1);
    let existing: Vec<Arc<MempoolItem>> = vec![];
    assert!(policy.check(&item, &existing).is_ok());
}

/// Test: BlockSelectionStrategy is implementable.
///
/// Proves API-007 acceptance criterion: "BlockSelectionStrategy trait is
/// publicly exported" and "select() has the specified signature."
#[test]
fn vv_req_api_007_selection_strategy_implementable() {
    let strategy = EmptyStrategy;
    let eligible: Vec<Arc<MempoolItem>> = vec![];
    let result = strategy.select(&eligible, 11_000_000_000, 6_000);
    assert!(result.is_empty());
}

/// Test: BlockSelectionStrategy is object-safe.
///
/// Proves it can be used as `&dyn BlockSelectionStrategy`.
#[test]
fn vv_req_api_007_selection_strategy_object_safe() {
    let strategy: &dyn BlockSelectionStrategy = &EmptyStrategy;
    let eligible: Vec<Arc<MempoolItem>> = vec![];
    let result = strategy.select(&eligible, 11_000_000_000, 6_000);
    assert!(result.is_empty());
}

/// Test: MempoolEventHook has default implementations.
///
/// Proves the hook trait can be implemented with no methods overridden.
/// All 5 callbacks have default (no-op) implementations.
#[test]
fn vv_req_api_007_event_hook_defaults() {
    let hook = NoOpHook;
    let item = MempoolItem::new_for_test(100, 1_000_000, 1);
    let id = Bytes32::default();

    // All methods callable with default no-op behavior
    hook.on_item_added(&item);
    hook.on_item_removed(&id, RemovalReason::Confirmed);
    hook.on_block_selected(&[]);
    hook.on_conflict_cached(&id);
    hook.on_pending_added(&item);
}

/// Test: RemovalReason enum has all 7 variants.
///
/// Proves LCY-006 requirement that RemovalReason covers all removal contexts.
/// Each variant is constructed to verify it exists.
#[test]
fn vv_req_api_007_removal_reason_variants() {
    let id = Bytes32::default();
    let reasons = vec![
        RemovalReason::Confirmed,
        RemovalReason::ReplacedByFee { replacement_id: id },
        RemovalReason::CascadeEvicted { parent_id: id },
        RemovalReason::Expired,
        RemovalReason::CapacityEviction,
        RemovalReason::ExplicitRemoval,
        RemovalReason::Cleared,
    ];
    assert_eq!(reasons.len(), 7);
}

/// Test: RemovalReason derives Debug + Clone + PartialEq.
///
/// These derives are needed for logging, event handling, and test assertions.
#[test]
fn vv_req_api_007_removal_reason_derives() {
    let reason = RemovalReason::Confirmed;
    let cloned = reason.clone();
    assert_eq!(reason, cloned);
    let _ = format!("{:?}", reason);
}
