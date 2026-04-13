//! REQUIREMENT: CPF-001 — mempool_coins Index
//!
//! Every coin created by an active bundle is tracked in mempool_coins
//! (coin_id → creating_bundle_id). Entries are removed when the creating
//! bundle is evicted.
//!
//! - Additions registered in mempool_coins on active pool insertion
//! - Entries removed when creating item is evicted (via RBF cascade)
//! - get_mempool_coin_record returns correct synthetic CoinRecord
//! - get_mempool_coin_creator returns None for unknown coin
//!
//! Reference: docs/requirements/domains/cpfp/specs/CPF-001.md

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

fn rbf_replacement(
    pt_coin: Coin,
    pt_puzzle: Program,
    extra_fee_coin: Coin,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let mut cr = HashMap::new();
    cr.insert(pt_coin.coin_id(), coin_record(pt_coin));
    cr.insert(extra_fee_coin.coin_id(), coin_record(extra_fee_coin));
    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(pt_coin, pt_puzzle, Program::default()),
            CoinSpend::new(extra_fee_coin, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    (bundle, cr)
}

/// CPF-001: Additions registered in mempool_coins on active pool insertion.
#[test]
fn vv_req_cpf_001_additions_registered_on_insert() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, cr, output) = pass_through_root(0x01, 100);
    let parent_id = bundle.name();

    assert_eq!(mempool.submit(bundle, &cr, 0, 0), Ok(SubmitResult::Success));

    let creator = mempool.get_mempool_coin_creator(&output.coin_id());
    assert_eq!(
        creator,
        Some(parent_id),
        "output coin should be registered in mempool_coins"
    );
}

/// CPF-001: Entries removed when the creating item is removed (via RBF cascade).
#[test]
fn vv_req_cpf_001_entries_removed_on_eviction() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, cr, output) = pass_through_root(0x01, 100);
    let parent_id = bundle.name();

    mempool.submit(bundle, &cr, 0, 0).unwrap();
    assert!(mempool.get_mempool_coin_creator(&output.coin_id()).is_some());

    // RBF-replace the parent: cascade-evicts it, cleaning mempool_coins
    let (pt_puzzle, pt_hash) = make_pass_through_puzzle(100);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), pt_hash, 100);
    let extra = make_coin(0xBB, 20_000_000);
    let (replacement, cr2) = rbf_replacement(pt_coin, pt_puzzle, extra);

    mempool.submit(replacement, &cr2, 0, 0).unwrap();

    assert!(
        mempool.get_mempool_coin_creator(&output.coin_id()).is_none()
            || mempool.get_mempool_coin_creator(&output.coin_id()) != Some(parent_id),
        "evicted parent's additions should be cleaned from mempool_coins"
    );
}

/// CPF-001: get_mempool_coin_record returns correct synthetic CoinRecord.
#[test]
fn vv_req_cpf_001_get_mempool_coin_record_fields() {
    let mempool = Mempool::new(DIG_TESTNET);
    let (bundle, cr, output) = pass_through_root(0x01, 500);

    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let record = mempool
        .get_mempool_coin_record(&output.coin_id())
        .expect("should return synthetic CoinRecord");

    assert_eq!(record.coin, output, "coin field must match the addition coin");
    assert!(!record.spent, "mempool coins are not spent");
    assert!(!record.coinbase, "mempool coins are not coinbase");
}

/// CPF-001: get_mempool_coin_creator returns None for unknown coin.
#[test]
fn vv_req_cpf_001_get_creator_none_for_unknown() {
    let mempool = Mempool::new(DIG_TESTNET);
    let unknown = Bytes32::from([0xFF; 32]);
    assert!(mempool.get_mempool_coin_creator(&unknown).is_none());
    assert!(mempool.get_mempool_coin_record(&unknown).is_none());
}
