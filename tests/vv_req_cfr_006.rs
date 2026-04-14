//! REQUIREMENT: CFR-006 — Remove Conflicting Items on Successful RBF
//!
//! When RBF succeeds, all conflicting items are removed from the active pool
//! and coin_index is updated to point to the replacement bundle.
//!
//! - Conflicting item removed; replacement inserted; pool count stable
//! - Both conflicting items removed when new bundle beats both
//! - coin_index entries cleaned after removal; new bundle's entries installed
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/CFR-006.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
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

fn alt_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let alt_sol = Program::new(vec![0x01].into());
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), alt_sol)],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

fn two_coin_bundle(coin_a: Coin, coin_b: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, Program::default(), Program::default()),
            CoinSpend::new(coin_b, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin_a.coin_id(), coin_record(coin_a));
    cr.insert(coin_b.coin_id(), coin_record(coin_b));
    (bundle, cr)
}

fn three_coin_bundle(
    coin_a: Coin,
    coin_b: Coin,
    coin_c: Coin,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, Program::default(), Program::default()),
            CoinSpend::new(coin_b, Program::default(), Program::default()),
            CoinSpend::new(coin_c, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin_a.coin_id(), coin_record(coin_a));
    cr.insert(coin_b.coin_id(), coin_record(coin_b));
    cr.insert(coin_c.coin_id(), coin_record(coin_c));
    (bundle, cr)
}

/// Successful RBF: conflicting item removed, replacement inserted.
///
/// Proves CFR-006: "Remove conflicting items on successful RBF."
#[test]
fn vv_req_cfr_006_conflicting_item_removed() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);
    assert!(mempool.contains(&a_id));

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    let b_id = bundle_b.name();
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success), "RBF should succeed");

    assert!(!mempool.contains(&a_id), "Old bundle A should be removed");
    assert!(mempool.contains(&b_id), "New bundle B should be inserted");
    assert_eq!(mempool.len(), 1, "Pool has 1 item: the replacement");
}

/// Successful RBF replacing two conflicting items.
///
/// Proves CFR-006: "Both conflicting items removed when new bundle beats both."
#[test]
fn vv_req_cfr_006_two_conflicts_both_removed() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let (bundle_b, cr_b) = nil_bundle(coin_y);
    let b_id = bundle_b.name();
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let c_id = bundle_c.name();
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "RBF with two conflicts should succeed"
    );

    assert!(!mempool.contains(&a_id), "Bundle A should be removed");
    assert!(!mempool.contains(&b_id), "Bundle B should be removed");
    assert!(mempool.contains(&c_id), "Bundle C should be inserted");
    assert_eq!(mempool.len(), 1, "Only the replacement remains");
}

/// Cascade-evicted dependents of the replaced bundle are NOT added to the conflict cache.
///
/// Proves CFR-006 + CPF-007: "cascade-evicted items must NOT be added to the conflict cache."
/// When B replaces A (via RBF), A's child C is cascade-evicted. C must not appear
/// in the conflict cache (it was not a direct conflict of B).
///
/// Setup: A (pass-through, creates output_coin) → C (CPFP child, spends output_coin).
///        B replaces A via RBF. C must be cascade-evicted, NOT conflict-cached.
#[test]
fn vv_req_cfr_006_cascade_evicted_not_in_conflict_cache() {
    use dig_clvm::{
        clvmr::{serde::node_to_bytes, Allocator},
        tree_hash, TreeHash,
    };

    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Helper: build pass-through puzzle (spends coin, emits CREATE_COIN for `amount`).
    fn clvm_encode_u64(v: u64) -> Vec<u8> {
        if v == 0 {
            return vec![];
        }
        let bytes = v.to_be_bytes();
        let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
        let trimmed = &bytes[start..];
        if trimmed[0] & 0x80 != 0 {
            let mut s = Vec::with_capacity(trimmed.len() + 1);
            s.push(0x00);
            s.extend_from_slice(trimmed);
            s
        } else {
            trimmed.to_vec()
        }
    }

    let make_pass_through = |amount: u64| -> (Program, Bytes32) {
        let mut a = Allocator::new();
        let nil = a.nil();
        let amt = a.new_atom(&clvm_encode_u64(amount)).unwrap();
        let ph = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();
        let op51 = a.new_atom(&[51u8]).unwrap();
        let tail = a.new_pair(amt, nil).unwrap();
        let mid = a.new_pair(ph, tail).unwrap();
        let cond = a.new_pair(op51, mid).unwrap();
        let cond_list = a.new_pair(cond, nil).unwrap();
        let q = a.new_atom(&[1u8]).unwrap();
        let prog = a.new_pair(q, cond_list).unwrap();
        let bytes = node_to_bytes(&a, prog).unwrap();
        let puzzle = Program::new(bytes.into());
        let hash: TreeHash = tree_hash(&a, prog);
        (puzzle, Bytes32::from(hash))
    };

    // Parent A: pass-through coin → creates output_coin.
    let (pt_puzzle, pt_hash) = make_pass_through(100);
    let parent_coin = Coin::new(Bytes32::from([0x01; 32]), pt_hash, 100);
    let output_coin = Coin::new(parent_coin.coin_id(), NIL_PUZZLE_HASH, 100);
    let bundle_a = SpendBundle::new(
        vec![CoinSpend::new(
            parent_coin,
            pt_puzzle.clone(),
            Program::default(),
        )],
        Signature::default(),
    );
    let a_id = bundle_a.name();
    let mut cr_a = std::collections::HashMap::new();
    cr_a.insert(parent_coin.coin_id(), coin_record(parent_coin));
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Child C: spends output_coin (a mempool coin created by A).
    let bundle_c = SpendBundle::new(
        vec![CoinSpend::new(
            output_coin,
            Program::default(),
            Program::default(),
        )],
        Signature::default(),
    );
    let c_id = bundle_c.name();
    mempool
        .submit(bundle_c, &std::collections::HashMap::new(), 0, 0)
        .unwrap();
    assert_eq!(mempool.len(), 2, "A and C both in pool");

    // B replaces A via RBF: spends same parent_coin + extra coin for higher fee.
    let extra = make_coin(0x0F, 20_000_000);
    let bundle_b = SpendBundle::new(
        vec![
            CoinSpend::new(parent_coin, pt_puzzle, Program::default()),
            CoinSpend::new(extra, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let mut cr_b = std::collections::HashMap::new();
    cr_b.insert(parent_coin.coin_id(), coin_record(parent_coin));
    cr_b.insert(extra.coin_id(), coin_record(extra));
    let b_result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(b_result.is_ok(), "RBF B should succeed: {:?}", b_result);

    // After RBF: A removed, C cascade-evicted. Conflict cache must NOT contain C.
    assert!(!mempool.contains(&a_id), "A was replaced by B");
    assert!(!mempool.contains(&c_id), "C was cascade-evicted");

    // The conflict cache must be empty — C was cascade-evicted (not a direct conflict).
    assert_eq!(
        mempool.conflict_len(),
        0,
        "cascade-evicted child C must NOT be in the conflict cache"
    );
}

/// After successful RBF, coin_index points to replacement, not the evicted bundle.
///
/// Proves CFR-006: "coin_index updated after replacement."
/// After B replaces A, C (spending same coin_X, not superset) detects conflict
/// against B — proving the index was updated.
#[test]
fn vv_req_cfr_006_coin_index_updated_after_rbf() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);
    assert!(!mempool.contains(&a_id), "A should be gone");

    // C: only spends coin_X — conflicts with B, but B also spends coin_boost → not superset
    let (bundle_c, cr_c) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "coin_X should now be indexed to B; C fails superset, got: {:?}",
        result
    );
}
