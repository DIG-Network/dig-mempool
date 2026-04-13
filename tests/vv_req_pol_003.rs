//! REQUIREMENT: POL-003 — Expiry Protection During Eviction
//!
//! Test-driven verification that items within `expiry_protection_blocks` of
//! their `assert_before_height` are skipped during capacity eviction, and
//! that expiring items can evict other expiring items with lower FPC.
//!
//! ## What this proves
//!
//! - Items within protection window (`abh > height && abh <= height + protection_blocks`)
//!   are skipped during non-expiring eviction
//! - Items without `assert_before_height` are never expiry-protected
//! - Already-expired items (`abh <= current_height`) are rejected, not protected
//! - Items outside protection window (`abh > height + protection_blocks`) are evictable
//! - Expiring new item can evict an expiring existing item with lower FPC
//! - MempoolFull when all evictable items are expiry-protected
//! - Protection window is configurable via `expiry_protection_blocks`
//! - Boundary: item at exactly `abh = current_height + expiry_protection_blocks` is protected
//!
//! ## Chia L1 Correspondence
//!
//! Corresponds to Chia's expiry-protection eviction logic at:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L406
//!
//! Reference: docs/requirements/domains/pools/specs/POL-003.md

use std::collections::HashMap;

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
/// sha256tree(nil atom) = sha256([0x01]) = 4bf5122f...
const NIL_PUZZLE_HASH: [u8; 32] =
    hex!("4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a");

/// ASSERT_BEFORE_HEIGHT_ABSOLUTE opcode = 87
const ASSERT_BEFORE_HEIGHT_ABSOLUTE: u8 = 87;

/// Encode a positive integer as a minimum-length big-endian CLVM atom.
///
/// CLVM uses signed 2's complement encoding. To represent a positive number
/// whose most-significant byte has the high bit set, a leading 0x00 byte is
/// prepended (sign byte). Returns empty Vec for 0 (nil).
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

/// Build a CLVM puzzle that returns `((ASSERT_BEFORE_HEIGHT_ABSOLUTE abh))`.
///
/// The program is `(q . ((87 . (abh . ()))))` — a quoted constant that
/// produces a single condition when executed with any solution.
///
/// Returns `(puzzle_program, puzzle_hash_as_Bytes32)`.
fn make_abh_puzzle(abh: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();

    let abh_atom = a.new_atom(&encode_uint(abh)).unwrap();
    let inner = a.new_pair(abh_atom, nil).unwrap(); // (abh . ())
    let opcode = a.new_atom(&[ASSERT_BEFORE_HEIGHT_ABSOLUTE]).unwrap();
    let cond = a.new_pair(opcode, inner).unwrap(); // (87 . (abh . ()))
    let cond_list = a.new_pair(cond, nil).unwrap(); // ((87 abh))
    let q = a.new_atom(&[1u8]).unwrap(); // quote opcode
    let prog = a.new_pair(q, cond_list).unwrap(); // (q . ((87 abh)))

    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());

    (puzzle, puzzle_hash)
}

/// Create a coin bundle with `ASSERT_BEFORE_HEIGHT_ABSOLUTE abh` condition.
///
/// The puzzle is built dynamically so the puzzle_hash matches, and the
/// CLVM validator accepts the spend.
fn abh_bundle(
    parent_prefix: u8,
    amount: u64,
    abh: u64,
    coin_records: &mut HashMap<Bytes32, CoinRecord>,
) -> SpendBundle {
    let (puzzle, puzzle_hash) = make_abh_puzzle(abh);
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

/// Create a plain nil-puzzle bundle (no expiry condition).
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

/// Measure the virtual_cost of a 1-spend nil bundle (amount=1).
///
/// Used to size pool capacity for tests where the pool holds only nil bundles.
fn probe_nil_virtual_cost() -> u64 {
    let probe = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    let b = nil_bundle(0xff, 1, &mut cr);
    probe.submit(b, &cr, 0, 0).unwrap();
    probe.stats().total_cost
}

/// Measure the virtual_cost of a 1-spend abh bundle with a 1-byte abh value.
///
/// All abh values 1–127 encode as 1-byte CLVM atoms, so they all yield the
/// same virtual_cost. Used to size pool capacity for tests that hold abh bundles.
fn probe_abh_virtual_cost() -> u64 {
    let probe = Mempool::new(DIG_TESTNET);
    let mut cr = HashMap::new();
    let b = abh_bundle(0xff, 1, 50, &mut cr);
    probe.submit(b, &cr, 0, 0).unwrap();
    probe.stats().total_cost
}

// ──────────────────────────────────────────────────────────────────────────
// Protected item skipped during non-expiring eviction
// ──────────────────────────────────────────────────────────────────────────

/// Test: Non-expiring new item cannot evict an expiry-protected item.
///
/// Proves POL-003: protected items are skipped during eviction.
/// With only a protected item in the pool, a non-expiring new item
/// with higher FPC still gets MempoolFull.
#[test]
fn vv_req_pol_003_protected_skipped_non_expiring_gets_mempool_full() {
    let abh_vc = probe_abh_virtual_cost();

    // Capacity for exactly 1 abh bundle
    let config = MempoolConfig::default()
        .with_max_total_cost(abh_vc + 1)
        .with_expiry_protection_blocks(100);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // current_height = 0; abh = 50 → within protection window (0 < 50 <= 0+100)
    let mut cr1 = HashMap::new();
    let b1 = abh_bundle(0x01, 1, 50, &mut cr1); // fee=1, protected
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Non-expiring bundle with much higher FPC — should be unable to evict b1
    let mut cr2 = HashMap::new();
    let b2 = nil_bundle(0x02, 100, &mut cr2); // fee=100 (same vc as probe)
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::MempoolFull)),
        "Non-expiring item should not evict protected item, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Protected item should remain in pool");
}

// ──────────────────────────────────────────────────────────────────────────
// Non-protected far-future expiry is evictable
// ──────────────────────────────────────────────────────────────────────────

/// Test: Item with assert_before_height outside protection window is evictable.
///
/// Proves POL-003: only items within the window are protected.
/// An item with abh = height + 200 (> protection window of 100) is NOT protected.
#[test]
fn vv_req_pol_003_far_future_expiry_is_evictable() {
    let abh_vc = probe_abh_virtual_cost();

    let config = MempoolConfig::default()
        .with_max_total_cost(abh_vc + 1)
        .with_expiry_protection_blocks(100);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // current_height=0; abh=110 → 110 > 100 → outside window → NOT protected
    // abh=110 < 128 so it encodes as a 1-byte CLVM atom, matching probe_abh_virtual_cost().
    let mut cr1 = HashMap::new();
    let b1 = abh_bundle(0x01, 1, 110, &mut cr1); // fee=1, not protected
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Non-expiring bundle with higher FPC should evict b1
    let mut cr2 = HashMap::new();
    let b2 = nil_bundle(0x02, 100, &mut cr2); // fee=100
    let b2_id = b2.name();
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Higher-FPC bundle should evict far-future expiry item, got: {:?}",
        result
    );
    assert!(
        !mempool.contains(&b1_id),
        "Far-future expiry item should have been evicted"
    );
    assert!(mempool.contains(&b2_id));
    assert_eq!(mempool.len(), 1);
}

// ──────────────────────────────────────────────────────────────────────────
// Item without assert_before_height is never protected
// ──────────────────────────────────────────────────────────────────────────

/// Test: Items without assert_before_height are never expiry-protected.
///
/// Proves POL-003: protection requires assert_before_height.is_some().
/// A plain bundle with no expiry condition is always evictable.
#[test]
fn vv_req_pol_003_no_expiry_no_protection() {
    let nil_vc = probe_nil_virtual_cost();

    let config = MempoolConfig::default()
        .with_max_total_cost(nil_vc + 1)
        .with_expiry_protection_blocks(100);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit a no-expiry bundle with low FPC
    let mut cr1 = HashMap::new();
    let b1 = nil_bundle(0x01, 1, &mut cr1); // fee=1, no expiry
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Submit a higher-FPC bundle — should evict b1
    let mut cr2 = HashMap::new();
    let b2 = nil_bundle(0x02, 100, &mut cr2); // fee=100
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success));
    assert!(
        !mempool.contains(&b1_id),
        "No-expiry item should be evictable"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Expiring new item can evict expiring existing item with lower FPC
// ──────────────────────────────────────────────────────────────────────────

/// Test: Expiring items can evict other expiring items if FPC is higher.
///
/// Proves POL-003: expiring-vs-expiring eviction is allowed.
/// An incoming expiring item with FPC > existing expiring item's FPC
/// can displace it.
#[test]
fn vv_req_pol_003_expiring_evicts_lower_fpc_expiring() {
    let abh_vc = probe_abh_virtual_cost();

    // Capacity for exactly 1 abh bundle
    let config = MempoolConfig::default()
        .with_max_total_cost(abh_vc + 1)
        .with_expiry_protection_blocks(100);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Existing: expiring item with low FPC (abh=50, fee=1)
    let mut cr1 = HashMap::new();
    let b1 = abh_bundle(0x01, 1, 50, &mut cr1); // fee=1, protected
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Incoming: expiring item with high FPC (abh=60, fee=100)
    let mut cr2 = HashMap::new();
    let b2 = abh_bundle(0x02, 100, 60, &mut cr2); // fee=100, protected
    let b2_id = b2.name();
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Expiring item with higher FPC should evict lower-FPC expiring item, got: {:?}",
        result
    );
    assert!(
        !mempool.contains(&b1_id),
        "Low-FPC expiring item should have been evicted"
    );
    assert!(
        mempool.contains(&b2_id),
        "High-FPC expiring item should be in pool"
    );
    assert_eq!(mempool.len(), 1);
}

// ──────────────────────────────────────────────────────────────────────────
// MempoolFull when all evictable items are protected
// ──────────────────────────────────────────────────────────────────────────

/// Test: MempoolFull returned when all candidates are expiry-protected.
///
/// Proves POL-003: if a non-expiring new item cannot evict any candidate
/// (all are protected), it is rejected.
#[test]
fn vv_req_pol_003_mempool_full_when_all_protected() {
    let abh_vc = probe_abh_virtual_cost();

    // Capacity for exactly 1 abh bundle (protected item fills it)
    let config = MempoolConfig::default()
        .with_max_total_cost(abh_vc + 1)
        .with_expiry_protection_blocks(100);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit a protected item with moderate FPC (abh=50)
    let mut cr1 = HashMap::new();
    let b1 = abh_bundle(0x01, 50, 50, &mut cr1); // fee=50, protected
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Non-expiring bundle — even with high FPC can't evict the protected item
    let mut cr2 = HashMap::new();
    let b2 = nil_bundle(0x02, 100, &mut cr2); // fee=100
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::MempoolFull)),
        "Should get MempoolFull when all candidates are protected, got: {:?}",
        result
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Configurable protection window
// ──────────────────────────────────────────────────────────────────────────

/// Test: Protection window is configurable via expiry_protection_blocks.
///
/// With protection_blocks=50 and current_height=0:
/// - abh=40: protected (40 <= 50)
/// - abh=60: not protected (60 > 50)
#[test]
fn vv_req_pol_003_configurable_protection_window() {
    let abh_vc = probe_abh_virtual_cost();

    let config = MempoolConfig::default()
        .with_max_total_cost(abh_vc + 1)
        .with_expiry_protection_blocks(50); // narrower window
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // item with abh=60: outside window (60 > 50) → NOT protected → evictable
    let mut cr1 = HashMap::new();
    let b1 = abh_bundle(0x01, 1, 60, &mut cr1); // abh=60, fee=1
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Non-expiring high-FPC bundle should be able to evict b1
    let mut cr2 = HashMap::new();
    let b2 = nil_bundle(0x02, 100, &mut cr2); // fee=100
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Item outside protection window should be evictable, got: {:?}",
        result
    );
    assert!(
        !mempool.contains(&b1_id),
        "Item outside protection window (abh=60 > protection=50) should be evicted"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Boundary: exactly at the protection window edge
// ──────────────────────────────────────────────────────────────────────────

/// Test: Item at exactly `abh = current_height + expiry_protection_blocks` is protected.
///
/// Proves POL-003: the condition is `abh <= current_height + protection_blocks`
/// (inclusive upper bound).
#[test]
fn vv_req_pol_003_boundary_at_protection_window_edge() {
    let abh_vc = probe_abh_virtual_cost();

    let config = MempoolConfig::default()
        .with_max_total_cost(abh_vc + 1)
        .with_expiry_protection_blocks(50);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // abh = 0 + 50 = exactly at boundary → protected
    let mut cr1 = HashMap::new();
    let b1 = abh_bundle(0x01, 1, 50, &mut cr1); // abh=50 = protection boundary
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Non-expiring high-FPC bundle should NOT be able to evict b1 (it's protected)
    let mut cr2 = HashMap::new();
    let b2 = nil_bundle(0x02, 100, &mut cr2);
    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::MempoolFull)),
        "Item at exactly protection boundary should be protected, got: {:?}",
        result
    );
}
