//! REQUIREMENT: LCY-004 — clear() Reorg Recovery Reset
//!
//! Proves clear():
//! - Resets all active pool state (items, coin_index, mempool_coins, dependency graph)
//! - Resets pending pool (pending, pending_coin_index, cost accumulators)
//! - Resets conflict cache
//! - Resets seen cache (allows resubmission of previously seen bundles)
//! - Preserves registered event hooks
//! - Preserves MempoolConfig (max_cost, limits)
//! - Fires on_item_removed(Cleared) for all removed active and pending items
//!
//! Reference: docs/requirements/domains/lifecycle/specs/LCY-004.md

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::Mempool;
use dig_mempool::{MempoolConfig, MempoolEventHook, MempoolItem, RemovalReason};
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

/// Build a CLVM program that returns a single condition: (opcode value).
/// Returns (program, puzzle_hash).
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

/// Hook that records on_item_removed calls.
struct ClearHook {
    cleared: Mutex<Vec<(Bytes32, RemovalReason)>>,
    added: Mutex<Vec<Bytes32>>,
}

impl ClearHook {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cleared: Mutex::new(Vec::new()),
            added: Mutex::new(Vec::new()),
        })
    }
}

impl MempoolEventHook for ClearHook {
    fn on_item_removed(&self, bundle_id: &Bytes32, reason: RemovalReason) {
        self.cleared.lock().unwrap().push((*bundle_id, reason));
    }
    fn on_item_added(&self, item: &MempoolItem) {
        self.added.lock().unwrap().push(item.spend_bundle_id);
    }
}

/// After clear(), len() returns 0 and is_empty() returns true.
///
/// Proves LCY-004: "After clear(), mempool is in the same state as newly constructed."
#[test]
fn vv_req_lcy_004_empty_after_clear() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin1 = make_coin(0x01, 1000);
    let coin2 = make_coin(0x02, 2000);
    let (b1, cr1) = nil_bundle(coin1);
    let (b2, cr2) = nil_bundle(coin2);
    let mut combined = cr1;
    combined.extend(cr2);
    mempool.submit(b1, &combined, 0, 0).unwrap();
    mempool.submit(b2, &combined, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    mempool.clear();

    assert_eq!(mempool.len(), 0, "len must be 0 after clear");
    assert!(mempool.is_empty(), "is_empty must be true after clear");
}

/// All active items are removed: get() returns None for previously active items.
///
/// Proves LCY-004: "All active pool state is cleared (items, coin_index, mempool_coins)."
#[test]
fn vv_req_lcy_004_active_items_removed() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();
    assert!(mempool.contains(&bundle_id));

    mempool.clear();

    assert!(
        mempool.get(&bundle_id).is_none(),
        "get() must return None for cleared item"
    );
    assert!(
        !mempool.contains(&bundle_id),
        "contains() must return false for cleared item"
    );
}

/// Pending (timelocked) items are removed: pending_len() returns 0 after clear().
///
/// Proves LCY-004: "Pending pool is cleared."
#[test]
fn vv_req_lcy_004_pending_items_cleared() {
    let mempool = Mempool::new(DIG_TESTNET);
    // Submit a bundle with assert_height = 100 at current_height = 0 → pending.
    let (puzzle, puzzle_hash) = single_cond_puzzle(ASSERT_HEIGHT_ABSOLUTE, 100);
    let coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));

    let result = mempool.submit(bundle, &cr, 0, 0).unwrap();
    assert!(
        matches!(result, dig_mempool::SubmitResult::Pending { .. }),
        "bundle must be pending"
    );
    assert_eq!(mempool.pending_len(), 1);

    mempool.clear();

    assert_eq!(
        mempool.pending_len(),
        0,
        "pending_len must be 0 after clear"
    );
}

/// Conflict cache is cleared: conflict_len() returns 0 after clear().
///
/// Proves LCY-004: "Conflict cache is cleared."
#[test]
fn vv_req_lcy_004_conflict_cache_cleared() {
    let mempool = Mempool::new(DIG_TESTNET);
    // Directly add a bundle to the conflict cache.
    let coin = make_coin(0x01, 1000);
    let (bundle, _cr) = nil_bundle(coin);
    mempool.add_to_conflict_cache(bundle, 1_000_000);
    assert_eq!(mempool.conflict_len(), 1);

    mempool.clear();

    assert_eq!(
        mempool.conflict_len(),
        0,
        "conflict_len must be 0 after clear"
    );
}

/// Seen cache is cleared: previously submitted bundle can be resubmitted after clear().
///
/// Proves LCY-004: "Seen cache is cleared (allows resubmission of previously seen bundles)."
#[test]
fn vv_req_lcy_004_seen_cache_cleared() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    // Need two identical bundles — clone via serialization/reconstruction.
    let bundle2 = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    assert_eq!(bundle.name(), bundle2.name(), "bundles are identical");

    // First submission: succeeds and adds to seen cache.
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    // Second submission (before clear): rejected as AlreadySeen.
    let err = mempool.submit(bundle2.clone(), &cr, 0, 0).unwrap_err();
    assert!(
        matches!(err, dig_mempool::MempoolError::AlreadySeen(_)),
        "must be AlreadySeen: {err:?}"
    );

    // After clear, the seen cache is empty so the bundle can be resubmitted.
    mempool.clear();

    let result = mempool.submit(bundle2, &cr, 0, 0);
    assert!(
        result.is_ok(),
        "resubmission after clear must succeed, got: {result:?}"
    );
}

/// Registered event hooks are preserved across clear().
///
/// Proves LCY-004: "Event hooks and configuration are preserved across clear()."
#[test]
fn vv_req_lcy_004_hooks_preserved() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = ClearHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    // Populate and clear.
    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();
    mempool.clear();

    // Verify the hook is still registered: submit a new bundle after clear.
    let coin2 = make_coin(0x02, 2000);
    let (bundle2, cr2) = nil_bundle(coin2);
    mempool.submit(bundle2, &cr2, 0, 0).unwrap();

    let added = hook.added.lock().unwrap();
    assert!(
        !added.is_empty(),
        "hook must still be registered after clear and fire on_item_added"
    );
}

/// Configuration is preserved across clear(): max_total_cost unchanged in stats().
///
/// Proves LCY-004: "Configuration is preserved."
#[test]
fn vv_req_lcy_004_config_preserved() {
    let config = MempoolConfig::default().with_max_total_cost(1_234_567_890);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    let expected_max = 1_234_567_890;

    assert_eq!(mempool.stats().max_cost, expected_max);

    // Populate and clear.
    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();
    mempool.clear();

    assert_eq!(
        mempool.stats().max_cost,
        expected_max,
        "max_cost must be preserved after clear"
    );
}

/// on_item_removed(Cleared) is fired for every active and pending item on clear().
///
/// Proves LCY-004: "clear() MUST fire on_item_removed(bundle_id, RemovalReason::Cleared)
/// for every active and pending item."
#[test]
fn vv_req_lcy_004_removal_hooks_fired() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = ClearHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    // Submit two active items.
    let coin1 = make_coin(0x01, 1000);
    let coin2 = make_coin(0x02, 2000);
    let (b1, cr1) = nil_bundle(coin1);
    let (b2, cr2) = nil_bundle(coin2);
    let id1 = b1.name();
    let id2 = b2.name();
    let mut combined = cr1;
    combined.extend(cr2);
    mempool.submit(b1, &combined, 0, 0).unwrap();
    mempool.submit(b2, &combined, 0, 0).unwrap();

    // Submit one pending item.
    let (puzzle, puzzle_hash) = single_cond_puzzle(ASSERT_HEIGHT_ABSOLUTE, 100);
    let coin3 = Coin::new(Bytes32::from([0x03; 32]), puzzle_hash, 3000);
    let b3 = SpendBundle::new(
        vec![CoinSpend::new(coin3, puzzle, Program::default())],
        Signature::default(),
    );
    let id3 = b3.name();
    let mut cr3 = HashMap::new();
    cr3.insert(coin3.coin_id(), coin_record(coin3));
    mempool.submit(b3, &cr3, 0, 0).unwrap();

    // Clear the hook log from the initial submissions.
    hook.cleared.lock().unwrap().clear();

    mempool.clear();

    let events = hook.cleared.lock().unwrap();
    assert_eq!(
        events.len(),
        3,
        "must fire 3 removal events (2 active + 1 pending): got {events:?}"
    );

    let ids: Vec<Bytes32> = events.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&id1), "id1 must be in removal events");
    assert!(ids.contains(&id2), "id2 must be in removal events");
    assert!(ids.contains(&id3), "id3 must be in removal events");

    for (_, reason) in events.iter() {
        assert_eq!(
            *reason,
            RemovalReason::Cleared,
            "all removal reasons must be Cleared, got {reason:?}"
        );
    }
}
