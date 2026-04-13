//! REQUIREMENT: CPF-003 — Maximum Dependency Depth
//!
//! Items with dependency depth > max_dependency_depth are rejected with
//! DependencyTooDeep{depth, max}. Default max is 25.
//!
//! - Depth 0 accepted
//! - Depth at limit accepted
//! - Depth > limit → DependencyTooDeep with correct fields
//! - Error includes actual depth and max
//! - max=0 disables CPFP entirely
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-003.md

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
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

/// CPF-003: Depth 0 (no dependencies) is always accepted.
#[test]
fn vv_req_cpf_003_depth_zero_accepted() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, cr, _) = pass_through_root(0x01, 100);
    assert_eq!(mempool.submit(bundle, &cr, 0, 0), Ok(SubmitResult::Success));
}

/// CPF-003: Item at exactly max_dependency_depth is accepted.
#[test]
fn vv_req_cpf_003_depth_at_limit_accepted() {
    let config = MempoolConfig::default().with_max_dependency_depth(2);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    // depth=2 == max_dependency_depth → accepted
    let (p2, p2_cr, _) = link_bundle(x1, 0x03, 200);
    assert_eq!(
        mempool.submit(p2, &p2_cr, 0, 0),
        Ok(SubmitResult::Success),
        "depth at limit should be accepted"
    );
}

/// CPF-003: Item exceeding max_dependency_depth is rejected with DependencyTooDeep.
#[test]
fn vv_req_cpf_003_depth_exceeds_limit_rejected() {
    let config = MempoolConfig::default().with_max_dependency_depth(2);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();
    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();
    let (p2, p2_cr, x2) = link_bundle(x1, 0x03, 200);
    mempool.submit(p2, &p2_cr, 0, 0).unwrap();

    // depth=3 > max_dependency_depth=2 → rejected
    let (p3, p3_cr, _) = link_bundle(x2, 0x04, 100);
    let result = mempool.submit(p3, &p3_cr, 0, 0);
    assert!(
        matches!(
            result,
            Err(MempoolError::DependencyTooDeep { depth: 3, max: 2 })
        ),
        "depth > limit must yield DependencyTooDeep, got {:?}",
        result
    );
}

/// CPF-003: Error includes actual depth and max.
#[test]
fn vv_req_cpf_003_error_includes_depth_and_max() {
    let config = MempoolConfig::default().with_max_dependency_depth(1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let (p1, p1_cr, x1) = link_bundle(x0, 0x02, 500);
    mempool.submit(p1, &p1_cr, 0, 0).unwrap();

    // depth=2 > max=1
    let (p2, p2_cr, _) = link_bundle(x1, 0x03, 200);
    let result = mempool.submit(p2, &p2_cr, 0, 0);
    assert!(
        matches!(
            result,
            Err(MempoolError::DependencyTooDeep { depth: 2, max: 1 })
        ),
        "error must report depth=2, max=1, got {:?}",
        result
    );
}

/// CPF-003: max_dependency_depth=0 disables CPFP — any dependent item rejected.
#[test]
fn vv_req_cpf_003_zero_depth_disables_cpfp() {
    let config = MempoolConfig::default().with_max_dependency_depth(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (p0, p0_cr, x0) = pass_through_root(0x01, 1000);
    mempool.submit(p0, &p0_cr, 0, 0).unwrap();

    let c_bundle = nil_bundle_no_cr(x0);
    let result = mempool.submit(c_bundle, &HashMap::new(), 0, 0);
    assert!(
        matches!(
            result,
            Err(MempoolError::DependencyTooDeep { depth: 1, max: 0 })
        ),
        "CPFP disabled: got {:?}",
        result
    );
}
