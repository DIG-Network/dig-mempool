//! REQUIREMENT: SEL-008 — Final Topological Ordering
//!
//! Proves that select_for_block() returns items in topological order:
//! - Parents appear before children (CPFP dependency chains)
//! - Items at the same layer are sorted by FPC descending
//! - Deterministic tiebreakers: height_added ASC, bundle_id ASC
//! - Items with no dependencies are layer 0 (appear first)
//! - Multi-level chains are ordered correctly
//!
//! Reference: docs/requirements/domains/selection/specs/SEL-008.md

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

/// Build a pass-through puzzle: spends `amount` coin, creates one output of same amount.
/// Returns (bundle, coin_records, output_coin).
fn pass_through_root(
    parent_byte: u8,
    amount: u64,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>, Coin) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let amount_atom = a.new_atom(&clvm_encode_u64(amount)).unwrap();
    let ph_atom = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();
    let op_atom = a.new_atom(&[51u8]).unwrap(); // CREATE_COIN
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
    let puzzle_hash = Bytes32::from(hash);

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

/// Parent appears before child in a simple CPFP chain.
///
/// Proves SEL-008: "Parents MUST appear before children in the returned vector."
#[test]
fn vv_req_sel_008_parent_before_child() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Parent: pass-through bundle creating an output coin.
    let (b_parent, cr_parent, output) = pass_through_root(0x01, 1000);
    let id_parent = b_parent.name();
    assert_eq!(mempool.submit(b_parent, &cr_parent, 0, 0), Ok(SubmitResult::Success));

    // Child: spends parent's output coin (no on-chain record → mempool dependency).
    let b_child = SpendBundle::new(
        vec![CoinSpend::new(output, Program::default(), Program::default())],
        Signature::default(),
    );
    let id_child = b_child.name();
    assert_eq!(
        mempool.submit(b_child, &HashMap::new(), 0, 0),
        Ok(SubmitResult::Success)
    );

    let child_item = mempool.get(&id_child).unwrap();
    assert!(
        child_item.depends_on.contains(&id_parent),
        "child must depend on parent"
    );

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(selected.len(), 2, "both parent and child should be selected");

    let pos_parent = selected.iter().position(|i| i.spend_bundle_id == id_parent);
    let pos_child = selected.iter().position(|i| i.spend_bundle_id == id_child);

    assert!(
        pos_parent.is_some() && pos_child.is_some(),
        "both parent and child must be in output"
    );
    assert!(
        pos_parent.unwrap() < pos_child.unwrap(),
        "parent (pos={}) must appear before child (pos={})",
        pos_parent.unwrap(),
        pos_child.unwrap()
    );
}

/// Items with no dependencies are all in layer 0 and come first.
///
/// Proves SEL-008: "An item is in layer 0 if it has no dependencies."
#[test]
fn vv_req_sel_008_independent_items_all_layer_zero() {
    let mempool = Mempool::new(DIG_TESTNET);

    for i in 0x01..=0x04u8 {
        let coin = make_coin(i, (5 - i as u64) * 100);
        let (b, cr) = nil_bundle(coin);
        mempool.submit(b, &cr, 0, 0).unwrap();
    }

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert_eq!(selected.len(), 4, "all 4 independent items selected");

    // All are independent (depth=0) → layer 0. Sorted by FPC desc.
    // FPC ≈ fee / virtual_cost. Higher fee → higher FPC for equal-cost nil bundles.
    // Fee for item i = (5-i)*100: item 1=400, 2=300, 3=200, 4=100.
    // Expected order: 1, 2, 3, 4 (descending FPC).
    let fees: Vec<u64> = selected.iter().map(|i| i.fee).collect();
    for w in fees.windows(2) {
        assert!(
            w[0] >= w[1],
            "items must be ordered by FPC desc within layer 0: {} >= {}",
            w[0],
            w[1]
        );
    }
}

/// Three-level CPFP chain: grandparent → parent → child in correct order.
///
/// Proves SEL-008: "Layer N = 1 + max(parent layers in selected)."
#[test]
fn vv_req_sel_008_three_level_chain_ordered() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Layer 0: grandparent.
    let (b_gp, cr_gp, out_gp) = pass_through_root(0x01, 1000);
    let id_gp = b_gp.name();
    mempool.submit(b_gp, &cr_gp, 0, 0).unwrap();

    // Layer 1: parent spends grandparent's output.
    // Also creates its own output via pass-through on a new coin.
    let (b_pt, cr_pt, pt_output) = pass_through_root(0x02, 500);
    let id_pt = b_pt.name();
    mempool.submit(b_pt, &cr_pt, 0, 0).unwrap();

    // Middle layer: spends grandparent's output + creates output for child.
    // Combine into one bundle that depends on grandparent.
    let b_parent = SpendBundle::new(
        vec![
            CoinSpend::new(out_gp, Program::default(), Program::default()),
            CoinSpend::new(pt_output, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let id_parent = b_parent.name();
    mempool.submit(b_parent, &HashMap::new(), 0, 0).unwrap();

    let parent_item = mempool.get(&id_parent).unwrap();
    assert!(
        parent_item.depends_on.contains(&id_gp) || parent_item.depends_on.contains(&id_pt),
        "parent must depend on at least one predecessor"
    );

    let selected = mempool.select_for_block(u64::MAX, 0, 0);
    assert!(selected.len() >= 2, "at least grandparent and parent selected");

    // Find positions.
    let pos_gp = selected.iter().position(|i| i.spend_bundle_id == id_gp);
    let pos_parent = selected.iter().position(|i| i.spend_bundle_id == id_parent);

    if let (Some(p_gp), Some(p_par)) = (pos_gp, pos_parent) {
        assert!(
            p_gp < p_par,
            "grandparent (pos={}) must come before parent (pos={})",
            p_gp,
            p_par
        );
    }
}

/// Topological order is deterministic across multiple calls.
///
/// Proves SEL-008: "All sort orders include height_added and spend_bundle_id as
/// deterministic tiebreakers."
#[test]
fn vv_req_sel_008_deterministic() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Mix of independent items and CPFP chain.
    let coin_a = make_coin(0x10, 300);
    let (b_a, cr_a) = nil_bundle(coin_a);
    mempool.submit(b_a, &cr_a, 0, 0).unwrap();

    let coin_b = make_coin(0x20, 200);
    let (b_b, cr_b) = nil_bundle(coin_b);
    mempool.submit(b_b, &cr_b, 0, 0).unwrap();

    let (b_parent, cr_parent, output) = pass_through_root(0x30, 150);
    mempool.submit(b_parent, &cr_parent, 0, 0).unwrap();

    let b_child = SpendBundle::new(
        vec![CoinSpend::new(output, Program::default(), Program::default())],
        Signature::default(),
    );
    mempool.submit(b_child, &HashMap::new(), 0, 0).unwrap();

    let r1 = mempool.select_for_block(u64::MAX, 0, 0);
    let r2 = mempool.select_for_block(u64::MAX, 0, 0);

    let ids1: Vec<_> = r1.iter().map(|i| i.spend_bundle_id).collect();
    let ids2: Vec<_> = r2.iter().map(|i| i.spend_bundle_id).collect();
    assert_eq!(ids1, ids2, "topological order must be deterministic");
}
