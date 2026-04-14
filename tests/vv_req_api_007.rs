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

use std::collections::HashMap;
use std::sync::Arc;

use dig_clvm::{Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::item::MempoolItem;
use dig_mempool::traits::{
    AdmissionPolicy, BlockSelectionStrategy, MempoolEventHook, RemovalReason,
};
use dig_mempool::{Bytes32, Mempool, MempoolError};
use hex_literal::hex;

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

fn make_coin(parent: u8, amount: u64) -> Coin {
    Coin::new(Bytes32::from([parent; 32]), NIL_PUZZLE_HASH, amount)
}

fn coin_record_for(coin: Coin) -> CoinRecord {
    CoinRecord {
        coin,
        coinbase: false,
        confirmed_block_index: 1,
        spent: false,
        spent_block_index: 0,
        timestamp: 100,
    }
}

fn nil_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record_for(coin));
    (bundle, cr)
}

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
    let reasons = [
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

// ── Integration tests ──────────────────────────────────────────────────────

/// Test: submit_with_policy() blocks admission when policy rejects.
///
/// Proves API-007: "submit_with_policy() invokes AdmissionPolicy::check()"
/// and "PolicyRejected error is returned when policy rejects."
#[test]
fn vv_req_api_007_submit_with_policy_rejects() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);

    let policy = RejectAllPolicy;
    let result = mempool.submit_with_policy(bundle, &cr, 0, 0, &policy);

    assert!(
        matches!(result, Err(MempoolError::PolicyRejected(_))),
        "expected PolicyRejected, got {:?}",
        result
    );
    assert_eq!(mempool.len(), 0, "rejected item must not be in pool");
}

/// Test: submit_with_policy() admits when policy accepts.
///
/// Proves API-007: "Policy acceptance works — item is admitted normally."
#[test]
fn vv_req_api_007_submit_with_policy_accepts() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    let id = bundle.name();

    let policy = AcceptAllPolicy;
    let result = mempool.submit_with_policy(bundle, &cr, 0, 0, &policy);

    assert!(result.is_ok(), "accepting policy must allow admission");
    assert!(mempool.contains(&id), "admitted item must be in pool");
}

/// Test: Policy receives current active items as existing_items snapshot.
///
/// Proves API-007: "existing_items is a snapshot of all current active items."
/// We submit one item first, then submit with a policy that counts existing items.
#[test]
fn vv_req_api_007_policy_receives_existing_items() {
    use std::sync::Mutex;

    struct CountingPolicy(Mutex<usize>);
    impl AdmissionPolicy for CountingPolicy {
        fn check(&self, _item: &MempoolItem, existing: &[Arc<MempoolItem>]) -> Result<(), String> {
            *self.0.lock().unwrap() = existing.len();
            Ok(())
        }
    }

    let mempool = Mempool::new(DIG_TESTNET);

    // Pre-populate with one item.
    let coin_a = make_coin(0x01, 100);
    let (bundle_a, cr_a) = nil_bundle(coin_a);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let policy = CountingPolicy(Mutex::new(0));
    let coin_b = make_coin(0x02, 100);
    let (bundle_b, cr_b) = nil_bundle(coin_b);
    mempool
        .submit_with_policy(bundle_b, &cr_b, 0, 0, &policy)
        .unwrap();

    let seen = *policy.0.lock().unwrap();
    assert_eq!(
        seen, 1,
        "policy must see 1 existing item (the pre-populated one)"
    );
}

/// Test: select_for_block_with_strategy() uses the custom strategy.
///
/// Proves API-007: "Custom strategy output is returned directly."
/// The custom strategy returns an empty set; verify the mempool returns that.
#[test]
fn vv_req_api_007_select_with_strategy_used() {
    let mempool = Mempool::new(DIG_TESTNET);
    // Submit two items so the pool has candidates.
    for i in 0x01..=0x02u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }
    assert_eq!(mempool.len(), 2);

    // Strategy returns nothing regardless of input.
    let strategy = EmptyStrategy;
    let result = mempool.select_for_block_with_strategy(&strategy, u64::MAX, 0, 0);
    assert!(
        result.is_empty(),
        "custom strategy returning empty must produce empty selection"
    );
}

/// Test: select_for_block_with_strategy() passes eligible items to strategy.
///
/// Proves API-007: "Strategy receives eligible items — non-expired, non-timelocked."
/// We verify the strategy sees the submitted (eligible) items.
#[test]
fn vv_req_api_007_strategy_receives_eligible_items() {
    use std::sync::Mutex;

    struct CountingStrategy(Mutex<usize>);
    impl BlockSelectionStrategy for CountingStrategy {
        fn select(
            &self,
            eligible: &[Arc<MempoolItem>],
            _max_cost: u64,
            _max_spends: usize,
        ) -> Vec<Arc<MempoolItem>> {
            *self.0.lock().unwrap() = eligible.len();
            vec![] // Return empty; we only care about what was received.
        }
    }

    let mempool = Mempool::new(DIG_TESTNET);
    for i in 0x01..=0x03u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    let strategy = CountingStrategy(Mutex::new(0));
    mempool.select_for_block_with_strategy(&strategy, u64::MAX, 0, 0);

    let count = *strategy.0.lock().unwrap();
    assert_eq!(count, 3, "strategy must receive all 3 eligible items");
}
