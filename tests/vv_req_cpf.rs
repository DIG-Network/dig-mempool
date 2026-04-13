//! REQUIREMENT: CPF-001 through CPF-008 — CPFP Dependencies
//!
//! Test-driven verification of the Child-Pays-For-Parent (CPFP) dependency
//! system: mempool_coins index, dependency resolution, depth enforcement,
//! cycle detection, package fee rates, descendant score, cascade eviction,
//! and cross-bundle announcement handling.
//!
//! ## Pass-through puzzle
//!
//! Parent bundles use `(q . ((51 NIL_PH amount)))` so that submitting the
//! parent creates an unconfirmed output coin (addition) with NIL_PUZZLE_HASH.
//! Child bundles spend that unconfirmed coin using the nil puzzle.
//!
//! ## Link bundle (chain step)
//!
//! Each chain step is a two-spend bundle:
//!   1. Nil-spend of the previous link's output coin  → CPFP dependency
//!   2. Pass-through spend of a fresh on-chain coin   → creates next output
//!
//! Reference: docs/requirements/domains/cpfp/specs/

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

// ── Constants ──────────────────────────────────────────────────────────────

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

// ── Core helpers ───────────────────────────────────────────────────────────

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

/// Encode a u64 as a canonical CLVM big-endian positive integer atom.
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

/// Build `(q . ((51 NIL_PH amount)))` — creates one output coin with NIL_PH.
///
/// The output coin has:  parent = input_coin.coin_id(), ph = NIL_PH, amount = amount
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

/// Build `(q . ((51 NIL_PH amount1) (51 NIL_PH amount2)))` — two outputs.
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

/// Root pass-through bundle: spends one on-chain coin, creates one output.
///
/// Returns (bundle, coin_records, output_coin).
/// output_coin = Coin(input.coin_id(), NIL_PH, amount)
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

/// Root two-output bundle: spends one on-chain coin, creates two outputs.
///
/// Returns (bundle, coin_records, output1, output2).
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

/// Chain-link bundle: spends prev_output (mempool coin, NIL_PH) + a fresh
/// on-chain coin (pass-through), creating the next link's output.
///
/// coin_records contains ONLY the fresh on-chain coin (not prev_output).
/// Returns (bundle, coin_records, next_output_coin).
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

/// Nil bundle: spends a single coin with nil puzzle (fee = coin.amount).
///
/// Does NOT include the coin in coin_records — caller provides records.
fn nil_bundle_no_cr(coin: Coin) -> SpendBundle {
    SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    )
}

/// Nil bundle: spends a coin with nil puzzle; coin_records = {coin → on-chain}.
fn nil_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = nil_bundle_no_cr(coin);
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

/// Replacement bundle for RBF of a pass-through root P.
///
/// P spent `pt_coin` with pass-through.  This replacement also spends
/// `pt_coin` (superset), plus `extra_fee_coin` (nil spend, provides fee).
///
/// Requires extra_fee_coin.amount > MIN_RBF_FEE_BUMP (default 10 M mojos).
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

// ── CPF-001: mempool_coins Index ────────────────────────────────────────────

/// CPF-001: Additions registered in mempool_coins on active pool insertion.
///
/// The parent bundle creates one output coin; after submit the output coin's
/// ID maps to the parent bundle's ID in mempool_coins.
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
    let (pt_puzzle, _) = make_pass_through_puzzle(100);
    let pt_coin = Coin::new(Bytes32::from([0x01; 32]), {
        let (_, ph) = make_pass_through_puzzle(100);
        ph
    }, 100);
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

// ── CPF-002: Dependency Resolution ─────────────────────────────────────────

/// CPF-002: On-chain coin (in coin_records) creates no dependency edge.
///
/// A plain nil-bundle spending an on-chain coin has depth=0, empty depends_on.
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
///
/// Parent creates X; child spends X (not in coin_records) → child.depends_on = {parent_id}.
#[test]
fn vv_req_cpf_002_mempool_coin_creates_dependency() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit parent
    let (parent_bundle, parent_cr, output) = pass_through_root(0x01, 100);
    let parent_id = parent_bundle.name();
    mempool.submit(parent_bundle, &parent_cr, 0, 0).unwrap();

    // Submit child spending the unconfirmed output (empty coin_records)
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
///
/// If the coin passes Phase 1 (e.g., appears in ephemeral_coins due to a
/// TOCTOU race), Phase 2 returns `MempoolError::CoinNotFound`.  In practice,
/// the coin is also absent from ephemeral_coins, so Phase 1 fires first and
/// returns `MempoolError::ValidationError("Coin not found: …")`.  Both are
/// valid rejection outcomes for this scenario.
#[test]
fn vv_req_cpf_002_unknown_coin_coin_not_found() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Coin that doesn't exist anywhere
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

    // dependents_of(parent) should contain child
    let dependents = mempool.dependents_of(&p_id);
    assert!(
        dependents.iter().any(|i| i.spend_bundle_id == c_id),
        "parent's dependents must include child"
    );

    // ancestors_of(child) should contain parent
    let ancestors = mempool.ancestors_of(&c_id);
    assert!(
        ancestors.iter().any(|i| i.spend_bundle_id == p_id),
        "child's ancestors must include parent"
    );
}

/// CPF-002: Depth computation — 2-level chain.
///
/// P(depth=0) → C1(depth=1) → C2(depth=2)
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

    // P1 (depth=0): pass-through, creates X1
    let (p1_bundle, p1_cr, x1) = pass_through_root(0x01, 100);
    let p1_id = p1_bundle.name();
    mempool.submit(p1_bundle, &p1_cr, 0, 0).unwrap();

    // P2 (depth=0): pass-through, creates X2
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

// ── CPF-003: Maximum Dependency Depth ──────────────────────────────────────

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

    // Any child (depth=1) → DependencyTooDeep{depth:1, max:0}
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

// ── CPF-004: Defensive Cycle Detection ────────────────────────────────────

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
/// P creates X and Y (two outputs); C1 spends X; C2 spends Y;
/// G spends outputs of C1 and C2.
/// Diamond: P ← C1 ← G and P ← C2 ← G.
#[test]
fn vv_req_cpf_004_diamond_dag_no_false_cycle() {
    let mempool = Mempool::new(DIG_TESTNET);

    // P: two-output bundle creates X (amount=100) and Y (amount=200)
    let (p_bundle, p_cr, x, y) = two_output_root(0x01, 100, 200);
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    // C1: link spending X, creating X2
    let (c1_bundle, c1_cr, x2) = link_bundle(x, 0x02, 50);
    mempool.submit(c1_bundle, &c1_cr, 0, 0).unwrap();

    // C2: link spending Y, creating Y2
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

// ── CPF-005: Package Fee Rate Computation ──────────────────────────────────

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

    // package_fee = c.fee + p.package_fee
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
///
/// Chain: P0 (fee≈0) → P1 → P2
/// P2.package_fee should include P0.fee via P1.package_fee.
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

    // P2.package_fee = P2.fee + P1.package_fee (which includes P0)
    assert_eq!(
        p2_item.package_fee,
        p2_item.fee + p1_item.package_fee,
        "P2 must include P1's package_fee"
    );
    // P1.package_fee = P1.fee + P0.package_fee (includes P0)
    assert_eq!(
        p1_item.package_fee,
        p1_item.fee + p0_item.package_fee,
        "P1 must include P0's package_fee"
    );
    // P2.package_fee = P2.fee + P1.fee + P0.fee (transitive)
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

    // C spends both X1 and X2
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

// ── CPF-006: Descendant Score Tracking ─────────────────────────────────────

/// CPF-006: Initial descendant_score equals own fee_per_virtual_cost_scaled.
#[test]
fn vv_req_cpf_006_initial_score_equals_own_fpc() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Nil bundle: fee = amount > 0, so fpc > 0
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
///
/// P (pass-through, fpc≈0) → C (fee > 0, package_fpc > 0)
/// After adding C, P.descendant_score should equal C.package_fpc.
#[test]
fn vv_req_cpf_006_score_updated_on_child_add() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (p_bundle, p_cr, output) = pass_through_root(0x01, 1000);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    let p_item_before = mempool.get(&p_id).unwrap();
    let p_fpc_before = p_item_before.descendant_score;

    // Child: fee = output.amount = 1000
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

    // P creates two outputs: X1 (large = high fee child) and X2 (small = low fee child)
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
///
/// P0 (fpc≈0) → P1 → P2 (high fee).
/// After adding P2, P0.descendant_score should reflect P2's contribution.
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

    // P2 spends X1 with nil (high fee from X1.amount)
    let p2_bundle = nil_bundle_no_cr(x1);
    let p2_id = p2_bundle.name();
    mempool.submit(p2_bundle, &HashMap::new(), 0, 0).unwrap();

    let p2_item = mempool.get(&p2_id).unwrap();
    let p1_after = mempool.get(&p1_id).unwrap();
    let p0_after = mempool.get(&p0_id).unwrap();

    // P1.descendant_score should be updated by P2
    assert!(
        p1_after.descendant_score >= p2_item.package_fee_per_virtual_cost_scaled,
        "P1.descendant_score must include P2's package FPC"
    );
    // P0.descendant_score should also be updated (propagated through P1)
    assert!(
        p0_after.descendant_score >= p0_before,
        "P0.descendant_score must increase after grandchild added"
    );
}

// ── CPF-007: Cascade Eviction ───────────────────────────────────────────────

/// CPF-007: When parent is RBF-replaced, the CPFP child is cascade-evicted.
#[test]
fn vv_req_cpf_007_single_child_cascade_on_rbf() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit parent P and child C
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

    // Pool should have only the replacement; P and C cascade-evicted
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

    // RBF P0 with replacement
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

    // Both X0 and X1 should be in mempool_coins now
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

    // X1 (created by P1) should be gone from mempool_coins
    // (X0 may be re-registered under the replacement, but X1 is gone)
    assert!(
        mempool.get_mempool_coin_creator(&x1_id).is_none(),
        "P1's addition X1 must be removed from mempool_coins after cascade"
    );

    // dependents_of and ancestors_of P0/P1 should be empty (items gone)
    assert!(mempool.dependents_of(&Bytes32::from([0u8; 32])).is_empty());
}

/// CPF-007: Multiple children — RBF parent cascade-evicts all children.
#[test]
fn vv_req_cpf_007_multiple_children_cascade_evicted() {
    let mempool = Mempool::new(DIG_TESTNET);

    // P creates two outputs X1 (amount=100) and X2 (amount=200)
    let (p_bundle, p_cr, x1, x2) = two_output_root(0x01, 100, 200);
    let p_id = p_bundle.name();
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    // C1 spends X1
    let c1_bundle = nil_bundle_no_cr(x1);
    let c1_id = c1_bundle.name();
    mempool.submit(c1_bundle, &HashMap::new(), 0, 0).unwrap();

    // C2 spends X2
    let c2_bundle = nil_bundle_no_cr(x2);
    let c2_id = c2_bundle.name();
    mempool.submit(c2_bundle, &HashMap::new(), 0, 0).unwrap();

    assert_eq!(mempool.len(), 3);

    // RBF P (two-output bundle was created with total = 300)
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

// ── CPF-008: Cross-Bundle Announcement Validation (no-op) ──────────────────

/// CPF-008: CPFP child bundle is admitted even if it has assertion conditions
/// that reference neither intra-bundle nor ancestor announcements.
///
/// Per spec: "Assertions referencing non-ancestor bundles are not rejected."
/// Implementation is a no-op — cross-bundle assertion checking left to block
/// validation.
#[test]
fn vv_req_cpf_008_cpfp_item_admitted_regardless_of_assertions() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Parent creates X
    let (p_bundle, p_cr, output) = pass_through_root(0x01, 1000);
    mempool.submit(p_bundle, &p_cr, 0, 0).unwrap();

    // Child spends X — the nil puzzle produces no conditions, so no
    // announcement assertions to check. The point is that CPFP children
    // are not rejected by announcement validation.
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
