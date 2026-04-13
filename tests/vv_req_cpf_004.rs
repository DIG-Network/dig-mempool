//! REQUIREMENT: CPF-004 — Defensive Cycle Detection
//!
//! The dependency graph is checked for cycles before inserting. In the UTXO
//! model, cycles are structurally impossible (hash pre-image resistance), so
//! these tests verify no false positives on valid DAGs.
//!
//! - Linear chain P → C1 → C2: no false cycle
//! - Diamond DAG (shared grandparent): no false cycle
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-004.md

#![allow(clippy::too_many_arguments)]

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

/// CPF-004: Linear chain P → C1 → C2 — no false cycle detected.
#[test]
fn vv_req_cpf_004_linear_chain_no_false_cycle() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    assert_eq!(mempool.submit(p0, &p0_cr, 0, 0), Ok(SubmitResult::Success));
    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    assert_eq!(mempool.submit(p1, &p1_cr, 0, 0), Ok(SubmitResult::Success));
    let (p2, p2_cr, _) = link_bundle(x1, 0x03, 200);
    assert_eq!(mempool.submit(p2, &p2_cr, 0, 0), Ok(SubmitResult::Success));

    assert_eq!(mempool.len(), 3, "all 3 items should be in pool");
}

/// CPF-004: Diamond DAG (two parents share a grandparent) — no false cycle.
///
/// P creates X (100) and Y (200); C1 spends X; C2 spends Y; G spends outputs
/// of C1 and C2. Diamond: P ← C1 ← G and P ← C2 ← G.
#[test]
fn vv_req_cpf_004_diamond_dag_no_false_cycle() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, x, y) = two_output_root(0x01, 100, 200);
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let (c1_bundle, c1_cr, x2) = link_bundle(x, 0x02, 50);
    mempool.submit(c1_bundle, &c1_cr, 0, 0).unwrap();

    let (c2_bundle, c2_cr, y2) = link_bundle(y, 0x03, 80);
    mempool.submit(c2_bundle, &c2_cr, 0, 0).unwrap();

    // G: spends X2 and Y2 (two mempool coins, two parents)
    let g_bundle = SpendBundle::new(
        vec![
            CoinSpend::new(x2, Program::default(), Program::default()),
            CoinSpend::new(y2, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let result = mempool.submit(g_bundle, &HashMap::new(), 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "diamond DAG should be accepted, no cycle"
    );

    assert_eq!(mempool.len(), 4);
}
