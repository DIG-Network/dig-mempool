//! REQUIREMENT: POL-002 — Active Pool Capacity Management
//!
//! Test-driven verification that submit() enforces max_total_cost by evicting
//! lowest-descendant_score items when the pool is full.
//!
//! ## What this proves
//!
//! - No eviction when pool has space (happy path)
//! - Lowest-score item is evicted when new high-FPC item arrives
//! - MempoolFull returned when new item FPC <= all evictable items' scores
//! - TooManySpends returned when spend count exceeds max_spends_per_block
//! - Only evict the minimum needed (first sufficient eviction stops loop)
//! - Empty pool returns MempoolFull when max_total_cost is too small
//!
//! ## Chia L1 Correspondence
//!
//! Corresponds to Chia's `add_to_pool()` eviction at:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L395
//!
//! Reference: docs/requirements/domains/pools/specs/POL-002.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};

/// SHA-256 tree hash of the nil CLVM atom (Program::default() = [0x80]).
/// sha256tree(nil atom) = sha256([0x01]) = 4bf5122f...
/// Required so coin puzzle_hash matches Program::default().
const NIL_PUZZLE_HASH: [u8; 32] = [
    0x4b, 0xf5, 0x12, 0x2f, 0x34, 0x45, 0x54, 0xc5, 0x3b, 0xde, 0x2e, 0xbb, 0x8c, 0xd2, 0xb7, 0xe3,
    0xd1, 0x60, 0x0a, 0xd6, 0x31, 0xc3, 0x85, 0xa5, 0xd7, 0xcc, 0xe2, 0x3c, 0x77, 0x85, 0x45, 0x9a,
];

/// Create a bundle spending the given coin (nil puzzle/solution), populating coin_records.
fn coin_bundle(coin: Coin, coin_records: &mut HashMap<Bytes32, CoinRecord>) -> SpendBundle {
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

/// Measure the virtual_cost of one nil-coin bundle by submitting to a probe mempool.
///
/// Uses a bundle spending coin with amount=1 (fee=1 mojos) to get the canonical
/// virtual_cost for a 1-spend nil-program bundle.
fn probe_bundle_virtual_cost() -> u64 {
    let probe = Mempool::new(DIG_TESTNET);
    let coin = Coin::new(
        Bytes32::from([0xffu8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        1,
    );
    let mut cr = HashMap::new();
    let b = coin_bundle(coin, &mut cr);
    probe.submit(b, &cr, 0, 0).unwrap();
    probe.stats().total_cost
}

// ──────────────────────────────────────────────────────────────────────────
// Happy path: pool has space
// ──────────────────────────────────────────────────────────────────────────

/// Test: No eviction when pool has sufficient space.
///
/// Proves POL-002: eviction is NOT triggered when
/// `total_cost + new_item.virtual_cost <= max_total_cost`.
#[test]
fn vv_req_pol_002_no_eviction_when_space_available() {
    let mempool = Mempool::new(DIG_TESTNET); // default: 8.25T capacity

    // Submit two different bundles — plenty of space for both
    let coin1 = Coin::new(
        Bytes32::from([1u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100,
    );
    let coin2 = Coin::new(
        Bytes32::from([2u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        200,
    );
    let mut cr1 = HashMap::new();
    let mut cr2 = HashMap::new();
    let b1 = coin_bundle(coin1, &mut cr1);
    let b2 = coin_bundle(coin2, &mut cr2);

    let r1 = mempool.submit(b1, &cr1, 0, 0);
    let r2 = mempool.submit(b2, &cr2, 0, 0);

    assert_eq!(r1, Ok(SubmitResult::Success));
    assert_eq!(r2, Ok(SubmitResult::Success));
    assert_eq!(mempool.len(), 2, "Both items should be in pool");
}

// ──────────────────────────────────────────────────────────────────────────
// Eviction: lowest score removed
// ──────────────────────────────────────────────────────────────────────────

/// Test: When pool is full, lowest-descendant_score item is evicted first.
///
/// Proves POL-002: "Items are evicted in ascending descendant_score order."
///
/// Setup: capacity for exactly 1 bundle. Submit bundle with fee=1 (low FPC).
/// Then submit bundle with fee=100 (high FPC) — triggers eviction of the
/// low-FPC bundle. Both use 1-byte CLVM amounts so virtual_costs are equal.
#[test]
fn vv_req_pol_002_lowest_score_evicted_first() {
    let bvc = probe_bundle_virtual_cost();

    // Capacity for exactly 1 bundle (can't fit 2)
    let config = MempoolConfig::default().with_max_total_cost(bvc + 1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle 1: fee=1 (very low FPC)
    let coin1 = Coin::new(Bytes32::from([1u8; 32]), Bytes32::from(NIL_PUZZLE_HASH), 1);
    let mut cr1 = HashMap::new();
    let b1 = coin_bundle(coin1, &mut cr1);
    let b1_id = b1.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Bundle 2: fee=100 (very high FPC relative to b1) — triggers eviction.
    // Amount=100 fits in 1 CLVM byte (same as amount=1), so virtual_cost equals bvc.
    let coin2 = Coin::new(
        Bytes32::from([2u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100,
    );
    let mut cr2 = HashMap::new();
    let b2 = coin_bundle(coin2, &mut cr2);
    let b2_id = b2.name();

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "High-FPC bundle should be admitted via eviction"
    );

    // Low-FPC bundle should have been evicted
    assert!(
        !mempool.contains(&b1_id),
        "Low-FPC bundle should have been evicted"
    );
    // High-FPC bundle should now be in the pool
    assert!(
        mempool.contains(&b2_id),
        "High-FPC bundle should be in the active pool"
    );
    assert_eq!(mempool.len(), 1);
}

// ──────────────────────────────────────────────────────────────────────────
// MempoolFull: new item can't beat existing items
// ──────────────────────────────────────────────────────────────────────────

/// Test: MempoolFull when new item FPC <= lowest evictable score.
///
/// Proves POL-002: "If the new item's FPC <= candidate.descendant_score,
/// reject with MempoolFull."
///
/// Setup: pool has a high-FPC item. New item has lower FPC.
/// New item cannot beat the existing item → MempoolFull.
#[test]
fn vv_req_pol_002_mempool_full_when_fpc_too_low() {
    let bvc = probe_bundle_virtual_cost();

    // Capacity for exactly 1 bundle
    let config = MempoolConfig::default().with_max_total_cost(bvc + 1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit a high-FPC bundle (takes the only slot).
    // Amount=100 fits in 1 CLVM byte → same virtual_cost as probe bundle (bvc).
    let coin1 = Coin::new(
        Bytes32::from([1u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100, // fee=100 mojos; high FPC vs b2's fee=1
    );
    let mut cr1 = HashMap::new();
    let b1 = coin_bundle(coin1, &mut cr1);
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Attempt to submit a low-FPC bundle (can't beat the existing one)
    let coin2 = Coin::new(
        Bytes32::from([2u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        1, // very low fee → very low FPC
    );
    let mut cr2 = HashMap::new();
    let b2 = coin_bundle(coin2, &mut cr2);

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::MempoolFull)),
        "Low-FPC bundle should get MempoolFull, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool should still have 1 item");
}

/// Test: MempoolFull when pool is empty but max_total_cost is too small.
///
/// Proves: when pool has no items to evict, MempoolFull is returned.
#[test]
fn vv_req_pol_002_mempool_full_empty_pool_too_small() {
    let bvc = probe_bundle_virtual_cost();

    // Capacity is strictly less than any real bundle's virtual_cost
    let config = MempoolConfig::default().with_max_total_cost(bvc - 1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // A real 1-spend bundle: virtual_cost == bvc > max_total_cost
    let coin = Coin::new(
        Bytes32::from([0xaau8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100,
    );
    let mut cr = HashMap::new();
    let b = coin_bundle(coin, &mut cr);

    let result = mempool.submit(b, &cr, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::MempoolFull)),
        "Should be MempoolFull when max_total_cost < virtual_cost, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 0);
}

// ──────────────────────────────────────────────────────────────────────────
// Spend count limit
// ──────────────────────────────────────────────────────────────────────────

/// Test: TooManySpends when adding new item would exceed max_spends_per_block.
///
/// Proves POL-002: "total_spends + new_item.num_spends > max_spends_per_block
/// → TooManySpends."
#[test]
fn vv_req_pol_002_too_many_spends() {
    // Set max_spends_per_block = 1 (only 1 spend allowed total)
    let config = MempoolConfig::default().with_max_spends_per_block(1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit a bundle with 1 spend — fills the spend budget
    let coin1 = Coin::new(
        Bytes32::from([1u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100,
    );
    let mut cr1 = HashMap::new();
    let b1 = coin_bundle(coin1, &mut cr1);
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Submit a second 1-spend bundle — total would be 2 > max 1
    let coin2 = Coin::new(
        Bytes32::from([2u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100,
    );
    let mut cr2 = HashMap::new();
    let b2 = coin_bundle(coin2, &mut cr2);

    let result = mempool.submit(b2, &cr2, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::TooManySpends { .. })),
        "Should be TooManySpends, got: {:?}",
        result
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Eviction stops when enough space is freed
// ──────────────────────────────────────────────────────────────────────────

/// Test: Only the minimum number of items is evicted to fit the new item.
///
/// Proves POL-002: "If sufficient space is now available, stop evicting."
///
/// Setup: 3-item capacity. Submit 3 items (pool full). Submit 1 more high-FPC
/// item — only 1 item should be evicted (the lowest-score one), not all 3.
#[test]
fn vv_req_pol_002_minimal_eviction() {
    let bvc = probe_bundle_virtual_cost();

    // Capacity for exactly 2 bundles
    let config = MempoolConfig::default().with_max_total_cost(bvc * 2 + 1);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Submit 2 low-FPC bundles (fill pool to capacity)
    let coin1 = Coin::new(Bytes32::from([1u8; 32]), Bytes32::from(NIL_PUZZLE_HASH), 1);
    let coin2 = Coin::new(Bytes32::from([2u8; 32]), Bytes32::from(NIL_PUZZLE_HASH), 2);
    let mut cr1 = HashMap::new();
    let mut cr2 = HashMap::new();
    let b1 = coin_bundle(coin1, &mut cr1);
    let b2 = coin_bundle(coin2, &mut cr2);
    let b1_id = b1.name();
    let b2_id = b2.name();
    mempool.submit(b1, &cr1, 0, 0).unwrap();
    mempool.submit(b2, &cr2, 0, 0).unwrap();
    assert_eq!(
        mempool.len(),
        2,
        "Pool should have 2 items before third submission"
    );

    // Submit 1 high-FPC bundle (should evict only 1 item, not both).
    // Amount=100 → fee=100, same virtual_cost as b1/b2, FPC = 100× b1's FPC.
    let coin3 = Coin::new(
        Bytes32::from([3u8; 32]),
        Bytes32::from(NIL_PUZZLE_HASH),
        100,
    );
    let mut cr3 = HashMap::new();
    let b3 = coin_bundle(coin3, &mut cr3);
    let b3_id = b3.name();

    let result = mempool.submit(b3, &cr3, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "High-FPC bundle should be admitted"
    );

    // Pool should have 2 items: the high-FPC bundle + the higher-fee low-FPC bundle
    assert_eq!(mempool.len(), 2, "Only 1 item should have been evicted");
    assert!(
        mempool.contains(&b3_id),
        "New high-FPC bundle should be in pool"
    );

    // The lowest-FPC item (b1, fee=1) should be evicted
    // The higher-FPC item (b2, fee=2) should remain
    assert!(
        !mempool.contains(&b1_id),
        "Lowest-FPC bundle (fee=1) should have been evicted"
    );
    assert!(
        mempool.contains(&b2_id),
        "Higher-FPC bundle (fee=2) should remain in pool"
    );
}
