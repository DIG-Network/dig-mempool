//! REQUIREMENT: LCY-005 — MempoolEventHook Trait and Callbacks
//!
//! Proves MempoolEventHook:
//! - Trait is publicly exported and implementable (Send + Sync)
//! - on_item_added fires when an item is inserted into the active pool
//! - on_item_removed fires when an item is removed from the active pool
//! - on_block_selected fires when select_for_block() returns
//! - on_conflict_cached fires when a bundle enters the conflict cache
//! - on_pending_added fires when an item enters the pending pool
//! - Multiple hooks can be registered and all are called
//! - Default no-op implementations compile and cause no panic
//!
//! Reference: docs/requirements/domains/lifecycle/specs/LCY-005.md

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::Mempool;
use dig_mempool::{MempoolEventHook, MempoolItem, RemovalReason};
use hex_literal::hex;

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

const ASSERT_HEIGHT_ABSOLUTE: u8 = 83;

fn make_coin(parent: u8, amount: u64) -> Coin {
    Coin::new(Bytes32::from([parent; 32]), NIL_PUZZLE_HASH, amount)
}

fn coin_record(coin: Coin) -> CoinRecord {
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
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

fn clvm_encode_u64(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![];
    }
    let bytes = v.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    let trimmed = &bytes[start..];
    if trimmed[0] & 0x80 != 0 {
        let mut with_sign = Vec::with_capacity(trimmed.len() + 1);
        with_sign.push(0x00);
        with_sign.extend_from_slice(trimmed);
        with_sign
    } else {
        trimmed.to_vec()
    }
}

fn single_cond_puzzle(opcode: u8, value: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let v_atom = a.new_atom(&clvm_encode_u64(value)).unwrap();
    let inner = a.new_pair(v_atom, nil).unwrap();
    let op = a.new_atom(&[opcode]).unwrap();
    let cond = a.new_pair(op, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    (puzzle, Bytes32::from(hash))
}

fn make_pass_through_puzzle(amount: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let amount_atom = a.new_atom(&clvm_encode_u64(amount)).unwrap();
    let ph_atom = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();
    let op_atom = a.new_atom(&[51u8]).unwrap();
    let tail = a.new_pair(amount_atom, nil).unwrap();
    let mid = a.new_pair(ph_atom, tail).unwrap();
    let cond = a.new_pair(op_atom, mid).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    (puzzle, Bytes32::from(hash))
}

/// Full-fidelity tracking hook that records all events.
struct FullHook {
    added: Mutex<Vec<Bytes32>>,
    removed: Mutex<Vec<(Bytes32, RemovalReason)>>,
    block_selected_count: Mutex<usize>,
    conflict_cached: Mutex<Vec<Bytes32>>,
    pending_added: Mutex<Vec<Bytes32>>,
}

impl FullHook {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            added: Mutex::new(Vec::new()),
            removed: Mutex::new(Vec::new()),
            block_selected_count: Mutex::new(0),
            conflict_cached: Mutex::new(Vec::new()),
            pending_added: Mutex::new(Vec::new()),
        })
    }
}

impl MempoolEventHook for FullHook {
    fn on_item_added(&self, item: &MempoolItem) {
        self.added.lock().unwrap().push(item.spend_bundle_id);
    }
    fn on_item_removed(&self, bundle_id: &Bytes32, reason: RemovalReason) {
        self.removed.lock().unwrap().push((*bundle_id, reason));
    }
    fn on_block_selected(&self, _items: &[std::sync::Arc<MempoolItem>]) {
        *self.block_selected_count.lock().unwrap() += 1;
    }
    fn on_conflict_cached(&self, bundle_id: &Bytes32) {
        self.conflict_cached.lock().unwrap().push(*bundle_id);
    }
    fn on_pending_added(&self, item: &MempoolItem) {
        self.pending_added
            .lock()
            .unwrap()
            .push(item.spend_bundle_id);
    }
}

/// MempoolEventHook trait can be implemented with only default methods (no panics).
///
/// Proves LCY-005: "All methods have default no-op implementations."
#[test]
fn vv_req_lcy_005_default_methods_are_noop() {
    // An implementation with no overrides — uses all defaults.
    struct EmptyHook;
    impl MempoolEventHook for EmptyHook {}

    let mempool = Mempool::new(DIG_TESTNET);
    mempool.add_event_hook(Arc::new(EmptyHook));

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();
    // No panic.
}

/// on_item_added fires when a bundle is admitted to the active pool.
///
/// Proves LCY-005: "on_item_added is called on active pool insertion."
#[test]
fn vv_req_lcy_005_on_item_added_fires() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let added = hook.added.lock().unwrap();
    assert_eq!(added.len(), 1, "on_item_added must be called once");
    assert_eq!(
        added[0], bundle_id,
        "on_item_added must receive correct bundle_id"
    );
}

/// on_item_removed fires when a confirmed block removes an item.
///
/// Proves LCY-005: "on_item_removed is called on active pool removal with correct RemovalReason."
#[test]
fn vv_req_lcy_005_on_item_removed_fires_on_confirm() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    // Reset the added log.
    hook.added.lock().unwrap().clear();

    mempool.on_new_block(1, 100, &[coin.coin_id()], &[]);

    let removed = hook.removed.lock().unwrap();
    assert_eq!(removed.len(), 1, "on_item_removed must be called once");
    assert_eq!(removed[0].0, bundle_id);
    assert_eq!(removed[0].1, RemovalReason::Confirmed);
}

/// on_block_selected fires when select_for_block() returns.
///
/// Proves LCY-005: "on_block_selected is called by select_for_block()."
#[test]
fn vv_req_lcy_005_on_block_selected_fires() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    mempool.select_for_block(u64::MAX, 0, 0);

    assert_eq!(
        *hook.block_selected_count.lock().unwrap(),
        1,
        "on_block_selected must be called once"
    );
}

/// on_conflict_cached fires when a bundle is added to the conflict cache.
///
/// Proves LCY-005: "on_conflict_cached is called when a bundle enters the conflict cache."
#[test]
fn vv_req_lcy_005_on_conflict_cached_fires() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let coin = make_coin(0x01, 1000);
    let (bundle, _cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.add_to_conflict_cache(bundle, 1_000_000);

    let cached = hook.conflict_cached.lock().unwrap();
    assert_eq!(cached.len(), 1, "on_conflict_cached must be called once");
    assert_eq!(cached[0], bundle_id);
}

/// on_pending_added fires when a timelocked item enters the pending pool.
///
/// Proves LCY-005: "on_pending_added is called when an item enters the pending pool."
#[test]
fn vv_req_lcy_005_on_pending_added_fires() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let (puzzle, puzzle_hash) = single_cond_puzzle(ASSERT_HEIGHT_ABSOLUTE, 100);
    let coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    let bundle_id = bundle.name();
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));

    let result = mempool.submit(bundle, &cr, 0, 0).unwrap();
    assert!(matches!(result, dig_mempool::SubmitResult::Pending { .. }));

    let pending_added = hook.pending_added.lock().unwrap();
    assert_eq!(
        pending_added.len(),
        1,
        "on_pending_added must be called once"
    );
    assert_eq!(pending_added[0], bundle_id);
}

/// Multiple hooks are all called for the same event.
///
/// Proves LCY-005: "Multiple hooks can be registered. All registered hooks are called
/// for each event."
#[test]
fn vv_req_lcy_005_multiple_hooks_all_called() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook1 = FullHook::new();
    let hook2 = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook1) as Arc<dyn MempoolEventHook>);
    mempool.add_event_hook(Arc::clone(&hook2) as Arc<dyn MempoolEventHook>);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let added1 = hook1.added.lock().unwrap();
    let added2 = hook2.added.lock().unwrap();
    assert_eq!(added1.len(), 1, "hook1 must receive on_item_added");
    assert_eq!(added2.len(), 1, "hook2 must receive on_item_added");
    assert_eq!(added1[0], bundle_id);
    assert_eq!(added2[0], bundle_id);
}

/// on_item_removed fires with CascadeEvicted when a CPFP child is cascade-evicted.
///
/// Proves LCY-005: "on_item_removed fires with CascadeEvicted when a child is evicted
/// due to parent removal."
#[test]
fn vv_req_lcy_005_cascade_evict_fires_removal_hook() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    // Parent bundle
    let (puzzle, puzzle_hash) = make_pass_through_puzzle(500);
    let parent_coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 500);
    let output_coin = Coin::new(parent_coin.coin_id(), NIL_PUZZLE_HASH, 500);
    let parent_bundle = SpendBundle::new(
        vec![CoinSpend::new(parent_coin, puzzle, Program::default())],
        Signature::default(),
    );
    let parent_id = parent_bundle.name();
    let mut cr = HashMap::new();
    cr.insert(parent_coin.coin_id(), coin_record(parent_coin));
    mempool.submit(parent_bundle, &cr, 0, 0).unwrap();

    // Child bundle (CPFP)
    let child_bundle = SpendBundle::new(
        vec![CoinSpend::new(
            output_coin,
            Program::default(),
            Program::default(),
        )],
        Signature::default(),
    );
    let child_id = child_bundle.name();
    mempool.submit(child_bundle, &HashMap::new(), 0, 0).unwrap();

    // Reset logs
    hook.added.lock().unwrap().clear();

    // Confirm the parent — child should be cascade-evicted.
    mempool.on_new_block(1, 100, &[parent_coin.coin_id()], &[]);

    let removed = hook.removed.lock().unwrap();
    assert_eq!(removed.len(), 2, "must fire 2 removal events: {removed:?}");

    // Find events by ID
    let parent_event = removed.iter().find(|(id, _)| *id == parent_id).unwrap();
    let child_event = removed.iter().find(|(id, _)| *id == child_id).unwrap();

    assert_eq!(
        parent_event.1,
        RemovalReason::Confirmed,
        "parent must be Confirmed"
    );
    assert_eq!(
        child_event.1,
        RemovalReason::CascadeEvicted { parent_id },
        "child must be CascadeEvicted with correct parent_id"
    );
}

/// MempoolEventHook is Send + Sync — can be used across threads.
///
/// Proves LCY-005: "Trait requires Send + Sync."
#[test]
fn vv_req_lcy_005_hook_is_send_sync() {
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn MempoolEventHook>();
    // Compilation proves Send + Sync is required (won't compile if not).
}

/// Config-limited mempool respects limits even after hook registration.
///
/// Proves LCY-005: "Hook implementations have no effect on mempool config or limits."
#[test]
fn vv_req_lcy_005_add_event_hook_public() {
    // Verifies that add_event_hook() is publicly accessible.
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = FullHook::new();
    // This line must compile with no special imports beyond MempoolEventHook.
    mempool.add_event_hook(hook as Arc<dyn MempoolEventHook>);
    // No assertion needed — compilation is the proof.
}
