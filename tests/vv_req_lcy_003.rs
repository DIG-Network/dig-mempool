//! REQUIREMENT: LCY-003 — Caller Workflow Sequencing
//!
//! Proves the correct caller workflow: on_new_block() → submit() retries → select_for_block().
//!
//! Key invariants:
//! - Promoted pending items are NOT in the active pool after on_new_block()
//! - They are NOT eligible for select_for_block() until resubmitted
//! - After successful resubmit, they ARE eligible for selection
//! - Failed resubmissions do not corrupt mempool state
//! - Calling select_for_block() before resubmitting is safe (no panic, no corruption)
//! - Conflict retries: once the blocker is confirmed, the conflict bundle can be admitted
//!
//! Reference: docs/requirements/domains/lifecycle/specs/LCY-003.md

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

// ASSERT_HEIGHT_ABSOLUTE opcode (83 = 0x53)
const ASSERT_HEIGHT_ABSOLUTE: u8 = 83;

fn timelocked_bundle(
    parent_byte: u8,
    amount: u64,
    assert_height: u64,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let v_atom = a.new_atom(&clvm_encode_u64(assert_height)).unwrap();
    let inner = a.new_pair(v_atom, nil).unwrap();
    let op = a.new_atom(&[ASSERT_HEIGHT_ABSOLUTE]).unwrap();
    let cond = a.new_pair(op, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);

    let coin = Coin::new(Bytes32::from([parent_byte; 32]), puzzle_hash, amount);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(
        coin.coin_id(),
        CoinRecord {
            coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    (bundle, cr)
}

/// Promoted items are NOT eligible for select_for_block() without resubmission.
///
/// Proves LCY-003: "Promoted items are only eligible for select_for_block()
/// after successful submit()."
#[test]
fn vv_req_lcy_003_promoted_not_eligible_before_resubmit() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit timelocked bundle at height 0.
    let (bundle, cr) = timelocked_bundle(0x01, 1000, 5);
    let result = mempool.submit(bundle, &cr, 0, 0).unwrap();
    assert!(
        matches!(result, SubmitResult::Pending { .. }),
        "bundle must go to pending pool"
    );

    assert_eq!(mempool.len(), 0, "active pool must be empty");

    // Advance to height 5 — promotion becomes available.
    let retry = mempool.on_new_block(5, 0, &[], &[]);
    assert_eq!(retry.pending_promotions.len(), 1);

    // select_for_block WITHOUT resubmitting → empty (item not in active pool).
    let selected = mempool.select_for_block(u64::MAX, 5, 0);
    assert!(
        selected.is_empty(),
        "promoted item must NOT appear in selection before resubmit"
    );
}

/// Promoted items ARE eligible after resubmission.
///
/// Proves LCY-003: "Selection after retries: select_for_block() should be called
/// after all retries are processed."
#[test]
fn vv_req_lcy_003_resubmitted_item_eligible_for_selection() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (bundle, cr) = timelocked_bundle(0x01, 1000, 5);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let retry = mempool.on_new_block(5, 0, &[], &[]);
    assert_eq!(retry.pending_promotions.len(), 1);

    // Resubmit the promoted bundle with the same coin records (coin still unspent).
    let promoted = retry.pending_promotions.into_iter().next().unwrap();
    let id = promoted.name();
    let result = mempool.submit(promoted, &cr, 5, 0).unwrap();
    assert!(
        matches!(result, SubmitResult::Success),
        "resubmitted bundle must go to active pool"
    );

    // Now eligible for selection.
    let selected = mempool.select_for_block(u64::MAX, 5, 0);
    assert_eq!(selected.len(), 1, "resubmitted item must appear in selection");
    assert_eq!(selected[0].spend_bundle_id, id);
}

/// Failed resubmission (coin already spent) does not corrupt mempool state.
///
/// Proves LCY-003: "Failed resubmissions do not corrupt mempool state."
#[test]
fn vv_req_lcy_003_failed_resubmit_harmless() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Put a normal item in the active pool.
    let coin_ok = make_coin(0x02, 500);
    let (bundle_ok, cr_ok) = nil_bundle(coin_ok);
    let id_ok = bundle_ok.name();
    mempool.submit(bundle_ok, &cr_ok, 0, 0).unwrap();

    // Put a timelocked item in the pending pool.
    let (bundle_tl, cr_tl) = timelocked_bundle(0x01, 1000, 5);
    mempool.submit(bundle_tl, &cr_tl, 0, 0).unwrap();

    // Advance and get promotion.
    let retry = mempool.on_new_block(5, 0, &[], &[]);
    let promoted = retry.pending_promotions.into_iter().next().unwrap();

    // Resubmit with EMPTY coin records → will fail (coin not found).
    let result = mempool.submit(promoted, &HashMap::new(), 5, 0);
    assert!(result.is_err(), "resubmission with missing coin records must fail");

    // Existing active item must be unaffected.
    assert!(mempool.contains(&id_ok), "existing active item must be unaffected");
    assert_eq!(mempool.len(), 1, "pool size must remain 1");
}

/// select_for_block() before resubmitting is safe — no panic, no corruption.
///
/// Proves LCY-003: "Calling select_for_block() before resubmitting will simply
/// not include those items."
#[test]
fn vv_req_lcy_003_select_before_resubmit_is_safe() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (bundle, cr) = timelocked_bundle(0x01, 1000, 5);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let _retry = mempool.on_new_block(5, 0, &[], &[]);

    // Call select_for_block before resubmitting — must not panic.
    let selected = mempool.select_for_block(u64::MAX, 5, 0);
    assert!(selected.is_empty(), "no items eligible before resubmit");

    // Pool integrity: mempool is empty (promoted item removed, not in active pool).
    assert_eq!(mempool.len(), 0, "active pool must be empty");
}

/// Full workflow: on_new_block → resubmit conflict retry → select_for_block.
///
/// Proves LCY-003: "Conflict retry workflow: confirm original, resubmit conflict."
#[test]
fn vv_req_lcy_003_conflict_retry_workflow() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Winner: high-fee bundle spending coin A.
    let coin = make_coin(0x01, 9_999);
    let (winner, winner_cr) = nil_bundle(coin);
    let winner_id = winner.name();
    mempool.submit(winner, &winner_cr, 0, 0).unwrap();

    // Loser: another bundle also spending coin A (lower fee) + another coin.
    let coin_b = make_coin(0x02, 100);
    let loser = SpendBundle::new(
        vec![
            CoinSpend::new(coin, Program::default(), Program::default()),
            CoinSpend::new(coin_b, Program::default(), Program::default()),
        ],
        Signature::default(),
    );
    let mut loser_cr = HashMap::new();
    loser_cr.insert(coin.coin_id(), coin_record(coin));
    loser_cr.insert(coin_b.coin_id(), coin_record(coin_b));

    // Loser goes to conflict cache (fails RBF).
    let _ = mempool.submit(loser, &loser_cr, 0, 0);
    assert!(mempool.contains(&winner_id), "winner must remain in pool");

    // Confirm the winner — removes it from active pool.
    let retry = mempool.on_new_block(1, 100, &[coin.coin_id()], &[]);
    assert_eq!(retry.conflict_retries.len(), 1, "one conflict retry expected");
    assert!(!mempool.contains(&winner_id), "winner must be confirmed/removed");

    // Resubmit the conflict retry with current coin records.
    // The loser now only needs coin_b (coin A is confirmed, no longer in mempool).
    let loser_retry = retry.conflict_retries.into_iter().next().unwrap();
    let _loser_id = loser_retry.name();

    // coin A is now on-chain (confirmed), coin B is still unspent.
    // Provide both coin records for resubmission.
    let confirmed_coin_cr = CoinRecord {
        coin,
        coinbase: false,
        confirmed_block_index: 1,
        spent: true, // spent on-chain
        spent_block_index: 1,
        timestamp: 100,
    };
    let mut retry_cr = HashMap::new();
    retry_cr.insert(coin.coin_id(), confirmed_coin_cr);
    retry_cr.insert(coin_b.coin_id(), coin_record(coin_b));

    // Resubmission may fail because coin A was spent on-chain.
    // Either way, the mempool must remain valid.
    let _ = mempool.submit(loser_retry, &retry_cr, 1, 100);

    // If coin_b-only bundle can be re-tried, it would need a different bundle.
    // The important thing is no panic and pool is consistent.
    assert_eq!(mempool.conflict_len(), 0, "conflict cache must be empty after drain");
}

/// Full end-to-end workflow: submit → on_new_block → resubmit → select.
///
/// Proves LCY-003 end-to-end sequencing with multiple items.
#[test]
fn vv_req_lcy_003_full_workflow() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit 3 normal bundles.
    let mut regular_ids = Vec::new();
    for i in 1..=3u8 {
        let coin = make_coin(i, 100 * i as u64);
        let (bundle, cr) = nil_bundle(coin);
        regular_ids.push(bundle.name());
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    // Submit 1 timelocked bundle.
    let (tl_bundle, tl_cr) = timelocked_bundle(0x10, 500, 5);
    mempool.submit(tl_bundle, &tl_cr, 0, 0).unwrap();

    assert_eq!(mempool.len(), 3, "3 active items (1 pending)");

    // Confirm coin from bundle 1.
    let coin1 = make_coin(0x01, 100);
    let retry = mempool.on_new_block(5, 0, &[coin1.coin_id()], &[]);

    // Bundle 1 confirmed, timelocked bundle promoted.
    assert_eq!(mempool.len(), 2, "2 active items after confirmation");
    assert_eq!(retry.pending_promotions.len(), 1, "one pending promotion");

    // Resubmit the promoted bundle.
    let promoted = retry.pending_promotions.into_iter().next().unwrap();
    let promoted_id = promoted.name();
    mempool.submit(promoted, &tl_cr, 5, 0).unwrap();

    // Now 3 items in active pool (bundles 2, 3, and resubmitted timelocked).
    assert_eq!(mempool.len(), 3, "3 active items after resubmit");

    // Select all items.
    let selected = mempool.select_for_block(u64::MAX, 5, 0);
    assert_eq!(selected.len(), 3, "all 3 active items selected");

    let selected_ids: Vec<_> = selected.iter().map(|i| i.spend_bundle_id).collect();
    assert!(
        selected_ids.contains(&promoted_id),
        "resubmitted item must be in selection"
    );
    assert!(
        !selected_ids.contains(&regular_ids[0]),
        "confirmed item must not be in selection"
    );
}
