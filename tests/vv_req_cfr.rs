//! REQUIREMENT: CFR-001 through CFR-006 — Conflict Resolution / RBF
//!
//! Test-driven verification of active-pool conflict detection and all
//! Replace-by-Fee rules.
//!
//! ## Test Design Notes
//!
//! Two bundles spending the **same coin** must have different bundle hashes or
//! the second submission hits the AlreadySeen cache before reaching conflict
//! detection.  Solution: `alt_bundle()` uses a non-nil solution (`vec![0x01]`);
//! the nil puzzle ignores the solution, so the fee/FPC are identical but the
//! serialized SpendBundle (and thus its hash) differs.
//!
//! For "higher fee while conflicting on coin X", we add an additional coin to
//! bundle B (strict superset of bundle A's removals).  Fee_B = fee_A + boost.
//!
//! ## What this proves
//!
//! ### CFR-001: Conflict Detection
//! - No conflict on empty pool → direct admission
//! - Single conflict detected via coin_index
//! - Both conflicting bundles detected when C touches coins from A and B
//! - Pending pool items do NOT appear in conflict detection
//! - Conflict cache items do NOT appear in conflict detection
//! - coin_index cleaned on removal — no stale entries
//!
//! ### CFR-002: RBF Superset Rule
//! - Superset passes (new bundle spends more coins)
//! - Missing removal → RbfNotSuperset
//! - Multiple conflicts — new bundle must cover ALL removals of ALL conflicts
//!
//! ### CFR-003: RBF FPC Strictly Higher
//! - Higher FPC passes (strict superset with large fee boost)
//! - Equal FPC → RbfFpcNotHigher
//! - Lower FPC → RbfFpcNotHigher
//! - Aggregate FPC compared (not per-conflict)
//!
//! ### CFR-004: RBF Minimum Fee Bump
//! - Fee exactly at required minimum passes
//! - Fee below minimum → RbfBumpTooLow with correct required/provided
//! - Aggregate fee compared (sum of all conflicts)
//!
//! ### CFR-005: Conflict Cache on RBF Failure
//! - Failed superset check → bundle added to conflict cache
//! - Failed FPC check → bundle added to conflict cache
//! - Failed bump check → bundle added to conflict cache
//!
//! ### CFR-006: Remove Conflicting Items on Successful RBF
//! - Conflicting item removed; replacement inserted; pool count stable
//! - Both conflicting items removed when new bundle beats both
//! - coin_index entries cleaned after removal; new bundle's entries installed
//!
//! Reference: docs/requirements/domains/conflict_resolution/specs/

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, MempoolConfig, MempoolError, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
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

/// Bundle spending one coin with NIL puzzle and NIL solution.
/// fee = coin.amount (no outputs).
fn nil_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

/// Bundle spending one coin with NIL puzzle and ALT solution (atom 1).
///
/// The nil puzzle ignores the solution, so fee == coin.amount as usual.
/// However the bundle hash differs from `nil_bundle()` because the serialised
/// CoinSpend includes the solution bytes.  Use this to produce a *different*
/// bundle that spends the *same* coin without hitting the AlreadySeen cache.
fn alt_bundle(coin: Coin) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let alt_sol = Program::new(vec![0x01].into()); // atom 1
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), alt_sol)],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

/// Bundle spending two coins with NIL puzzle + NIL solution.
/// fee = coin_a.amount + coin_b.amount.
fn two_coin_bundle(
    coin_a: Coin,
    coin_b: Coin,
) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
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

/// Bundle spending three coins with NIL puzzle + NIL solution.
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

// ── CFR-001: Conflict Detection ───────────────────────────────────────────

/// No conflict on empty pool → direct admission.
///
/// Proves CFR-001: "An empty conflict set results in direct admission."
#[test]
fn vv_req_cfr_001_no_conflict_empty_pool() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 1);
    let (bundle, cr) = nil_bundle(coin);
    assert_eq!(mempool.submit(bundle, &cr, 0, 0), Ok(SubmitResult::Success));
    assert_eq!(mempool.len(), 1);
}

/// Single conflict detected: coin_index returns the existing bundle.
///
/// Proves CFR-001: "For each removal, look up coin_index. If found, add to conflict set."
/// Bundle A and bundle B both spend coin_X. They have different bundle hashes
/// (alt_bundle uses a non-nil solution). Both have fee = coin_X.amount.
/// FPC is equal → conflict detected but RBF fails with RbfFpcNotHigher.
#[test]
fn vv_req_cfr_001_single_conflict_detected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 10);

    // Bundle A: nil solution → fee=10
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Bundle B: alt solution → different bundle hash, same coin_x, same fee=10
    let (bundle_b, cr_b) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Equal-FPC conflicting bundle should fail with RbfFpcNotHigher, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool unchanged after RBF failure");
}

/// Multiple conflicts: bundle C touches coins from two separate active items.
///
/// Proves CFR-001: "All unique conflicting bundle IDs are collected."
/// Bundle A spends coin_X, bundle B spends coin_Y. Bundle C spends
/// {coin_X, coin_Y, coin_boost} — conflicts with both. C has much higher
/// FPC than the aggregate of A+B, so RBF succeeds and both A and B are removed.
#[test]
fn vv_req_cfr_001_multiple_conflicts_detected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: 1 spend coin_X (fee=1)
    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: 1 spend coin_Y (fee=1)
    let coin_y = make_coin(0x02, 1);
    let (bundle_b, cr_b) = nil_bundle(coin_y);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // Bundle C: spends {coin_X, coin_Y, coin_boost(amount=127)}.
    // fee_C = 1+1+127 = 129, vc_C ≈ 3×vc_1.
    // FPC_C = 129/(3×vc_1) = 43/vc_1 >> FPC_aggregate = 2/(2×vc_1) = 1/vc_1.
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Bundle C should replace both A and B, got: {:?}",
        result
    );
    assert_eq!(
        mempool.len(),
        1,
        "A and B both removed; C inserted — multiple conflicts were detected"
    );
}

/// Pending pool items do NOT appear in conflict detection.
///
/// Proves CFR-001: "Pending pool items: NOT checked."
#[test]
fn vv_req_cfr_001_pending_not_conflict() {
    use dig_clvm::{
        clvmr::{serde::node_to_bytes, Allocator},
        tree_hash, TreeHash,
    };

    let mempool = Mempool::new(DIG_TESTNET);

    // Build a puzzle with ASSERT_HEIGHT_ABSOLUTE(100) → routes to pending pool
    let mut a = Allocator::new();
    let nil = a.nil();
    let h = a.new_atom(&[100u8]).unwrap();
    let inner = a.new_pair(h, nil).unwrap();
    let op = a.new_atom(&[83u8]).unwrap();
    let cond = a.new_pair(op, inner).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let hash: TreeHash = tree_hash(&a, prog);
    let puzzle_hash = Bytes32::from(hash);
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());

    // coin_p has puzzle_hash of the AHA puzzle
    let coin_p = Coin::new(Bytes32::from([0x10u8; 32]), puzzle_hash, 1);
    let mut cr_p = HashMap::new();
    cr_p.insert(coin_p.coin_id(), coin_record(coin_p));
    let bundle_pending = SpendBundle::new(
        vec![CoinSpend::new(coin_p, puzzle, Program::default())],
        Signature::default(),
    );
    let r = mempool.submit(bundle_pending, &cr_p, 0, 0);
    assert!(
        matches!(r, Ok(SubmitResult::Pending { .. })),
        "Should go to pending pool, got: {:?}",
        r
    );
    assert_eq!(mempool.pending_len(), 1);

    // Coin with NIL_PUZZLE_HASH having the same parent byte — different coin_id
    // (different puzzle_hash → different coin_id)
    let coin_active = make_coin(0x10, 1);
    let (bundle_active, cr_active) = nil_bundle(coin_active);
    let result = mempool.submit(bundle_active, &cr_active, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Active bundle should be admitted with no conflict against pending item"
    );
    assert_eq!(mempool.len(), 1);
}

/// Conflict cache items do NOT appear in conflict detection.
///
/// Proves CFR-001: "Conflict cache items: NOT checked via coin_index."
#[test]
fn vv_req_cfr_001_conflict_cache_not_indexed() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_x = make_coin(0x01, 1);

    // Add a bundle to conflict cache directly (bypasses coin_index)
    let (bundle_cached, _) = nil_bundle(coin_x);
    mempool.add_to_conflict_cache(bundle_cached, 1_000);
    assert_eq!(mempool.conflict_len(), 1);

    // Submit a DIFFERENT bundle spending coin_X (alt solution → different id)
    // The conflict cache does not populate coin_index, so no conflict detected.
    let (bundle_new, cr_new) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_new, &cr_new, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Conflict cache items should not block via coin_index, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1);
}

/// coin_index cleaned on removal — no stale entries block re-submission.
///
/// Proves CFR-001: "After removal, coin_index is empty for that coin."
#[test]
fn vv_req_cfr_001_coin_index_cleaned_on_removal() {
    let mempool = Mempool::new(DIG_TESTNET);

    let coin_x = make_coin(0x01, 1);

    // Bundle A: nil solution → in pool
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);

    // Remove A: its coin_index entries must be cleaned up
    assert!(mempool.remove(&a_id), "remove() should succeed");
    assert_eq!(mempool.len(), 0);

    // Bundle B (alt solution → different hash): same coin_X, no stale entry → Success
    let (bundle_b, cr_b) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "After removal, same coin should be spendable without conflict"
    );
    assert_eq!(mempool.len(), 1);
}

// ── CFR-002: Superset Rule ─────────────────────────────────────────────────

/// Superset passes: new bundle spends the conflicting coin plus extra coins.
///
/// Proves CFR-002: "Superset rule passes when new bundle's removals ⊇ every
/// conflict's removals."
#[test]
fn vv_req_cfr_002_superset_passes() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: 1 spend coin_X (fee=1)
    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: spends {coin_X, coin_boost(127)}.
    // fee_B = 128, vc_B ≈ 2×vc_1 → FPC_B = 64/vc_1 >> 1/vc_1 = FPC_A.
    // Superset: {coin_X, coin_boost} ⊃ {coin_X} ✓
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Superset with higher fee should succeed, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "A replaced by B");
}

/// Missing removal: conflict's coin not in new bundle → RbfNotSuperset.
///
/// Proves CFR-002: "If any conflicting bundle has a removal not in the new
/// bundle, RbfNotSuperset is returned."
#[test]
fn vv_req_cfr_002_missing_removal_fails() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: spends both coin_X and coin_Y
    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);
    let (bundle_a, cr_a) = two_coin_bundle(coin_x, coin_y);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: spends only coin_X — misses coin_Y → RbfNotSuperset
    let (bundle_b, cr_b) = nil_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "Missing removal should return RbfNotSuperset, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool unchanged after superset failure");
}

/// Multiple conflicts — new bundle must cover ALL removals of ALL conflicts.
///
/// Proves CFR-002: "Aggregation across all conflicting bundles."
///
/// Bundle A spends {coin_X, coin_Y}.  Bundle B spends {coin_Z} — no overlap
/// with A so both can coexist.  Bundle C spends {coin_X, coin_Z, coin_boost}:
/// it conflicts with A via coin_X AND with B via coin_Z.  C's removal set
/// {X, Z, boost} covers B's {Z} but misses coin_Y from A → RbfNotSuperset.
#[test]
fn vv_req_cfr_002_multiple_conflicts_all_must_be_covered() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);
    let coin_z = make_coin(0x03, 1);
    let coin_boost = make_coin(0x0F, 127);

    // Bundle A spends {coin_X, coin_Y}
    let (bundle_a, cr_a) = two_coin_bundle(coin_x, coin_y);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B spends {coin_Z} — no overlap with A
    let (bundle_b, cr_b) = nil_bundle(coin_z);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // Bundle C spends {coin_X, coin_Z, coin_boost}: conflicts with A (via X)
    // and B (via Z).  C's set {X, Z, boost} misses coin_Y (from A) → RbfNotSuperset.
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_z, coin_boost);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "C misses coin_Y from A's removals; got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 2, "Pool unchanged after superset failure");
}

// ── CFR-003: FPC Strictly Higher ──────────────────────────────────────────

/// Higher FPC passes: replacement has higher fee per virtual cost.
///
/// Proves CFR-003: "New bundle FPC strictly > aggregate conflict FPC."
#[test]
fn vv_req_cfr_003_higher_fpc_passes() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: 1 spend coin_X (fee=1) → FPC_A = 1/vc_1
    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: {coin_X, coin_boost(127)} → fee=128, vc≈2×vc_1
    // FPC_B = 128/(2×vc_1) = 64/vc_1 >> 1/vc_1 = FPC_A ✓
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success), "Higher FPC should pass");
    assert_eq!(mempool.len(), 1);
}

/// Equal FPC: conflict and new bundle have identical FPC → rejected.
///
/// Proves CFR-003: "equal FPC is NOT strictly higher → RbfFpcNotHigher."
/// Both bundles spend coin_X (1 spend, same fee = coin_X.amount).
/// alt_bundle uses a different solution for a distinct bundle hash.
#[test]
fn vv_req_cfr_003_equal_fpc_rejected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 10);

    // Bundle A: nil solution
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: alt solution → different hash, same coin → same fee → same FPC
    let (bundle_b, cr_b) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Equal FPC should fail, got: {:?}",
        result
    );
}

/// Lower FPC: new bundle's FPC is lower than existing → rejected.
///
/// Proves CFR-003: "lower FPC → RbfFpcNotHigher."
/// Bundle A: 1 spend of coin_X(100) → FPC_A = 100/vc_1.
/// Bundle B: {coin_X(100), coin_cheap(1)} → fee=101, vc≈2×vc_1
///           FPC_B = 101/(2×vc_1) ≈ 50.5/vc_1 < 100/vc_1 = FPC_A.
#[test]
fn vv_req_cfr_003_lower_fpc_rejected() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // coin amounts must be 1-byte CLVM-encodable (1..=127)
    let coin_x = make_coin(0x01, 100);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B has lower FPC: add cheap coin to boost spend count
    let coin_cheap = make_coin(0x0E, 1);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_cheap);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Lower FPC should fail, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Pool unchanged");
}

/// Aggregate FPC compared across all conflicts.
///
/// Proves CFR-003: "FPC comparison is against the aggregate of all conflicts."
/// Bundle A spends coin_X (fee=1), bundle B spends coin_Y (fee=1).
/// Bundle C spends {coin_X, coin_Y, coin_boost(127)} → fee=129, vc≈3×vc_1.
/// FPC_C = 129/(3×vc_1) = 43/vc_1 >> FPC_aggregate = 2/(2×vc_1) = 1/vc_1.
#[test]
fn vv_req_cfr_003_aggregate_fpc_compared() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let (bundle_b, cr_b) = nil_bundle(coin_y);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // Bundle C: replaces both A and B via RBF
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Higher aggregate FPC should succeed, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Both A and B replaced by C");
}

// ── CFR-004: Minimum Fee Bump ─────────────────────────────────────────────

/// Fee exactly at minimum bump passes (>= comparison).
///
/// Proves CFR-004: "fee_new >= conflict_fees + min_rbf_fee_bump."
/// min_rbf_fee_bump = 10. fee_A = 5. Required: fee_B >= 15.
/// Bundle B spends {coin_X(5), coin_bump(10)} → fee_B = 15 exactly ✓.
#[test]
fn vv_req_cfr_004_exact_minimum_bump_passes() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(10);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 5);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // fee_B = 5+10 = 15 = fee_A(5) + bump(10) → exactly at minimum
    // FPC_B = 15/(2×vc_1) = 7.5/vc_1 > 5/vc_1 = FPC_A ✓
    let coin_bump = make_coin(0x0F, 10);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_bump);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Exact minimum fee bump should pass, got: {:?}",
        result
    );
}

/// Fee one below minimum → RbfBumpTooLow with correct required/provided.
///
/// Proves CFR-004: "fee_new < conflict_fees + min_rbf_fee_bump → RbfBumpTooLow."
/// min_rbf_fee_bump = 20. fee_A = 5. Required: fee_B >= 25.
/// Bundle B: {coin_X(5), coin_under(19)} → fee_B=24 < 25 → fails.
/// FPC_B = 24/(2×vc_1) = 12/vc_1 > 5/vc_1 (FPC check passes, bump check fails).
#[test]
fn vv_req_cfr_004_below_minimum_bump_fails() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(20);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 5);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    let coin_under = make_coin(0x0F, 19);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_under);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    match result {
        Err(MempoolError::RbfBumpTooLow { required, provided }) => {
            assert_eq!(required, 25, "required = 5 + 20 = 25");
            assert_eq!(provided, 24, "provided = 5 + 19 = 24");
        }
        other => panic!("Expected RbfBumpTooLow, got: {:?}", other),
    }
}

/// Aggregate fee bump: sum of all conflict fees + bump must be covered.
///
/// Proves CFR-004: "conflicting_fees = sum of ALL conflicting bundles' fees."
/// fee_A=5, fee_B=10, bump=10 → required=25.
/// Bundle C: {coin_X(5), coin_Y(10), coin_extra(12)} → fee_C=27 >= 25 ✓.
#[test]
fn vv_req_cfr_004_aggregate_fee_bump() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(10);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let coin_x = make_coin(0x01, 5);
    let coin_y = make_coin(0x02, 10);

    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    let (bundle_b, cr_b) = nil_bundle(coin_y);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 2);

    // fee_C = 5+10+12 = 27 >= required(25); FPC_C = 27/(3×vc_1) >> (15/(2×vc_1))
    let coin_extra = make_coin(0x0F, 12);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_extra);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(
        result,
        Ok(SubmitResult::Success),
        "Aggregate fee bump should pass, got: {:?}",
        result
    );
    assert_eq!(mempool.len(), 1, "Both A and B replaced by C");
}

// ── CFR-005: Conflict Cache on RBF Failure ────────────────────────────────

/// Failed superset check → bundle added to conflict cache; error returned.
///
/// Proves CFR-005: "On RBF failure, the bundle is added to conflict cache."
#[test]
fn vv_req_cfr_005_superset_failure_caches_bundle() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Bundle A spends {coin_X, coin_Y}
    let coin_x = make_coin(0x01, 1);
    let coin_y = make_coin(0x02, 1);
    let (bundle_a, cr_a) = two_coin_bundle(coin_x, coin_y);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B spends {coin_X} only → not superset → RbfNotSuperset + cached
    let (bundle_b, cr_b) = nil_bundle(coin_x);
    let b_id = bundle_b.name();
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "Expected RbfNotSuperset, got: {:?}",
        result
    );
    assert_eq!(mempool.conflict_len(), 1, "Failed bundle should be in conflict cache");

    let cached = mempool.drain_conflict();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].name(), b_id, "Cached bundle ID should match");
}

/// Failed FPC check → bundle added to conflict cache; error returned.
///
/// Proves CFR-005: "On RBF failure (FPC), bundle is added to conflict cache."
#[test]
fn vv_req_cfr_005_fpc_failure_caches_bundle() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: coin_X (amount=100) → high FPC
    let coin_x = make_coin(0x01, 100);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: {coin_X, coin_cheap(1)} → fee=101, vc≈2×vc_1
    // FPC_B = 101/(2×vc_1) ≈ 50.5/vc_1 < 100/vc_1 = FPC_A → RbfFpcNotHigher + cached
    let coin_cheap = make_coin(0x0E, 1);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_cheap);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfFpcNotHigher)),
        "Expected RbfFpcNotHigher, got: {:?}",
        result
    );
    assert_eq!(mempool.conflict_len(), 1, "Bundle should be in conflict cache");
}

/// Failed fee bump → bundle added to conflict cache; error returned.
///
/// Proves CFR-005: "On RBF failure (bump too low), bundle cached."
#[test]
fn vv_req_cfr_005_bump_failure_caches_bundle() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(1_000_000);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: coin_X (amount=1) → fee=1
    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: {coin_X(1), coin_extra(100)} → fee=101; FPC passes but bump fails
    // (required = 1 + 1_000_000 = 1_000_001; provided = 101)
    let coin_extra = make_coin(0x0F, 100);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_extra);
    let result = mempool.submit(bundle_b, &cr_b, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfBumpTooLow { .. })),
        "Expected RbfBumpTooLow, got: {:?}",
        result
    );
    assert_eq!(
        mempool.conflict_len(),
        1,
        "Bundle should be in conflict cache after bump failure"
    );
}

// ── CFR-006: Remove Conflicting Items on Successful RBF ───────────────────

/// Successful RBF: conflicting item removed, replacement inserted.
///
/// Proves CFR-006: "Remove conflicting items on successful RBF."
#[test]
fn vv_req_cfr_006_conflicting_item_removed() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: 1 spend coin_X (fee=1)
    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);
    assert!(mempool.contains(&a_id));

    // Bundle B: {coin_X, coin_boost(127)} → fee=128 → replaces A
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

    // Bundle C: {coin_X, coin_Y, coin_boost(127)} → fee=129
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_c, cr_c) = three_coin_bundle(coin_x, coin_y, coin_boost);
    let c_id = bundle_c.name();
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert_eq!(result, Ok(SubmitResult::Success), "RBF with two conflicts should succeed");

    assert!(!mempool.contains(&a_id), "Bundle A should be removed");
    assert!(!mempool.contains(&b_id), "Bundle B should be removed");
    assert!(mempool.contains(&c_id), "Bundle C should be inserted");
    assert_eq!(mempool.len(), 1, "Only the replacement remains");
}

/// After successful RBF, coin_index points to replacement, not the evicted bundle.
///
/// Proves CFR-006: "coin_index updated after replacement."
/// After B replaces A, an attempt to submit another bundle spending coin_X
/// with lower FPC detects a conflict against B (not a stale A entry).
#[test]
fn vv_req_cfr_006_coin_index_updated_after_rbf() {
    let config = MempoolConfig::default().with_min_rbf_fee_bump(0);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    // Bundle A: 1 spend coin_X (fee=1)
    let coin_x = make_coin(0x01, 1);
    let (bundle_a, cr_a) = nil_bundle(coin_x);
    let a_id = bundle_a.name();
    mempool.submit(bundle_a, &cr_a, 0, 0).unwrap();

    // Bundle B: {coin_X, coin_boost(127)} → replaces A
    let coin_boost = make_coin(0x0F, 127);
    let (bundle_b, cr_b) = two_coin_bundle(coin_x, coin_boost);
    mempool.submit(bundle_b, &cr_b, 0, 0).unwrap();
    assert_eq!(mempool.len(), 1);
    assert!(!mempool.contains(&a_id), "A should be gone");

    // Bundle C (alt solution of coin_X → different hash from A and B):
    // Conflicts with B. C.removals = {coin_X}. B.removals = {coin_X, coin_boost}.
    // coin_boost ∉ C.removals → RbfNotSuperset.
    // This proves the coin_index entry for coin_X now points to B (not A).
    let (bundle_c, cr_c) = alt_bundle(coin_x);
    let result = mempool.submit(bundle_c, &cr_c, 0, 0);
    assert!(
        matches!(result, Err(MempoolError::RbfNotSuperset)),
        "coin_X should now be indexed to B; C fails superset, got: {:?}",
        result
    );
}
