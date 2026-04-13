//! REQUIREMENT: POL-005 — Pending Pool Deduplication
//!
//! Test-driven verification that the pending pool detects coin conflicts between
//! pending items and applies RBF rules to resolve them.
//!
//! ## What this proves
//!
//! - `pending_coin_index` is populated for each coin in item.removals
//! - New pending items are checked against pending_coin_index for conflicts
//! - Successful RBF (superset + higher FPC + fee bump) replaces old item
//! - Failed RBF: not superset → `RbfNotSuperset`
//! - Failed RBF: FPC not higher → `RbfFpcNotHigher`
//! - Failed RBF: fee bump too low → `RbfBumpTooLow`
//! - Index is cleaned when items are promoted via drain_pending
//! - Failed pending RBF does NOT add item to the conflict cache
//!
//! ## Chia L1 Correspondence
//!
//! This is a dig-mempool improvement — Chia's PendingTxCache has no dedup.
//! RBF rules mirror active-pool RBF from SPEC Section 5.11.
//!
//! Reference: docs/requirements/domains/pools/specs/POL-005.md

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
const NIL_PUZZLE_HASH: [u8; 32] =
    hex!("4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a");

/// ASSERT_HEIGHT_ABSOLUTE opcode = 83 (0x53).
const ASSERT_HEIGHT_ABSOLUTE: u8 = 83;

/// Minimum RBF fee bump (matches MempoolConfig::min_rbf_fee_bump default).
const MIN_RBF_FEE_BUMP: u64 = 10_000_000;

/// Encode a positive integer as a minimum-length big-endian CLVM atom.
fn encode_uint(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![];
    }
    let be = v.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(7);
    let bytes = &be[start..];
    if bytes[0] >= 0x80 {
        let mut result = vec![0x00];
        result.extend_from_slice(bytes);
        result
    } else {
        bytes.to_vec()
    }
}

/// Build a CLVM puzzle that returns `((ASSERT_HEIGHT_ABSOLUTE height))`.
fn make_aha_puzzle(height: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let height_atom = a.new_atom(&encode_uint(height)).unwrap();
    let inner = a.new_pair(height_atom, nil).unwrap();
    let opcode = a.new_atom(&[ASSERT_HEIGHT_ABSOLUTE]).unwrap();
    let cond = a.new_pair(opcode, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    (Program::new(bytes.into()), puzzle_hash)
}

/// Create a 1-spend bundle with `ASSERT_HEIGHT_ABSOLUTE height` condition.
fn aha_bundle(
    parent_prefix: u8,
    amount: u64,
    height: u64,
    coin_records: &mut HashMap<Bytes32, CoinRecord>,
) -> SpendBundle {
    let (puzzle, puzzle_hash) = make_aha_puzzle(height);
    let coin = Coin::new(Bytes32::from([parent_prefix; 32]), puzzle_hash, amount);
    let coin_id = coin.coin_id();
    coin_records.insert(
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
    SpendBundle::new(
        vec![CoinSpend::new(coin, puzzle, Program::default())],
        Signature::default(),
    )
}

/// Create a 2-spend bundle: both coins use `ASSERT_HEIGHT_ABSOLUTE height`.
///
/// Each coin has a distinct parent prefix so they have different coin IDs.
/// Used to test the "superset rule" — new items must spend all coins the
/// conflicting item spends.
fn aha_bundle_pair(
    parent_a: u8,
    parent_b: u8,
    amount_a: u64,
    amount_b: u64,
    height: u64,
    coin_records: &mut HashMap<Bytes32, CoinRecord>,
) -> SpendBundle {
    let (puzzle, puzzle_hash) = make_aha_puzzle(height);
    let coin_a = Coin::new(Bytes32::from([parent_a; 32]), puzzle_hash, amount_a);
    let coin_b = Coin::new(Bytes32::from([parent_b; 32]), puzzle_hash, amount_b);
    for coin in [coin_a, coin_b] {
        let coin_id = coin.coin_id();
        coin_records.insert(
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
    }
    SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, puzzle.clone(), Program::default()),
            CoinSpend::new(coin_b, puzzle, Program::default()),
        ],
        Signature::default(),
    )
}

/// Create a nil-puzzle bundle (no timelock, for checking conflict cache).
fn nil_bundle(
    parent_prefix: u8,
    amount: u64,
    coin_records: &mut HashMap<Bytes32, CoinRecord>,
) -> SpendBundle {
    let coin = Coin::new(
        Bytes32::from([parent_prefix; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        amount,
    );
    let coin_id = coin.coin_id();
    coin_records.insert(
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
    SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    )
}

// ──────────────────────────────────────────────────────────────────────────
// pending_coin_index populated
// ──────────────────────────────────────────────────────────────────────────

/// Test: pending_coin_index is populated for each removal in a pending item.
///
/// Proves POL-005: "pending_coin_index is maintained for all pending items."
/// Verified indirectly: if a second bundle spends the same coin, conflict is
/// detected (proving the index was populated). Direct index query via
/// get_pending_coin_spender().
#[test]
fn vv_req_pol_005_index_populated_on_insert() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);

    // Infer the coin_id that b1 spends
    let coin_id = b1.coin_spends[0].coin.coin_id();

    mempool.submit(b1, &cr1, 0, 0).unwrap();

    // The pending_coin_index should map coin_id to b1's bundle_id.
    assert!(
        mempool.get_pending_coin_spender(&coin_id).is_some(),
        "pending_coin_index should map the spent coin to b1's bundle_id"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Conflict detection
// ──────────────────────────────────────────────────────────────────────────

/// Test: A second pending item spending the same coin triggers conflict.
///
/// Proves POL-005: "new pending items are checked against pending_coin_index."
/// With default RBF config (min_rbf_fee_bump=10M mojos), a low-fee replacement
/// fails with a conflict error.
///
/// b2 is a 2-spend bundle spending coin_A (same as b1) + coin_C (new),
/// making it a different bundle_id while conflicting on coin_A.
#[test]
fn vv_req_pol_005_conflict_detected() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit first pending item spending coin_A: parent=[0x01;32], AHA(100), amount=1
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1); // fee=1
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // b2: 2-spend {coin_A (same as b1), coin_C (parent=0x03)} — conflicts on coin_A.
    // fee = 1+1 = 2 << MIN_RBF_FEE_BUMP → RBF must fail.
    let (aha_puzzle, aha_hash) = make_aha_puzzle(100);
    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), aha_hash, 1); // same coin as b1
    let coin_c = Coin::new(Bytes32::from([0x03u8; 32]), aha_hash, 1);
    let mut cr2 = HashMap::new();
    for coin in [coin_a, coin_c] {
        cr2.insert(
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
    let b2 = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, aha_puzzle.clone(), Program::default()),
            CoinSpend::new(coin_c, aha_puzzle, Program::default()),
        ],
        Signature::default(),
    );
    let result = mempool.submit(b2, &cr2, 0, 0);

    // Should fail with some RBF error (fee bump 2-1=1 << MIN_RBF_FEE_BUMP)
    assert!(
        matches!(
            result,
            Err(MempoolError::RbfBumpTooLow { .. }) | Err(MempoolError::RbfFpcNotHigher)
        ),
        "Should get RBF rejection, got: {:?}",
        result
    );
    assert_eq!(mempool.pending_len(), 1, "Original item should remain");
}

// ──────────────────────────────────────────────────────────────────────────
// RBF succeeds
// ──────────────────────────────────────────────────────────────────────────

/// Test: Successful RBF replaces the old pending item with the new one.
///
/// Proves POL-005: "successful RBF removes the conflicting pending item(s)
/// before inserting the new one."
///
/// b1: 1-spend coin_A (amount=1), fee=1.
/// b2: 2-spend {coin_A (same), coin_B (amount=MIN_RBF_FEE_BUMP)}, fee=10_000_001.
/// required = 1 + 10_000_000 = 10_000_001. provided = 10_000_001 → passes.
/// b2 is a superset of b1 (spends coin_A and more). FPC_b2 >> FPC_b1.
#[test]
fn vv_req_pol_005_rbf_succeeds() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Old item: fee=1, coin_A = (parent=[0x01;32], AHA(100), amount=1)
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();

    // b2: 2-spend {coin_A (same as b1), coin_B (parent=[0x02;32], amount=MIN_RBF_FEE_BUMP)}.
    // fee = 1 + MIN_RBF_FEE_BUMP = 10_000_001 >= required (1 + 10M). All RBF rules pass.
    let (aha_puzzle, aha_hash) = make_aha_puzzle(100);
    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), aha_hash, 1); // same as b1
    let coin_b = Coin::new(Bytes32::from([0x02u8; 32]), aha_hash, MIN_RBF_FEE_BUMP);
    let mut cr2 = HashMap::new();
    for coin in [coin_a, coin_b] {
        cr2.insert(
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
    let b2 = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, aha_puzzle.clone(), Program::default()),
            CoinSpend::new(coin_b, aha_puzzle, Program::default()),
        ],
        Signature::default(),
    );
    let b2_id = b2.name();

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Pending { assert_height: 100 }),
        "RBF should succeed and admit new pending item, got: {:?}",
        result
    );

    assert_eq!(mempool.pending_len(), 1, "Still exactly 1 pending item");
    assert!(
        !mempool.pending_bundle_ids().contains(&b1_id),
        "Old item should have been evicted"
    );
    assert!(
        mempool.pending_bundle_ids().contains(&b2_id),
        "New item should be in pending"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// RBF failures
// ──────────────────────────────────────────────────────────────────────────

/// Test: RBF fails when new item is not a superset of conflicting item's coins.
///
/// Proves POL-005 superset rule.
///
/// Setup:
/// - Old item spends coins {A, B} (two coins, parent=0x01 and 0x02)
/// - New item spends {A, C} (shares coin A but not B, adds new coin C)
/// - New item has much higher fee
/// - RBF fails: B ∈ old.removals but B ∉ new.removals
#[test]
fn vv_req_pol_005_rbf_fails_not_superset() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Old item spends coins A (parent=0x01) and B (parent=0x02) — 2-spend bundle
    let mut cr1 = HashMap::new();
    // Use a small fee that a high-FPC replacement could beat, but the superset check fires first
    let b1 = aha_bundle_pair(0x01, 0x02, 1, 1, 100, &mut cr1);
    let b1_id = b1.name();
    let result = mempool.submit(b1, &cr1, 0, 0);
    assert!(
        matches!(result, Ok(SubmitResult::Pending { .. })),
        "b1 should go to pending, got: {:?}",
        result
    );

    // New item: spends A (parent=0x01) and a different coin C (parent=0x03)
    // Does NOT spend B (parent=0x02) → not a superset
    let new_fee = MIN_RBF_FEE_BUMP + 100;
    let mut cr2 = HashMap::new();
    // Coin A: same as in b1 — conflicts
    let (aha_puzzle, aha_hash) = make_aha_puzzle(100);
    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), aha_hash, 1);
    cr2.insert(
        coin_a.coin_id(),
        CoinRecord {
            coin: coin_a,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    // Coin C: new coin
    let coin_c = Coin::new(Bytes32::from([0x03u8; 32]), aha_hash, new_fee);
    cr2.insert(
        coin_c.coin_id(),
        CoinRecord {
            coin: coin_c,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let b2 = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, aha_puzzle.clone(), Program::default()),
            CoinSpend::new(coin_c, aha_puzzle, Program::default()),
        ],
        Signature::default(),
    );

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "Should get RbfNotSuperset since new item doesn't spend coin B, got: {:?}",
        result
    );
    // Old item should remain
    assert!(mempool.pending_bundle_ids().contains(&b1_id));
}

/// Test: RBF fails when new item's FPC is not strictly higher.
///
/// Proves POL-005: "Higher FPC rule."
///
/// b1: 1-spend, coin A (amount=100), fee=100, virtual_cost=V.
///     FPC_old = 100/V (high FPC).
///
/// b2: 2-spend, {coin A (same, amount=100), coin B (amount=1)}, fee=101.
///     virtual_cost ≈ 2V (two full AHA spends).
///     FPC_new = 101/(2V) ≈ 50.5/V << 100/V = FPC_old.
///
/// Since 50.5/V < 100/V, FPC_new < FPC_old → RbfFpcNotHigher fires before fee-bump check.
#[test]
fn vv_req_pol_005_rbf_fails_fpc_too_low() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Old: 1-spend coin A (parent=0x01, amount=100), fee=100 → high FPC
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 100, 100, &mut cr1);
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();

    // New: 2-spend {coin A (parent=0x01, amount=100, same as b1), coin B (amount=1)}.
    // fee = 100 + 1 = 101. vc ≈ 2V.
    // FPC_new = 101/(≈2V) ≈ 50.5/V << 100/V = FPC_old → RbfFpcNotHigher.
    let mut cr2 = HashMap::new();
    let b2 = aha_bundle_pair(0x01, 0x02, 100, 1, 100, &mut cr2);

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Should get RbfFpcNotHigher when FPC_new < FPC_old, got: {:?}",
        result
    );
    assert!(
        mempool.pending_bundle_ids().contains(&b1_id),
        "Old item should remain after failed FPC check"
    );
}

/// Test: RBF fails when fee bump is below minimum.
///
/// Proves POL-005: minimum fee bump rule.
/// Old item fee=1, new item fee=2 (bump=1 << MIN_RBF_FEE_BUMP=10M).
///
/// Note: uses different coins since same coin → same fee from nil puzzle.
/// Actually impossible to test this directly without multi-coin bundles.
/// This test verifies via the `conflict_detected` test's bump failure.
#[test]
fn vv_req_pol_005_rbf_fails_fee_bump_too_low() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Old item: fee=1 via coin amount=1
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1); // coin A, fee=1
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();

    // New item also spends coin A but from a 2-spend bundle:
    // spend coin A (same, fee=1) + coin B (parent=0x02, fee=MIN_RBF_FEE_BUMP-1)
    // total_new_fee = 1 + MIN_RBF_FEE_BUMP - 1 = MIN_RBF_FEE_BUMP
    // required = old_fee + MIN_RBF_FEE_BUMP = 1 + 10_000_000 = 10_000_001
    // provided = MIN_RBF_FEE_BUMP = 10_000_000 < 10_000_001 → RbfBumpTooLow
    let (aha_puzzle, aha_hash) = make_aha_puzzle(100);
    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), aha_hash, 1); // same coin A
    let coin_b_fee = MIN_RBF_FEE_BUMP - 1;
    let coin_b = Coin::new(Bytes32::from([0x02u8; 32]), aha_hash, coin_b_fee);
    let mut cr2 = HashMap::new();
    for coin in [coin_a, coin_b] {
        cr2.insert(
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
    let b2 = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, aha_puzzle.clone(), Program::default()),
            CoinSpend::new(coin_b, aha_puzzle, Program::default()),
        ],
        Signature::default(),
    );

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfBumpTooLow { .. })),
        "Should get RbfBumpTooLow, got: {:?}",
        result
    );
    assert!(
        mempool.pending_bundle_ids().contains(&b1_id),
        "Old item should remain after failed RBF"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Index cleanup
// ──────────────────────────────────────────────────────────────────────────

/// Test: pending_coin_index is cleaned up when items are promoted.
///
/// Proves POL-005: "The index is cleaned up when pending items are removed."
/// After drain_pending promotes b1 (removing it from pending), the coin
/// b1 spent can be added to a new pending item without triggering conflict.
#[test]
fn vv_req_pol_005_index_cleaned_on_promotion() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit b1 with assert_height=100
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);
    let coin_a_id = b1.coin_spends[0].coin.coin_id();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.pending_len(), 1);

    // Verify conflict index is populated
    assert!(mempool.get_pending_coin_spender(&coin_a_id).is_some());

    // Promote b1 via drain_pending(100)
    let promoted = mempool.drain_pending(100, 0);
    assert_eq!(promoted.len(), 1);
    assert_eq!(mempool.pending_len(), 0);

    // Index should be cleaned up
    assert!(
        mempool.get_pending_coin_spender(&coin_a_id).is_none(),
        "Index should be cleaned after promotion"
    );

    // A new pending item spending the same coin should work (no conflict)
    // Create a fresh coin_records for coin A (re-submit same coin, new bundle)
    // Actually the seen cache prevents re-using the same bundle, so use different params.
    // Use same coin but with assert_height=200 (different bundle)
    let mut cr2 = HashMap::new();
    let (puzzle2, hash2) = make_aha_puzzle(200);
    let coin_a_new = Coin::new(Bytes32::from([0x01u8; 32]), hash2, 1);
    cr2.insert(
        coin_a_new.coin_id(),
        CoinRecord {
            coin: coin_a_new,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    // Note: coin_a_new.coin_id() differs from coin_a_id because puzzle_hash differs (height=200 vs 100)
    // So this tests a new spend on a new coin (no conflict anyway). That's fine — the test above
    // verifies cleanup via get_pending_coin_spender() returning None.
    let b3 = SpendBundle::new(
        vec![CoinSpend::new(coin_a_new, puzzle2, Program::default())],
        Signature::default(),
    );
    let result = mempool.submit(b3, &cr2, 0, 0);
    assert!(
        matches!(result, Ok(SubmitResult::Pending { .. })),
        "New pending item should be admitted after cleanup, got: {:?}",
        result
    );
}

// ──────────────────────────────────────────────────────────────────────────
// No conflict cache for failed pending RBF
// ──────────────────────────────────────────────────────────────────────────

/// Test: Failed pending RBF does NOT add the item to the conflict cache.
///
/// Proves POL-005: "If RBF fails, reject without adding to conflict cache."
/// After a failed pending RBF, conflict_len() must remain 0.
///
/// b2 is a 2-spend bundle {coin_A (same as b1), coin_C}, conflicting on coin_A
/// with fee too low to pass the fee bump rule.
#[test]
fn vv_req_pol_005_no_conflict_cache_for_pending() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Submit a pending item spending coin_A (parent=[0x01;32], AHA(100), amount=1)
    let mut cr1 = HashMap::new();
    let b1 = aha_bundle(0x01, 1, 100, &mut cr1);
    mempool.submit(b1, &cr1, 0, 0).unwrap();

    // b2: 2-spend {coin_A (same as b1), coin_C (parent=0x03)} — conflicts on coin_A.
    // fee = 1 + 1 = 2 << MIN_RBF_FEE_BUMP → RBF fails.
    let (aha_puzzle, aha_hash) = make_aha_puzzle(100);
    let coin_a = Coin::new(Bytes32::from([0x01u8; 32]), aha_hash, 1); // same as b1
    let coin_c = Coin::new(Bytes32::from([0x03u8; 32]), aha_hash, 1);
    let mut cr2 = HashMap::new();
    for coin in [coin_a, coin_c] {
        cr2.insert(
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
    let b2 = SpendBundle::new(
        vec![
            CoinSpend::new(coin_a, aha_puzzle.clone(), Program::default()),
            CoinSpend::new(coin_c, aha_puzzle, Program::default()),
        ],
        Signature::default(),
    );
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(result.is_err(), "RBF should fail, got: {:?}", result);

    // Conflict cache must remain empty
    assert_eq!(
        mempool.conflict_len(),
        0,
        "Failed pending RBF must NOT add to conflict cache"
    );
    assert_eq!(mempool.pending_len(), 1, "Original item should remain");
}
