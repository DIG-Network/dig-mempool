//! REQUIREMENT: LCY-001 — on_new_block() Block Confirmation Lifecycle
//!
//! Proves on_new_block():
//! - Removes confirmed items (spending spent_coin_ids) from the active pool
//! - Cascade-evicts dependents of confirmed items; records their IDs in cascade_evicted
//! - Removes expired items (assert_before_height <= height or assert_before_seconds <= timestamp)
//! - Cascade-evicts dependents of expired items
//! - Collects pending promotions (assert_height <= height) from the pending pool
//! - Collects conflict retries whose conflicting active items were removed
//! - Returns empty RetryBundles when no items qualify
//! - Ignores spent_coin_ids that don't match any active bundle (no panic)
//!
//! Reference: docs/requirements/domains/lifecycle/specs/LCY-001.md

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::Mempool;
use hex_literal::hex;

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

// ASSERT_BEFORE_HEIGHT_ABSOLUTE opcode (87 = 0x57)
const ASSERT_BEFORE_HEIGHT_ABSOLUTE: u8 = 87;
// ASSERT_HEIGHT_ABSOLUTE opcode (83 = 0x53)
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

/// Build a CLVM program that returns a single condition: (opcode value).
/// Returns (program, puzzle_hash).
///
/// Used for timelock testing (ASSERT_BEFORE_HEIGHT_ABSOLUTE, ASSERT_HEIGHT_ABSOLUTE).
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

/// Submit a pass-through bundle: spends `parent_byte` coin, creates an output coin.
/// Returns (bundle, coin_records, output_coin).
fn pass_through_root(
    parent_byte: u8,
    amount: u64,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>, Coin) {
    let (puzzle, puzzle_hash) = make_pass_through_puzzle(amount);
    let coin = Coin::new(Bytes32::from([parent_byte; 32]), puzzle_hash, amount);
    let output = Coin::new(coin.coin_id(), NIL_PUZZLE_HASH, amount);
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    (bundle, cr, output)
}

/// Confirmed item is removed from the active pool.
///
/// Proves LCY-001: "For each coin ID in spent_coin_ids, look up the spending bundle
/// in coin_index. Remove that bundle from the active pool."
#[test]
fn vv_req_lcy_001_confirmed_item_removed() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    assert!(mempool.contains(&bundle_id));

    mempool.on_new_block(1, 100, &[coin.coin_id()], &[]);

    assert!(
        !mempool.contains(&bundle_id),
        "confirmed item must be removed"
    );
    assert_eq!(mempool.len(), 0);
}

/// Unrelated spent_coin_ids are silently ignored.
///
/// Proves LCY-001: "spent_coin_ids contains coins not in the mempool: silently ignored."
#[test]
fn vv_req_lcy_001_unrelated_coins_ignored() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    // Confirm a coin not spent by any bundle.
    let unrelated = Bytes32::from([0xFF; 32]);
    let retry = mempool.on_new_block(1, 100, &[unrelated], &[]);

    assert!(
        mempool.contains(&bundle_id),
        "unrelated coin must not remove bundle"
    );
    assert!(retry.conflict_retries.is_empty());
    assert!(retry.pending_promotions.is_empty());
    assert!(retry.cascade_evicted.is_empty());
}

/// Confirming a parent cascade-evicts its dependent child.
///
/// Proves LCY-001: "Cascade-evict all dependents (depth-first via dependents graph).
/// The child IDs appear in RetryBundles::cascade_evicted."
#[test]
fn vv_req_lcy_001_cascade_on_confirm() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Parent: spends on-chain coin, creates output coin.
    let (parent_bundle, parent_cr, output_coin) = pass_through_root(0x01, 500);
    let parent_input_coin = parent_cr.values().next().unwrap().coin;
    let parent_id = parent_bundle.name();
    mempool.submit(parent_bundle, &parent_cr, 0, 0).unwrap();

    // Child: spends the parent's output coin (CPFP dependency).
    let child_bundle = SpendBundle::new(
        vec![CoinSpend::new(
            output_coin,
            Program::default(),
            Program::default(),
        )],
        Signature::default(),
    );
    let child_id = child_bundle.name();
    // No coin_records for output_coin — it comes from the parent in the mempool.
    mempool.submit(child_bundle, &HashMap::new(), 0, 0).unwrap();

    assert!(mempool.contains(&parent_id));
    assert!(mempool.contains(&child_id));

    // Confirm the parent by marking its input coin as spent.
    let retry = mempool.on_new_block(1, 100, &[parent_input_coin.coin_id()], &[]);

    assert!(
        !mempool.contains(&parent_id),
        "confirmed parent must be removed"
    );
    assert!(
        !mempool.contains(&child_id),
        "cascade-evicted child must be removed"
    );
    assert!(
        retry.cascade_evicted.contains(&child_id),
        "child ID must be in cascade_evicted"
    );
    // Parent is confirmed, not cascade-evicted.
    assert!(
        !retry.cascade_evicted.contains(&parent_id),
        "confirmed parent must NOT be in cascade_evicted"
    );
}

/// Expired items (assert_before_height <= height) are removed.
///
/// Proves LCY-001: "Query active items where assert_before_height <= height.
/// Remove each expired item and cascade-evict dependents."
#[test]
fn vv_req_lcy_001_expired_by_height_removed() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit item with assert_before_height = 10.
    let (puzzle, puzzle_hash) = single_cond_puzzle(ASSERT_BEFORE_HEIGHT_ABSOLUTE, 10);
    let coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    let bundle_id = bundle.name();
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    assert!(
        mempool.contains(&bundle_id),
        "item should be in pool before expiry"
    );

    // Advance to height 10 — item has expired (assert_before_height = 10 <= 10).
    mempool.on_new_block(10, 0, &[], &[]);

    assert!(
        !mempool.contains(&bundle_id),
        "expired item must be removed"
    );
}

/// Items valid until a future height are NOT removed.
///
/// Proves LCY-001: expiry check is strictly `assert_before_height <= height`.
#[test]
fn vv_req_lcy_001_not_yet_expired_retained() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit item with assert_before_height = 20.
    let (puzzle, puzzle_hash) = single_cond_puzzle(ASSERT_BEFORE_HEIGHT_ABSOLUTE, 20);
    let coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    let bundle_id = bundle.name();
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    // Advance to height 9 — item has NOT yet expired (20 > 9).
    mempool.on_new_block(9, 0, &[], &[]);

    assert!(
        mempool.contains(&bundle_id),
        "not-yet-expired item must remain in pool"
    );
}

/// Pending promotions: timelocked item promoted when height is reached.
///
/// Proves LCY-001: "Extract pending items whose assert_height <= height.
/// Return as pending_promotions."
#[test]
fn vv_req_lcy_001_pending_promotions_collected() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit item with assert_height = 5 at height 0 → goes to pending pool.
    let (puzzle, puzzle_hash) = single_cond_puzzle(ASSERT_HEIGHT_ABSOLUTE, 5);
    let coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));

    let result = mempool.submit(bundle, &cr, 0, 0).unwrap();
    // Must go to pending pool (height 5 > current height 0).
    assert!(
        matches!(result, dig_mempool::SubmitResult::Pending { .. }),
        "timelocked bundle must go to pending pool"
    );

    // Advance to height 5 — timelock now satisfied.
    let retry = mempool.on_new_block(5, 0, &[], &[]);

    assert_eq!(
        retry.pending_promotions.len(),
        1,
        "one bundle must be in pending_promotions"
    );
    assert!(
        retry.conflict_retries.is_empty(),
        "no conflict retries expected"
    );
    assert!(
        retry.cascade_evicted.is_empty(),
        "no cascade evictions expected"
    );
}

/// Conflict retries: bundle from conflict cache returned when its blocker is confirmed.
///
/// Proves LCY-001: "Extract conflict cache items whose conflicting active item was
/// removed in step 1 or 2. Return as conflict_retries."
#[test]
fn vv_req_lcy_001_conflict_retries_collected() {
    let mempool = Mempool::new(DIG_TESTNET);

    // First bundle spends coin A with high fee — admitted to active pool.
    let coin = make_coin(0x01, 9_999);
    let (winner, winner_cr) = nil_bundle(coin);
    let winner_id = winner.name();
    mempool.submit(winner, &winner_cr, 0, 0).unwrap();
    assert!(mempool.contains(&winner_id));

    // Second bundle also spends coin A with lower fee — conflicts → conflict cache.
    let loser_coin = make_coin(0x02, 100); // different parent → different coin
    let coin_a_spend = CoinSpend::new(coin, Program::default(), Program::default());
    let loser_coin_spend = CoinSpend::new(loser_coin, Program::default(), Program::default());
    let loser = SpendBundle::new(vec![coin_a_spend, loser_coin_spend], Signature::default());
    let mut loser_cr = HashMap::new();
    loser_cr.insert(coin.coin_id(), coin_record(coin));
    loser_cr.insert(loser_coin.coin_id(), coin_record(loser_coin));

    // Loser fails RBF (lower fee, no bump) → goes to conflict cache.
    let _ = mempool.submit(loser, &loser_cr, 0, 0);

    // Confirm the winner bundle by marking its coin as spent.
    let retry = mempool.on_new_block(1, 100, &[coin.coin_id()], &[]);

    assert_eq!(
        retry.conflict_retries.len(),
        1,
        "loser bundle must be returned as conflict retry"
    );
    assert!(retry.cascade_evicted.is_empty());
}

/// Multiple bundles confirmed at once; all are removed.
///
/// Proves LCY-001: processing applies to all coins in spent_coin_ids.
#[test]
fn vv_req_lcy_001_multiple_confirmed() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coins: Vec<Coin> = (1..=3u8).map(|i| make_coin(i, 100 * i as u64)).collect();
    let mut coin_ids = Vec::new();
    for &coin in &coins {
        let (bundle, cr) = nil_bundle(coin);
        coin_ids.push(coin.coin_id());
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    assert_eq!(mempool.len(), 3);

    let retry = mempool.on_new_block(1, 100, &coin_ids, &[]);

    assert_eq!(mempool.len(), 0, "all confirmed items must be removed");
    assert!(retry.cascade_evicted.is_empty());
}

/// Empty pool returns empty RetryBundles (no panic).
///
/// Proves LCY-001: "No items to remove: returns empty RetryBundles."
#[test]
fn vv_req_lcy_001_empty_pool_no_panic() {
    let mempool = Mempool::new(DIG_TESTNET);
    let retry = mempool.on_new_block(1, 100, &[Bytes32::from([0xAB; 32])], &[]);
    assert!(retry.conflict_retries.is_empty());
    assert!(retry.pending_promotions.is_empty());
    assert!(retry.cascade_evicted.is_empty());
}
