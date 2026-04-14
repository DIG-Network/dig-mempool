//! REQUIREMENT: POL-008 — Identical Spend Dedup Index
//!
//! Test-driven verification of the identical-spend dedup index:
//! - First admitted bundle establishes the cost bearer for each dedup key.
//! - Subsequent bundles with the same (coin_id, sha256(solution)) get cost saving.
//! - Bearer re-assignment when the bearing bundle is removed.
//! - Non-eligible bundles (eligible_for_dedup == false) are excluded.
//! - Feature gated by `enable_identical_spend_dedup`.
//!
//! ## What this proves
//!
//! - `dedup_index` is populated for eligible bundles (bearer establishment)
//! - Second bundle with same spend key gets `cost_saving > 0` and
//!   `effective_virtual_cost < virtual_cost`
//! - Bundles with `eligible_for_dedup == false` are NOT indexed
//! - Removing the cost bearer re-assigns to next waiter
//! - Disabling the feature (`enable_identical_spend_dedup = false`) skips all dedup
//! - Different solution bytes produce separate dedup keys (not deduped)
//! - `effective_virtual_cost = virtual_cost - cost_saving` (spec formula verified)
//!
//! ## Pass-through puzzle
//!
//! Tests use a "pass-through" CLVM puzzle:
//!   `(q . ((51 NIL_PUZZLE_HASH AMOUNT)))`
//! This creates an output with the same amount as the input coin, so no fee
//! is extracted from this spend. chia-consensus keeps `ELIGIBLE_FOR_DEDUP` set
//! because `coin_amount == spend_additions` (no excess amount).
//!
//! Reference: docs/requirements/domains/pools/specs/POL-008.md

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom.
const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

/// Build a pass-through CLVM puzzle: `(q . ((51 NIL_PUZZLE_HASH amount)))`.
///
/// Running this puzzle with ANY solution returns `((51 NIL_PUZZLE_HASH amount))`.
/// The spend creates one output coin with amount = input coin amount.
/// Because coin_amount == spend_additions, chia-consensus does NOT clear the
/// `ELIGIBLE_FOR_DEDUP` flag, so bundles using this puzzle are dedup-eligible.
fn make_pass_through_puzzle(amount: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();

    // Encode amount as a minimal big-endian atom (canonical CLVM integer).
    // For amounts <= 127, a single byte suffices. For amounts >= 128, we need
    // a leading 0x00 sign byte to distinguish from negative numbers.
    let amount_bytes = clvm_encode_u64(amount);
    let amount_atom = a.new_atom(&amount_bytes).unwrap();

    // Output puzzle hash: NIL_PUZZLE_HASH (32-byte constant)
    let ph_atom = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();

    // CREATE_COIN opcode = 51 = 0x33
    let op_atom = a.new_atom(&[51u8]).unwrap();

    // condition: (51 ph amount)
    let cond = {
        let tail = a.new_pair(amount_atom, nil).unwrap();
        let mid = a.new_pair(ph_atom, tail).unwrap();
        a.new_pair(op_atom, mid).unwrap()
    };

    // conditions list: ((51 ph amount))
    let cond_list = a.new_pair(cond, nil).unwrap();

    // quotient program: (q . cond_list)
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();

    // Serialize to bytes and compute tree hash (= puzzle_hash)
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);

    (puzzle, puzzle_hash)
}

/// Encode a u64 as a canonical CLVM big-endian positive integer atom.
///
/// CLVM integers are big-endian signed. A leading 0x00 byte is required
/// when the high bit of the most-significant payload byte would otherwise
/// indicate a negative number (i.e., value >= 128 for 1-byte representation).
fn clvm_encode_u64(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![];
    }
    let bytes = v.to_be_bytes();
    // Strip leading zero bytes.
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    let trimmed = &bytes[start..];
    // Prepend 0x00 sign byte if the high bit is set (to prevent sign-extension).
    if trimmed[0] & 0x80 != 0 {
        let mut with_sign = Vec::with_capacity(trimmed.len() + 1);
        with_sign.push(0x00);
        with_sign.extend_from_slice(trimmed);
        with_sign
    } else {
        trimmed.to_vec()
    }
}

/// Build a 1-spend pass-through bundle for a coin with the given parent prefix and amount.
///
/// Returns `(bundle, coin_records, coin_id, puzzle, puzzle_hash)`.
fn pass_through_bundle(
    parent_prefix: u8,
    amount: u64,
    solution: Program,
) -> (
    SpendBundle,
    HashMap<Bytes32, CoinRecord>,
    Bytes32,
    Program,
    Bytes32,
) {
    let (puzzle, puzzle_hash) = make_pass_through_puzzle(amount);
    let coin = Coin::new(Bytes32::from([parent_prefix; 32]), puzzle_hash, amount);
    let coin_id = coin.coin_id();

    let mut cr = HashMap::new();
    cr.insert(
        coin_id,
        CoinRecord {
            coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );

    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle.clone(), solution)],
        Signature::default(),
    );

    (bundle, cr, coin_id, puzzle, puzzle_hash)
}

/// Build a 2-spend pass-through bundle spending coin_a AND coin_b.
///
/// Both coins use the pass-through puzzle (different parent prefixes).
/// Returns `(bundle, merged_coin_records)`.
fn pass_through_bundle_2(
    parent_a: u8,
    parent_b: u8,
    amount: u64,
    solution: Program,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let (puzzle, puzzle_hash) = make_pass_through_puzzle(amount);

    let coin_a = Coin::new(Bytes32::from([parent_a; 32]), puzzle_hash, amount);
    let coin_b = Coin::new(Bytes32::from([parent_b; 32]), puzzle_hash, amount);

    let mut cr = HashMap::new();
    for coin in [coin_a, coin_b] {
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
    }

    let bundle = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, puzzle.clone(), solution.clone()),
            CoinSpend::new(coin_b, puzzle.clone(), solution),
        ],
        Signature::default(),
    );

    (bundle, cr)
}

// ──────────────────────────────────────────────────────────────────────────
// Test 1: First bundle establishes cost bearer
// ──────────────────────────────────────────────────────────────────────────

/// Test: The first eligible bundle establishes itself as the cost bearer.
///
/// Proves POL-008: "Values are the bundle ID of the cost-bearer."
/// After submitting bundle A with a pass-through spend, the dedup_index
/// should have one entry, and the bearer is A's bundle ID.
#[test]
fn vv_req_pol_008_first_bundle_establishes_bearer() {
    let mempool = Mempool::new(DIG_TESTNET);

    let (bundle_a, cr_a, coin_x_id, _puzzle, _ph) =
        pass_through_bundle(0x01, 100, Program::default());
    let a_id = bundle_a.name();

    // Submit A → should go to active pool, eligible_for_dedup = true
    let result = mempool.submit(bundle_a, &cr_a, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success));

    // Retrieve item and verify eligibility
    let item_a = mempool
        .get(&a_id)
        .expect("bundle A should be in active pool");
    assert!(
        item_a.eligible_for_dedup,
        "pass-through bundle should be eligible_for_dedup"
    );

    // dedup_index should have exactly 1 entry: (coin_x_id, sha256(nil)) → a_id
    assert_eq!(
        mempool.dedup_index_len(),
        1,
        "one dedup key should be registered"
    );

    // The bearer for coin_x's dedup key should be bundle A
    // Solution = Program::default() → sha256("") = known hash
    let solution_hash = sha256_of(Program::default().as_ref());
    let bearer = mempool.get_dedup_bearer(&coin_x_id, &solution_hash);
    assert_eq!(
        bearer,
        Some(a_id),
        "bundle A should be the cost bearer for coin X's key"
    );

    // Bundle A is the bearer: cost_saving should be 0 (no prior bearer)
    assert_eq!(item_a.cost_saving, 0, "bearer has no cost saving");
    assert_eq!(
        item_a.effective_virtual_cost, item_a.virtual_cost,
        "bearer: effective_virtual_cost equals virtual_cost"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Test 2: Second bundle gets cost saving
// ──────────────────────────────────────────────────────────────────────────

/// Test: A second eligible bundle with the same spend gets cost saving.
///
/// Proves POL-008: "Duplicate spends reduce the new item's effective cost."
/// Bundle B has 2 spends including coin X (same as A). Bundle B's cost
/// for coin X's spend is "saved" because A is the bearer.
#[test]
fn vv_req_pol_008_second_bundle_gets_cost_saving() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit A (1-spend, coin X) → bearer for key_x
    let (bundle_a, cr_a, _coin_x_id, _puzzle, _ph) =
        pass_through_bundle(0x01, 100, Program::default());
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Submit B (2-spend: coin X + coin Y, same puzzle, same solution)
    // B has the same CoinSpend for X as A, plus an extra spend of coin Y.
    // Since CFR-001 is not yet implemented, both can coexist in the pool.
    let (bundle_b, cr_b) = pass_through_bundle_2(0x01, 0x02, 100, Program::default());
    let b_id = bundle_b.name();

    let result_b = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result_b,
        Ok(SubmitResult::Success),
        "bundle B should be admitted"
    );

    let item_b = mempool.get(&b_id).expect("bundle B should be in pool");

    // Bundle B should be eligible for dedup
    assert!(
        item_b.eligible_for_dedup,
        "bundle B should be eligible_for_dedup"
    );

    // Bundle B should have cost_saving > 0 (coin X's spend is deduped)
    assert!(
        item_b.cost_saving > 0,
        "bundle B should have cost_saving > 0 (coin X spend is deduped), got cost_saving={}",
        item_b.cost_saving
    );

    // effective_virtual_cost should be less than virtual_cost
    assert!(
        item_b.effective_virtual_cost < item_b.virtual_cost,
        "effective_virtual_cost ({}) should be < virtual_cost ({})",
        item_b.effective_virtual_cost,
        item_b.virtual_cost
    );

    // effective_virtual_cost = virtual_cost - cost_saving (spec formula)
    assert_eq!(
        item_b.effective_virtual_cost,
        item_b.virtual_cost - item_b.cost_saving,
        "effective_virtual_cost = virtual_cost - cost_saving"
    );

    // cost_saving = (cost / num_spends) * 1 deduped spend
    // Bundle B has 2 spends, 1 deduped (coin X)
    let expected_cost_saving = item_b.cost / item_b.num_spends as u64;
    assert_eq!(
        item_b.cost_saving, expected_cost_saving,
        "cost_saving should equal cost/num_spends for 1 deduped spend"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Test 3: Non-eligible bundles are NOT indexed
// ──────────────────────────────────────────────────────────────────────────

/// Test: Bundles with `eligible_for_dedup == false` are not added to dedup_index.
///
/// Proves POL-008: "Only items with eligible_for_dedup == true participate."
/// Nil-puzzle bundles have ELIGIBLE_FOR_DEDUP cleared by chia-consensus because
/// coin_amount > spend_additions (the coin is consumed, creating no outputs).
#[test]
fn vv_req_pol_008_non_eligible_not_indexed() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Build a nil-puzzle bundle (spends coin without creating output → fee = amount)
    // chia-consensus clears ELIGIBLE_FOR_DEDUP because coin_amount > spend_additions (0)
    let coin = Coin::new(Bytes32::from([0x01u8; 32]), NIL_PUZZLE_HASH, 100);
    let coin_id = coin.coin_id();
    let mut cr = HashMap::new();
    cr.insert(
        coin_id,
        CoinRecord {
            coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let bundle_id = bundle.name();

    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let item = mempool.get(&bundle_id).expect("bundle should be admitted");

    // nil-puzzle bundle: eligible_for_dedup should be false (coin_amount > 0 = additions)
    assert!(
        !item.eligible_for_dedup,
        "nil-puzzle bundle should NOT be eligible_for_dedup (coin burns value)"
    );

    // dedup_index should be empty — non-eligible bundles are not indexed
    assert_eq!(
        mempool.dedup_index_len(),
        0,
        "dedup_index should be empty for non-eligible bundles"
    );

    // cost_saving should be 0, effective_virtual_cost should equal virtual_cost
    assert_eq!(item.cost_saving, 0);
    assert_eq!(item.effective_virtual_cost, item.virtual_cost);
}

// ──────────────────────────────────────────────────────────────────────────
// Test 4: Cost-bearer removal re-assigns to next waiter
// ──────────────────────────────────────────────────────────────────────────

/// Test: Removing the cost bearer promotes the first waiter to bearer.
///
/// Proves POL-008: "Cost-bearer removal triggers re-assignment."
/// After A (bearer) is removed, B (waiter) becomes the new bearer.
/// A subsequent bundle C with the same key gets cost_saving > 0,
/// confirming B is now the bearer (not that the key was deleted).
#[test]
fn vv_req_pol_008_cost_bearer_removal_reassigns() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit A: 1-spend (coin X) → bearer for key_x
    let (bundle_a, cr_a, coin_x_id, _puzzle_a, _ph) =
        pass_through_bundle(0x01, 100, Program::default());
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Submit B: 2-spend (coin X + coin Y) → waiter for key_x, bearer for key_y
    let (bundle_b, cr_b) = pass_through_bundle_2(0x01, 0x02, 100, Program::default());
    let b_id = bundle_b.name();
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();

    // Verify initial state: A is bearer for key_x
    let solution_hash = sha256_of(Program::default().as_ref());
    assert_eq!(
        mempool.get_dedup_bearer(&coin_x_id, &solution_hash),
        Some(a_id),
        "A should be bearer for key_x before removal"
    );

    // Remove A from the active pool
    assert!(mempool.remove(&a_id), "bundle A should be removable");
    assert_eq!(mempool.len(), 1, "only B should remain after removing A");

    // After removal: B should now be the bearer for key_x
    assert_eq!(
        mempool.get_dedup_bearer(&coin_x_id, &solution_hash),
        Some(b_id),
        "B should become the bearer for key_x after A is removed"
    );

    // Submit C: 2-spend (coin X + coin Z) — different bundle ID from A or B
    // C should find key_x in dedup_index (bearer = B) and get cost_saving > 0
    let (bundle_c, cr_c) = pass_through_bundle_2(0x01, 0x03, 100, Program::default());
    let c_id = bundle_c.name();
    mempool.submit(bundle_c, &cr_c, 0, 0).unwrap();

    let item_c = mempool.get(&c_id).expect("bundle C should be admitted");
    assert!(
        item_c.cost_saving > 0,
        "bundle C should have cost_saving > 0 because B is now bearer for key_x"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Test 5: Feature disabled — no dedup tracking
// ──────────────────────────────────────────────────────────────────────────

/// Test: Disabling `enable_identical_spend_dedup` skips all dedup tracking.
///
/// Proves POL-008: "The feature is gated by enable_identical_spend_dedup."
/// With the feature off, eligible bundles should have cost_saving = 0 and
/// effective_virtual_cost = virtual_cost.
///
/// Note: bundles B and A use non-overlapping coins (coin_x vs. coin_y) so they
/// coexist without triggering CFR conflict detection.  The dedup feature is what
/// would otherwise make a same-spend waiter possible; disabling it means the
/// dedup_index is never populated, regardless of eligibility.
#[test]
fn vv_req_pol_008_feature_disabled() {
    let config = MempoolConfig::default().with_identical_spend_dedup(false);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit A: coin_x (eligible pass-through bundle)
    let (bundle_a, cr_a, coin_x_id, _puzzle, _ph) =
        pass_through_bundle(0x01, 100, Program::default());
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // dedup_index should be empty (feature disabled)
    assert_eq!(
        mempool.dedup_index_len(),
        0,
        "dedup_index should be empty when feature is disabled"
    );

    let item_a = mempool.get(&a_id).unwrap();
    assert_eq!(
        item_a.cost_saving, 0,
        "cost_saving should be 0 when feature disabled"
    );
    assert_eq!(
        item_a.effective_virtual_cost, item_a.virtual_cost,
        "effective_virtual_cost should equal virtual_cost when feature disabled"
    );

    // Submit B: coin_y only (no coin overlap with A — avoids CFR conflict).
    // Even though B is dedup-eligible, the disabled feature means dedup_index
    // is never populated and no cost_saving is recorded.
    let (bundle_b, cr_b, coin_y_id, _puzzle_b, _ph_b) =
        pass_through_bundle(0x02, 100, Program::default());
    let b_id = bundle_b.name();
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();

    let item_b = mempool.get(&b_id).unwrap();
    assert_eq!(
        item_b.cost_saving, 0,
        "B cost_saving should be 0 when feature disabled"
    );
    assert_eq!(item_b.effective_virtual_cost, item_b.virtual_cost);

    // dedup_index remains empty — no bearers registered
    assert_eq!(
        mempool.dedup_index_len(),
        0,
        "dedup_index should remain empty"
    );

    let solution_hash = sha256_of(Program::default().as_ref());
    assert_eq!(
        mempool.get_dedup_bearer(&coin_x_id, &solution_hash),
        None,
        "no bearer for coin_x when feature disabled"
    );
    assert_eq!(
        mempool.get_dedup_bearer(&coin_y_id, &solution_hash),
        None,
        "no bearer for coin_y when feature disabled"
    );
    let _ = b_id; // suppress unused variable warning
}

// ──────────────────────────────────────────────────────────────────────────
// Test 6: Different solutions produce separate dedup keys
// ──────────────────────────────────────────────────────────────────────────

/// Test: Same coin with different solution bytes is NOT treated as a dedup waiter.
///
/// Proves POL-008: "Keys are (coin_id, sha256(solution))."
/// Two bundles spending the same coin with DIFFERENT solution bytes produce
/// different sha256 hashes and therefore different dedup keys.  Because the
/// solutions differ, the incoming bundle is NOT a waiter — it is a real
/// conflict and goes through CFR RBF evaluation (CFR-001).
///
/// This is stronger proof than two coexisting bundles: the dedup bypass
/// specifically checks `pool.dedup_index.contains_key(&(coin_id, sol_hash))`.
/// A different solution yields a different hash, so no entry is found, and
/// the bundle is correctly routed to RBF.
#[test]
fn vv_req_pol_008_different_solutions_not_deduped() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Solution A: Program::default() = nil = empty bytes
    let solution_nil = Program::default();
    // Solution B: CLVM pair `(1 . ())` = bytes [0xff, 0x01, 0x80]
    // This is valid CLVM and produces a different sha256 from the nil solution.
    let solution_other = Program::new(vec![0xff, 0x01, 0x80].into());

    // Verify the two solutions have different sha256 hashes (precondition).
    let hash_nil = sha256_of(solution_nil.as_ref());
    let hash_other = sha256_of(solution_other.as_ref());
    assert_ne!(
        hash_nil, hash_other,
        "different solutions must hash differently"
    );

    // Bundle A: coin_x with solution_nil
    let (bundle_a, cr_a, coin_x_id, _puzzle, _ph) =
        pass_through_bundle(0x01, 100, solution_nil.clone());
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // dedup_index: 1 entry — (coin_x.id, hash_nil) → A
    assert_eq!(
        mempool.dedup_index_len(),
        1,
        "A is bearer for (coin_x, hash_nil)"
    );
    assert_eq!(mempool.get_dedup_bearer(&coin_x_id, &hash_nil), Some(a_id));

    // Bundle B: same coin_x but solution_other.
    // Different hash → dedup_index has no entry for (coin_x, hash_other) → B is NOT
    // a waiter.  CFR conflict detection fires (coin_x is in coin_index → conflict with A).
    // B has the same FPC as A → RbfFpcNotHigher.
    let (bundle_b, cr_b, _cid, _puzzle_b, _ph_b) =
        pass_through_bundle(0x01, 100, solution_other.clone());
    let b_id = bundle_b.name();
    assert_ne!(
        b_id, a_id,
        "different solutions produce different bundle IDs"
    );

    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(dig_mempool::MempoolError::RbfFpcNotHigher)),
        "different solution → real conflict → RBF fails; got: {:?}",
        result
    );

    // Pool unchanged: only A remains
    assert_eq!(mempool.len(), 1, "only A should be in pool");
    assert!(mempool.get(&a_id).is_some());

    // dedup_index unchanged: only A's key registered (B never reached dedup step)
    assert_eq!(
        mempool.dedup_index_len(),
        1,
        "dedup_index should still have only 1 entry"
    );
    // No entry for hash_other (B was rejected before dedup insertion)
    assert_eq!(
        mempool.get_dedup_bearer(&coin_x_id, &hash_other),
        None,
        "B was rejected — no bearer registered for solution_other"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Test 7: Effective cost calculation formula
// ──────────────────────────────────────────────────────────────────────────

/// Test: `effective_virtual_cost = virtual_cost - cost_saving` (spec formula).
///
/// Proves POL-008: the cost_saving and effective_virtual_cost fields are
/// computed consistently per the spec. Bundle B has 2 spends with 1 deduped,
/// so cost_saving = cost/2 and effective_virtual_cost = virtual_cost - cost/2.
#[test]
fn vv_req_pol_008_effective_cost_calculation() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit A: 1-spend (coin X) → bearer
    let (bundle_a, cr_a, _cid, _puzzle, _ph) = pass_through_bundle(0x01, 100, Program::default());
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Submit B: 2-spend (coin X + coin Y) → 1 deduped spend (X)
    let (bundle_b, cr_b) = pass_through_bundle_2(0x01, 0x02, 100, Program::default());
    let b_id = bundle_b.name();
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();

    let item_b = mempool.get(&b_id).expect("B should be in pool");

    // Verify all fields are consistent
    assert_eq!(item_b.num_spends, 2, "bundle B should have 2 spends");

    // cost_saving = cost / num_spends (uniform approximation) * num_deduped (1)
    let expected_cost_saving = item_b.cost / item_b.num_spends as u64;
    assert_eq!(
        item_b.cost_saving, expected_cost_saving,
        "cost_saving should equal cost / num_spends for 1 deduped spend"
    );

    // effective_virtual_cost = virtual_cost - cost_saving
    let expected_effective_vc = item_b.virtual_cost - item_b.cost_saving;
    assert_eq!(
        item_b.effective_virtual_cost, expected_effective_vc,
        "effective_virtual_cost = virtual_cost - cost_saving"
    );

    // effective_virtual_cost should be strictly less than virtual_cost
    assert!(
        item_b.effective_virtual_cost < item_b.virtual_cost,
        "effective cost should be reduced for deduped bundle"
    );

    // The saving should be positive (cost > 0 for a real CLVM bundle)
    assert!(
        item_b.cost_saving > 0,
        "cost_saving should be > 0 for a deduped spend"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Helper: SHA-256 of bytes (mirrors the internal sha256_bytes function)
// ──────────────────────────────────────────────────────────────────────────

/// Compute SHA-256 of the given bytes, returning a Bytes32.
/// Mirrors the `sha256_bytes` function in `src/mempool.rs`.
fn sha256_of(bytes: &[u8]) -> Bytes32 {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(bytes);
    let array: [u8; 32] = hash.into();
    Bytes32::from(array)
}
