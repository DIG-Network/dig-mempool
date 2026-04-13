//! REQUIREMENT: CPF-005 — Package Fee Rate Computation
//!
//! package_fee = item.fee + sum(parent.package_fee).
//! package_virtual_cost = item.vc + sum(parent.package_virtual_cost).
//! Uses parent.package_* (not individual) to include transitive ancestors.
//!
//! - Root item: package == individual
//! - Single parent: child.package_fee = child.fee + parent.package_fee
//! - Multi-level: grandchild includes grandparent's fees transitively
//! - Multiple parents: package sums both parents' values
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-005.md

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

/// CPF-005: Root item (no deps) — package == individual.
#[test]
fn vv_req_cpf_005_root_item_package_equals_individual() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, cr, _) = pass_through_root(0x01, 100);
    let id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let item = mempool.get(&id).unwrap();
    assert_eq!(item.package_fee, item.fee);
    assert_eq!(item.package_virtual_cost, item.virtual_cost);
    assert_eq!(
        item.package_fee_per_virtual_cost_scaled,
        item.fee_per_virtual_cost_scaled
    );
}

/// CPF-005: Single parent chain — child.package_fee = child.fee + parent.package_fee.
#[test]
fn vv_req_cpf_005_single_parent_package_fee() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, output) = pass_through_root(0x01, 1000);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let c_bundle = nil_bundle_no_cr(output);
    let c_id = c_bundle.name();
    mempool.submit(c_bundle, &HashMap::new(), 0, 0).unwrap();

    let p_item = mempool.get(&p_id).unwrap();
    let c_item = mempool.get(&c_id).unwrap();

    assert_eq!(
        c_item.package_fee,
        c_item.fee + p_item.package_fee,
        "child package_fee must include parent's package_fee"
    );
    assert_eq!(
        c_item.package_virtual_cost,
        c_item.virtual_cost + p_item.package_virtual_cost,
        "child package_virtual_cost must include parent's"
    );
}

/// CPF-005: Multi-level chain — grandchild includes grandparent's fees transitively.
#[test]
fn vv_req_cpf_005_transitive_ancestors_included() {
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

    let p0_item = mempool.get(&p0_id).unwrap();
    let p1_item = mempool.get(&p1_id).unwrap();
    let p2_item = mempool.get(&p2_id).unwrap();

    assert_eq!(
        p2_item.package_fee,
        p2_item.fee + p1_item.package_fee,
        "P2 must include P1's package_fee"
    );
    assert_eq!(
        p1_item.package_fee,
        p1_item.fee + p0_item.package_fee,
        "P1 must include P0's package_fee"
    );
    assert_eq!(
        p2_item.package_fee,
        p2_item.fee + p1_item.fee + p0_item.fee,
        "P2 transitively includes P0.fee"
    );
}

/// CPF-005: Multiple parents — package sums both parents' package values.
#[test]
fn vv_req_cpf_005_multiple_parents_summed() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p1, p1_cr, x1) = pass_through_root(0x01, 100);
    let p1_id = p1.name();
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    let (p2, p2_cr, x2) = pass_through_root(0x02, 200);
    let p2_id = p2.name();
    mempool.submit(p2, &p2_cr, 0, 0).unwrap();

    let c_bundle = SpendBundle::new(
        vec![
            CoinSpend::new(x1, Program::default(), Program::default()),
            CoinSpend::new(x2, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let c_id = c_bundle.name();
    mempool.submit(c_bundle, &HashMap::new(), 0, 0).unwrap();

    let p1_item = mempool.get(&p1_id).unwrap();
    let p2_item = mempool.get(&p2_id).unwrap();
    let c_item = mempool.get(&c_id).unwrap();

    assert_eq!(
        c_item.package_fee,
        c_item.fee + p1_item.package_fee + p2_item.package_fee,
        "C must include both parents' package fees"
    );
}
