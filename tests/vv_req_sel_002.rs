//! REQUIREMENT: SEL-002 — Pre-Selection Filtering
//!
//! Proves that select_for_block() filters eligible items at the current
//! height/timestamp before running any selection strategy:
//! - Expired-by-height items excluded (assert_before_height <= height)
//! - Expired-by-seconds items excluded (assert_before_seconds <= timestamp)
//! - Future-timelocked items excluded (assert_height > height)
//! - None timelocks never filtered
//! - Boundary: assert_before_height == height → excluded (strict upper bound)
//! - Boundary: assert_height == height → included (minimum is met)
//!
//! Reference: docs/requirements/domains/selection/specs/SEL-002.md

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, SubmitResult};
use hex_literal::hex;

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

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

/// Build a puzzle returning `(opcode value)` as its only condition.
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

fn submit_with_opcode(
    mempool: &Mempool,
    parent: u8,
    amount: u64,
    opcode: u8,
    value: u64,
    height: u64,
    timestamp: u64,
) -> SubmitResult {
    let (puzzle, puzzle_hash) = single_cond_puzzle(opcode, value);
    let coin = Coin::new(Bytes32::from([parent; 32]), puzzle_hash, amount);
    let mut cr = HashMap::new();
    cr.insert(
        coin.coin_id(),
        CoinRecord {
            coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    mempool.submit(bundle, &cr, height, timestamp).unwrap()
}

/// Item with assert_before_height=10 at select height=10 is excluded.
///
/// Proves SEL-002: "assert_before_height <= height → Exclude"
#[test]
fn vv_req_sel_002_expired_by_height_excluded() {
    let mempool = Mempool::new(DIG_TESTNET);

    // 87 = ASSERT_BEFORE_HEIGHT_ABSOLUTE. Admitted at height=5 (5 < 10).
    let r = submit_with_opcode(&mempool, 0x01, 1000, 87, 10, 5, 0);
    assert_eq!(
        r,
        SubmitResult::Success,
        "item should be admitted at height=5"
    );
    assert_eq!(mempool.len(), 1);

    // At height=10, assert_before_height=10 → 10 <= 10 → excluded.
    let selected = mempool.select_for_block(u64::MAX, 10, 0);
    assert!(
        selected.is_empty(),
        "expired item (assert_before_height=10, height=10) must be excluded"
    );
}

/// Item with assert_before_seconds=5000 at timestamp=5000 is excluded.
///
/// Proves SEL-002: "assert_before_seconds <= timestamp → Exclude"
#[test]
fn vv_req_sel_002_expired_by_seconds_excluded() {
    let mempool = Mempool::new(DIG_TESTNET);

    // 85 = ASSERT_BEFORE_SECONDS_ABSOLUTE. Admitted at timestamp=1000 (1000 < 5000).
    let r = submit_with_opcode(&mempool, 0x01, 1000, 85, 5000, 0, 1000);
    assert_eq!(
        r,
        SubmitResult::Success,
        "item should be admitted at ts=1000"
    );
    assert_eq!(mempool.len(), 1);

    // At timestamp=5000, assert_before_seconds=5000 → 5000 <= 5000 → excluded.
    let selected = mempool.select_for_block(u64::MAX, 0, 5000);
    assert!(
        selected.is_empty(),
        "expired item (assert_before_seconds=5000, ts=5000) must be excluded"
    );
}

/// Items with no timelocks are never filtered out.
///
/// Proves SEL-002: "Items with None for any of these fields are unconstrained."
#[test]
fn vv_req_sel_002_no_timelocks_always_included() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    // Select at any height/timestamp → item has no timelocks → always included.
    let selected = mempool.select_for_block(u64::MAX, 999, 999_999);
    assert_eq!(
        selected.len(),
        1,
        "unconstrained item must be included at any height/ts"
    );
}

/// Item with assert_height=5 submitted at height=10 is excluded at select height=4.
///
/// Proves SEL-002: "assert_height > height → Exclude (future-timelocked)"
#[test]
fn vv_req_sel_002_future_timelocked_excluded() {
    let mempool = Mempool::new(DIG_TESTNET);

    // 83 = ASSERT_HEIGHT_ABSOLUTE(5). Submitted at height=10 (5 <= 10) → active pool.
    let r = submit_with_opcode(&mempool, 0x01, 1000, 83, 5, 10, 0);
    assert_eq!(
        r,
        SubmitResult::Success,
        "item with assert_height=5 at h=10 → active"
    );
    assert_eq!(mempool.len(), 1);

    // At height=4, assert_height=5 > 4 → excluded.
    let selected = mempool.select_for_block(u64::MAX, 4, 0);
    assert!(
        selected.is_empty(),
        "future-timelocked item (assert_height=5, select_height=4) must be excluded"
    );
}

/// Boundary: assert_height == select height → item is included.
///
/// Proves SEL-002: "assert_height is a minimum: valid when height >= assert_height."
#[test]
fn vv_req_sel_002_assert_height_at_boundary_included() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit ASSERT_HEIGHT_ABSOLUTE(5) at height=10 → active with assert_height=5.
    submit_with_opcode(&mempool, 0x01, 1000, 83, 5, 10, 0);
    assert_eq!(mempool.len(), 1);

    // At height=5, assert_height=5 → 5 > 5 is false → included.
    let selected = mempool.select_for_block(u64::MAX, 5, 0);
    assert_eq!(
        selected.len(),
        1,
        "assert_height=5 at height=5 must be included (5 >= 5)"
    );
}

/// Boundary: assert_before_height exactly at height → excluded.
///
/// Proves SEL-002: "assert_before_height is a strict upper bound: valid only when
/// height < assert_before_height."
#[test]
fn vv_req_sel_002_assert_before_height_at_boundary_excluded() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Admitted at height=8 (8 < 10). assert_before_height=10.
    submit_with_opcode(&mempool, 0x01, 1000, 87, 10, 8, 0);
    assert_eq!(mempool.len(), 1);

    // At height=9: 10 <= 9 is false → included.
    let s1 = mempool.select_for_block(u64::MAX, 9, 0);
    assert_eq!(s1.len(), 1, "one below boundary → included");

    // At height=10: 10 <= 10 → excluded.
    let s2 = mempool.select_for_block(u64::MAX, 10, 0);
    assert!(s2.is_empty(), "at-boundary → excluded");
}

/// Expired item and valid item: only valid is returned.
///
/// Proves SEL-002: filtering excludes only ineligible items.
#[test]
fn vv_req_sel_002_mixed_expired_and_valid() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Item A: expires at height=10 (admitted at height=5).
    submit_with_opcode(&mempool, 0x01, 1000, 87, 10, 5, 0);

    // Item B: no timelock constraint.
    let coin_b = make_coin(0x02, 500);
    let (bundle_b, cr_b) = nil_bundle(coin_b);
    let id_b = bundle_b.name();
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();

    assert_eq!(mempool.len(), 2);

    // At height=10: A is expired, B is valid.
    let selected = mempool.select_for_block(u64::MAX, 10, 0);
    assert_eq!(selected.len(), 1, "only non-expired item selected");
    assert_eq!(selected[0].spend_bundle_id, id_b, "item B must be selected");
}
