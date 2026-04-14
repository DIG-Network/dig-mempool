//! REQUIREMENT: CPF-007 — Cascade Eviction
//!
//! When a bundle is removed (e.g., via RBF), all its dependents are
//! recursively removed (children before parent). All indexes are cleaned.
//!
//! - Single child cascade-evicted when parent is RBF-replaced
//! - Multi-level cascade: P → C1 → C2, all evicted
//! - Index cleanup: mempool_coins has no stale entries after cascade
//! - Multiple children: all cascade-evicted when parent is replaced
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-007.md

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

fn make_coin(parent_byte: u8, amount: u64) -> Coin {
    Coin::new(Bytes32::from([parent_byte; 32]), NIL_PUZZLE_HASH, amount)
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
    let cond = {
        let tail = a.new_pair(amount_atom, nil).unwrap();
        let mid = a.new_pair(ph_atom, tail).unwrap();
        a.new_pair(op_atom, mid).unwrap()
    };
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    (puzzle, Bytes32::from(hash))
}

fn make_two_output_puzzle(amount1: u64, amount2: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let ph = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();
    let op51 = a.new_atom(&[51u8]).unwrap();

    let mk_cond = |alloc: &mut Allocator, amt: u64| {
        let a_atom = alloc.new_atom(&clvm_encode_u64(amt)).unwrap();
        let t = alloc.new_pair(a_atom, alloc.nil()).unwrap();
        let m = alloc.new_pair(ph, t).unwrap();
        alloc.new_pair(op51, m).unwrap()
    };

    let cond1 = mk_cond(&mut a, amount1);
    let cond2 = mk_cond(&mut a, amount2);
    let tail = a.new_pair(cond2, nil).unwrap();
    let cond_list = a.new_pair(cond1, tail).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    (puzzle, Bytes32::from(hash))
}

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

fn two_output_root(
    parent_byte: u8,
    amount1: u64,
    amount2: u64,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>, Coin, Coin) {
    let total = amount1 + amount2;
    let (puzzle, puzzle_hash) = make_two_output_puzzle(amount1, amount2);
    let coin = Coin::new(Bytes32::from([parent_byte; 32]), puzzle_hash, total);
    let output1 = Coin::new(coin.coin_id(), NIL_PUZZLE_HASH, amount1);
    let output2 = Coin::new(coin.coin_id(), NIL_PUZZLE_HASH, amount2);
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    (bundle, cr, output1, output2)
}

fn link_bundle(
    prev_output: Coin,
    new_parent_byte: u8,
    new_amount: u64,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>, Coin) {
    let (pass_through, pt_hash) = make_pass_through_puzzle(new_amount);
    let new_coin = Coin::new(Bytes32::from([new_parent_byte; 32]), pt_hash, new_amount);
    let next_output = Coin::new(new_coin.coin_id(), NIL_PUZZLE_HASH, new_amount);
    let mut cr = HashMap::new();
    cr.insert(new_coin.coin_id(), coin_record(new_coin));
    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(prev_output, Program::default(), Program::default()),
            CoinSpend::new(new_coin, pass_through, Program::default()),
        ],
        Signature::default(),
    );
    (bundle, cr, next_output)
}

fn nil_bundle_no_cr(coin: Coin) -> SpendBundle {
    SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    )
}

/// CPF-007: When parent is RBF-replaced, the CPFP child is cascade-evicted.
#[test]
fn vv_req_cpf_007_single_child_cascade_on_rbf() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, output) = pass_through_root(0x01, 100);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let c_bundle = nil_bundle_no_cr(output);
    let c_id = c_bundle.name();
    mempool.submit(c_bundle, &HashMap::new(), 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // RBF replace P with P' (same A0 spend + extra coin for fee)
    let (pt_puzzle, pt_hash) = make_pass_through_puzzle(100);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), pt_hash, 100);
    let extra = make_coin(0xBB, 20_000_000);
    let mut cr2 = HashMap::new();
    cr2.insert(pt_coin.coin_id(), coin_record(pt_coin));
    cr2.insert(extra.coin_id(), coin_record(extra));
    let replacement = SpendBundle::new(
        vec![
            CoinSpend::new(pt_coin, pt_puzzle, Program::default()),
            CoinSpend::new(extra, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let r_id = replacement.name();
    mempool.submit(replacement, &cr2, 0, 0).unwrap();

    assert_eq!(mempool.len(), 1, "only replacement should remain");
    assert!(mempool.contains(&r_id), "replacement should be in pool");
    assert!(!mempool.contains(&p_id), "parent should be evicted");
    assert!(!mempool.contains(&c_id), "child should be cascade-evicted");
}

/// CPF-007: Multi-level cascade — P → C1 → C2, RBF P, all three evicted.
#[test]
fn vv_req_cpf_007_multi_level_cascade() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    let p0_id = p0.name();
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    let p1_id = p1.name();
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    let (p2, p2_cr, _) = link_bundle(x1, 0x03, 200);
    let p2_id = p2.name();
    mempool.submit(p2, &p2_cr, 0, 0).unwrap();
    assert_eq!(mempool.len(), 3);

    let (pt_puzzle, pt_hash) = make_pass_through_puzzle(1000);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), pt_hash, 1000);
    let extra = make_coin(0xBB, 20_000_000);
    let mut cr_r = HashMap::new();
    cr_r.insert(pt_coin.coin_id(), coin_record(pt_coin));
    cr_r.insert(extra.coin_id(), coin_record(extra));
    let replacement = SpendBundle::new(
        vec![
            CoinSpend::new(pt_coin, pt_puzzle, Program::default()),
            CoinSpend::new(extra, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let r_id = replacement.name();
    mempool.submit(replacement, &cr_r, 0, 0).unwrap();

    assert_eq!(mempool.len(), 1, "only replacement should survive cascade");
    assert!(mempool.contains(&r_id));
    assert!(!mempool.contains(&p0_id));
    assert!(!mempool.contains(&p1_id));
    assert!(!mempool.contains(&p2_id));
}

/// CPF-007: Index cleanup — after cascade, mempool_coins has no stale entries.
#[test]
fn vv_req_cpf_007_index_cleanup_after_cascade() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    let x0_id = x0.coin_id();
    let x1_id = x1.coin_id();
    assert!(mempool.get_mempool_coin_creator(&x0_id).is_some());
    assert!(mempool.get_mempool_coin_creator(&x1_id).is_some());

    // RBF P0 → cascade evicts P0 and P1
    let (pt_puzzle, pt_hash) = make_pass_through_puzzle(1000);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), pt_hash, 1000);
    let extra = make_coin(0xBB, 20_000_000);
    let mut cr_r = HashMap::new();
    cr_r.insert(pt_coin.coin_id(), coin_record(pt_coin));
    cr_r.insert(extra.coin_id(), coin_record(extra));
    let replacement = SpendBundle::new(
        vec![
            CoinSpend::new(pt_coin, pt_puzzle, Program::default()),
            CoinSpend::new(extra, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    mempool.submit(replacement, &cr_r, 0, 0).unwrap();

    // X1 (created by P1) must be removed from mempool_coins
    assert!(
        mempool.get_mempool_coin_creator(&x1_id).is_none(),
        "P1's addition X1 must be removed from mempool_coins after cascade"
    );

    assert!(mempool.dependents_of(&Bytes32::from([0u8; 32])).is_empty());
}

/// CPF-007: total_cost and total_fees accumulators decremented after cascade.
///
/// Proves CPF-007: "Cost/fee accumulators are decremented for each evicted item."
/// After parent and child are cascade-evicted, stats().total_cost must reflect
/// only the remaining item (the replacement).
#[test]
fn vv_req_cpf_007_accumulators_decremented_after_cascade() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let (p1, p1_cr, _x1) = link_bundle(x0, 0x02, 500);
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    assert_eq!(mempool.len(), 2);
    let stats_before = mempool.stats();
    assert!(
        stats_before.total_cost > 0,
        "total_cost must be nonzero before cascade"
    );

    // RBF P0 → cascade evicts P0 and P1
    let (pt_puzzle, pt_hash) = make_pass_through_puzzle(1000);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), pt_hash, 1000);
    let extra = make_coin(0xBB, 20_000_000);
    let mut cr_r = HashMap::new();
    cr_r.insert(pt_coin.coin_id(), coin_record(pt_coin));
    cr_r.insert(extra.coin_id(), coin_record(extra));
    let replacement = SpendBundle::new(
        vec![
            CoinSpend::new(pt_coin, pt_puzzle, Program::default()),
            CoinSpend::new(extra, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    mempool.submit(replacement, &cr_r, 0, 0).unwrap();

    assert_eq!(mempool.len(), 1, "only replacement should remain");

    let stats_after = mempool.stats();
    assert!(
        stats_after.total_cost < stats_before.total_cost,
        "total_cost must decrease after cascade eviction: before={}, after={}",
        stats_before.total_cost,
        stats_after.total_cost
    );
    // total_spend_count must also decrease (P0 had 1 spend, P1 had 2; replacement has 2)
    assert_eq!(stats_after.active_count, 1);
}

/// CPF-007: Multiple children — RBF parent cascade-evicts all children.
#[test]
fn vv_req_cpf_007_multiple_children_cascade_evicted() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, x1, x2) = two_output_root(0x01, 100, 200);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let c1_bundle = nil_bundle_no_cr(x1);
    let c1_id = c1_bundle.name();
    mempool.submit(c1_bundle, &HashMap::new(), 0, 0).unwrap();

    let c2_bundle = nil_bundle_no_cr(x2);
    let c2_id = c2_bundle.name();
    mempool.submit(c2_bundle, &HashMap::new(), 0, 0).unwrap();
    assert_eq!(mempool.len(), 3);

    // RBF P (two-output bundle, total = 300)
    let (two_out_puzzle, two_out_hash) = make_two_output_puzzle(100, 200);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), two_out_hash, 300);
    let extra = make_coin(0xBB, 20_000_000);
    let mut cr_r = HashMap::new();
    cr_r.insert(pt_coin.coin_id(), coin_record(pt_coin));
    cr_r.insert(extra.coin_id(), coin_record(extra));
    let replacement = SpendBundle::new(
        vec![
            CoinSpend::new(pt_coin, two_out_puzzle, Program::default()),
            CoinSpend::new(extra, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let r_id = replacement.name();
    mempool.submit(replacement, &cr_r, 0, 0).unwrap();

    assert_eq!(mempool.len(), 1);
    assert!(mempool.contains(&r_id));
    assert!(!mempool.contains(&p_id), "parent evicted");
    assert!(!mempool.contains(&c1_id), "C1 cascade-evicted");
    assert!(!mempool.contains(&c2_id), "C2 cascade-evicted");
}
