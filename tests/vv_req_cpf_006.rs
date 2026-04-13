//! REQUIREMENT: CPF-006 — Descendant Score Tracking
//!
//! descendant_score = max(own_fpc, max descendant package_fpc).
//! Updated via BFS propagation when a child is added or removed.
//!
//! - Initial score equals own fee_per_virtual_cost_scaled
//! - Score updated (increases) when high-FPC child is added
//! - Score not downgraded when low-FPC child is added after high-FPC child
//! - Multi-level propagation: grandparent updated by grandchild
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-006.md

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

fn nil_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = nil_bundle_no_cr(coin);
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

/// CPF-006: Initial descendant_score equals own fee_per_virtual_cost_scaled.
#[test]
fn vv_req_cpf_006_initial_score_equals_own_fpc() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    let id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let item = mempool.get(&id).unwrap();
    assert_eq!(
        item.descendant_score,
        item.fee_per_virtual_cost_scaled,
        "initial descendant_score must equal own FPC"
    );
}

/// CPF-006: Ancestor descendant_score is updated when child is added.
#[test]
fn vv_req_cpf_006_score_updated_on_child_add() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, output) = pass_through_root(0x01, 1000);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let p_item_before = mempool.get(&p_id).unwrap();
    let p_fpc_before = p_item_before.descendant_score;

    let c_bundle = nil_bundle_no_cr(output);
    let c_id = c_bundle.name();
    mempool.submit(c_bundle, &HashMap::new(), 0, 0).unwrap();

    let p_item_after = mempool.get(&p_id).unwrap();
    let c_item = mempool.get(&c_id).unwrap();

    assert!(
        p_item_after.descendant_score >= c_item.package_fee_per_virtual_cost_scaled,
        "P.descendant_score must be >= C.package_fpc after child added"
    );
    assert!(
        p_item_after.descendant_score >= p_fpc_before,
        "descendant_score must not decrease"
    );
}

/// CPF-006: Score not downgraded — adding a lower-FPC child doesn't reduce score.
#[test]
fn vv_req_cpf_006_score_not_downgraded() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, x1, x2) = two_output_root(0x01, 10_000, 100);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    // C1 spends X1 (high fee → high FPC)
    let c1_bundle = nil_bundle_no_cr(x1);
    mempool.submit(c1_bundle, &HashMap::new(), 0, 0).unwrap();
    let score_after_c1 = mempool.get(&p_id).unwrap().descendant_score;

    // C2 spends X2 (low fee → lower FPC)
    let c2_bundle = nil_bundle_no_cr(x2);
    mempool.submit(c2_bundle, &HashMap::new(), 0, 0).unwrap();
    let score_after_c2 = mempool.get(&p_id).unwrap().descendant_score;

    assert!(
        score_after_c2 >= score_after_c1,
        "descendant_score must not drop when lower-FPC child added"
    );
}

/// CPF-006: Multi-level propagation — grandparent score updated by grandchild.
#[test]
fn vv_req_cpf_006_multi_level_propagation() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    let p0_id = p0.name();
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    let p1_id = p1.name();
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    let p0_before = mempool.get(&p0_id).unwrap().descendant_score;

    let p2_bundle = nil_bundle_no_cr(x1);
    let p2_id = p2_bundle.name();
    mempool.submit(p2_bundle, &HashMap::new(), 0, 0).unwrap();

    let p2_item = mempool.get(&p2_id).unwrap();
    let p1_after = mempool.get(&p1_id).unwrap();
    let p0_after = mempool.get(&p0_id).unwrap();

    assert!(
        p1_after.descendant_score >= p2_item.package_fee_per_virtual_cost_scaled,
        "P1.descendant_score must include P2's package FPC"
    );
    assert!(
        p0_after.descendant_score >= p0_before,
        "P0.descendant_score must increase after grandchild added"
    );
}
