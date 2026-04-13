//! REQUIREMENT: CPF-008 — Cross-Bundle Announcement Validation (no-op)
//!
//! Per SPEC §5.9, assertions referencing non-ancestor bundles are not rejected
//! in the mempool — they may be satisfied during block validation. This is
//! implemented as a no-op: all CPFP items are admitted regardless of
//! announcement assertion state.
//!
//! - CPFP child bundle admitted regardless of assertion conditions
//! - Non-CPFP item (no dependencies) admitted normally
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-008.md

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

/// CPF-008: CPFP child bundle is admitted regardless of assertion conditions.
///
/// Per spec §5.9: "Assertions referencing non-ancestor bundles are not
/// rejected." CPF-008 is implemented as a no-op.
#[test]
fn vv_req_cpf_008_cpfp_item_admitted_regardless_of_assertions() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, output) = pass_through_root(0x01, 1000);
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let c_bundle = nil_bundle_no_cr(output);
    let c_id = c_bundle.name();
    let result = mempool.submit(c_bundle, &HashMap::new(), 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "CPFP child must be admitted; CPF-008 is a no-op"
    );
    assert!(mempool.contains(&c_id));
}

/// CPF-008: Non-CPFP item (no dependencies) — no cross-bundle validation
/// performed; item admitted normally.
#[test]
fn vv_req_cpf_008_no_validation_for_non_cpfp_items() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    assert_eq!(mempool.submit(bundle, &cr, 0, 0), Ok(SubmitResult::Success));
    assert_eq!(mempool.len(), 1);
}
