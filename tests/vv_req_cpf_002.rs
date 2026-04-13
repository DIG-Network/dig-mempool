//! REQUIREMENT: CPF-002 — Dependency Resolution
//!
//! Phase 2 resolves each removal coin against coin_records (on-chain) and
//! mempool_coins (in-pool). On-chain coins create no dependency. Mempool coins
//! create a dependency edge. Unknown coins are rejected.
//!
//! - On-chain coin creates no dependency edge
//! - Mempool coin creates a dependency edge (depends_on + depth)
//! - Unknown coin → CoinNotFound (or ValidationError)
//! - Bidirectional graph: dependents_of / ancestors_of agree
//! - Depth computation for 2-level chain
//! - Multiple parents: depth = 1 + max(parent depths)
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-002.md

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolError, SubmitResult};
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

fn nil_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = nil_bundle_no_cr(coin);
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

/// CPF-002: On-chain coin (in coin_records) creates no dependency edge.
#[test]
fn vv_req_cpf_002_onchain_coin_no_dependency() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    let id = bundle.name();

    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let item = mempool.get(&id).unwrap();
    assert!(item.depends_on.is_empty(), "on-chain spend → no dependency");
    assert_eq!(item.depth, 0);
}

/// CPF-002: Mempool coin creates a dependency edge.
#[test]
fn vv_req_cpf_002_mempool_coin_creates_dependency() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (parent_bundle, parent_cr, output) = pass_through_root(0x01, 100);
    let parent_id = parent_bundle.name();
    mempool.submit(parent_bundle, &parent_cr, 0, 0).unwrap();

    let child_bundle = nil_bundle_no_cr(output);
    let child_id = child_bundle.name();
    let result = mempool.submit(child_bundle, &HashMap::new(), 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success));

    let child_item = mempool.get(&child_id).unwrap();
    assert!(
        child_item.depends_on.contains(&parent_id),
        "child must depend on parent"
    );
    assert_eq!(child_item.depth, 1);
}

/// CPF-002: Unknown coin (not on-chain, not in mempool) → some "not found" error.
#[test]
fn vv_req_cpf_002_unknown_coin_coin_not_found() {
    let mempool = Mempool::new(DIG_TESTNET);

    let phantom = Coin::new(Bytes32::from([0xDE; 32]), NIL_PUZZLE_HASH, 50);
    let bundle = nil_bundle_no_cr(phantom);
    let result = mempool.submit(bundle, &HashMap::new(), 0, 0);

    let is_not_found = match &result {
        Err(MempoolError::CoinNotFound(_)) => true,
        Err(MempoolError::ValidationError(s)) => {
            s.contains("not found") || s.contains("CoinNotFound")
        }
        _ => false,
    };
    assert!(
        is_not_found,
        "unknown coin must be rejected as not-found, got {:?}",
        result
    );
}

/// CPF-002: Bidirectional graph consistency — dependents_of and ancestors_of agree.
#[test]
fn vv_req_cpf_002_bidirectional_graph_consistency() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, output) = pass_through_root(0x01, 100);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let c_bundle = nil_bundle_no_cr(output);
    let c_id = c_bundle.name();
    mempool.submit(c_bundle, &HashMap::new(), 0, 0).unwrap();

    let dependents = mempool.dependents_of(&p_id);
    assert!(
        dependents.iter().any(|i| i.spend_bundle_id == c_id),
        "parent's dependents must include child"
    );

    let ancestors = mempool.ancestors_of(&c_id);
    assert!(
        ancestors.iter().any(|i| i.spend_bundle_id == p_id),
        "child's ancestors must include parent"
    );
}

/// CPF-002: Depth computation — 2-level chain P(0) → C1(1) → C2(2).
#[test]
fn vv_req_cpf_002_depth_computation_chain() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p0_bundle, p0_cr, x0) = pass_through_root(0x01, 1000);
    let p0_id = p0_bundle.name();
    mempool.submit(p0_bundle, &p0_cr, 0, 0).unwrap();

    let (p1_bundle, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    let p1_id = p1_bundle.name();
    mempool.submit(p1_bundle, &p1_cr, 0, 0).unwrap();

    let (p2_bundle, p2_cr, _x2) = link_bundle(x1, 0x03, 200);
    let p2_id = p2_bundle.name();
    mempool.submit(p2_bundle, &p2_cr, 0, 0).unwrap();

    assert_eq!(mempool.get(&p0_id).unwrap().depth, 0);
    assert_eq!(mempool.get(&p1_id).unwrap().depth, 1);
    assert_eq!(mempool.get(&p2_id).unwrap().depth, 2);
}

/// CPF-002: Multiple parents — child depends on two parents, depth = 1 + max.
#[test]
fn vv_req_cpf_002_multiple_parents() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p1_bundle, p1_cr, x1) = pass_through_root(0x01, 100);
    let p1_id = p1_bundle.name();
    mempool.submit(p1_bundle, &p1_cr, 0, 0).unwrap();

    let (p2_bundle, p2_cr, x2) = pass_through_root(0x02, 200);
    let p2_id = p2_bundle.name();
    mempool.submit(p2_bundle, &p2_cr, 0, 0).unwrap();

    // C spends X1 and X2 (both mempool coins) → depends on both P1 and P2
    let c_bundle = SpendBundle::new(
        vec![
            CoinSpend::new(x1, Program::default(), Program::default()),
            CoinSpend::new(x2, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let c_id = c_bundle.name();
    mempool.submit(c_bundle, &HashMap::new(), 0, 0).unwrap();

    let c_item = mempool.get(&c_id).unwrap();
    assert!(c_item.depends_on.contains(&p1_id));
    assert!(c_item.depends_on.contains(&p2_id));
    assert_eq!(c_item.depth, 1); // 1 + max(0, 0) = 1
}
