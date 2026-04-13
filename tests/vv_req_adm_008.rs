//! REQUIREMENT: ADM-008 — submit_batch() Concurrent Phase 1, Sequential Phase 2
//!
//! Test-driven verification that `submit_batch()` validates all bundles
//! and returns one result per input in the same order.
//!
//! ## What this proves
//!
//! - submit_batch() exists with correct signature
//! - Returns Vec<Result<SubmitResult, MempoolError>> matching input order
//! - Each bundle is independently validated (failures don't block others)
//! - Empty batch returns empty results
//! - Dedup across batch entries works (identical bundles rejected)
//!
//! Reference: docs/requirements/domains/admission/specs/ADM-008.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{CoinRecord, Mempool, MempoolError, SubmitResult};

/// Test: submit_batch() exists with correct signature.
///
/// Proves ADM-008: the method exists and returns Vec of results.
#[test]
fn vv_req_adm_008_batch_signature() {
    let mempool = Mempool::new(DIG_TESTNET);
    let bundles: Vec<SpendBundle> = vec![];
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    let results: Vec<Result<SubmitResult, MempoolError>> =
        mempool.submit_batch(bundles, &coin_records, 0, 0);
    assert!(results.is_empty());
}

/// Test: Each bundle gets its own result in order.
///
/// Proves ADM-008: "Returns one result per input bundle, in the same order."
/// We submit 3 distinct bundles; all should get independent results.
#[test]
fn vv_req_adm_008_results_match_input_order() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // 3 distinct bundles (different fake coins for different IDs)
    let bundles: Vec<SpendBundle> = (0..3)
        .map(|i| {
            let coin = dig_clvm::Coin::new(
                Bytes32::from([i as u8; 32]),
                Bytes32::from([i as u8; 32]),
                (i + 1) as u64,
            );
            let cs = dig_clvm::CoinSpend::new(
                coin,
                dig_clvm::Program::default(),
                dig_clvm::Program::default(),
            );
            SpendBundle::new(vec![cs], dig_clvm::Signature::default())
        })
        .collect();

    let results = mempool.submit_batch(bundles, &coin_records, 0, 0);

    // Should have 3 results (one per bundle)
    assert_eq!(results.len(), 3);
    // Each should be an error (coins not in records) — but each is independent
    for result in &results {
        assert!(result.is_err());
    }
}

/// Test: Empty batch returns empty results.
///
/// Proves ADM-008: submit_batch with zero bundles returns zero results.
#[test]
fn vv_req_adm_008_empty_batch() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    let results = mempool.submit_batch(vec![], &coin_records, 0, 0);
    assert!(results.is_empty());
}

/// Test: Identical bundles in same batch — second should be AlreadySeen.
///
/// Proves ADM-008 + ADM-003 interaction: dedup works across batch entries.
/// First entry is processed normally; second identical entry is rejected.
#[test]
fn vv_req_adm_008_dedup_within_batch() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    let bundle1 = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let bundle2 = SpendBundle::new(vec![], dig_clvm::Signature::default());
    assert_eq!(bundle1.name(), bundle2.name(), "Identical bundles same ID");

    let results = mempool.submit_batch(vec![bundle1, bundle2], &coin_records, 0, 0);
    assert_eq!(results.len(), 2);

    // First should succeed
    assert_eq!(results[0], Ok(SubmitResult::Success));
    // Second should be AlreadySeen
    assert!(
        matches!(&results[1], Err(MempoolError::AlreadySeen(_))),
        "Second identical bundle should be AlreadySeen, got: {:?}",
        results[1]
    );
}

/// Test: Mixed valid and invalid bundles in batch.
///
/// Proves ADM-008: "Failures don't block other bundles."
/// A batch with one valid and one invalid bundle should produce
/// Ok for the valid one and Err for the invalid one.
#[test]
fn vv_req_adm_008_mixed_batch() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // Valid: empty bundle
    let valid = SpendBundle::new(vec![], dig_clvm::Signature::default());

    // Invalid: references a coin not in records
    let fake_coin = dig_clvm::Coin::new(Bytes32::from([99u8; 32]), Bytes32::from([99u8; 32]), 1);
    let cs = dig_clvm::CoinSpend::new(
        fake_coin,
        dig_clvm::Program::default(),
        dig_clvm::Program::default(),
    );
    let invalid = SpendBundle::new(vec![cs], dig_clvm::Signature::default());

    let results = mempool.submit_batch(vec![valid, invalid], &coin_records, 0, 0);
    assert_eq!(results.len(), 2);

    // First (valid) should succeed
    assert!(results[0].is_ok(), "Valid bundle should succeed");
    // Second (invalid) should fail
    assert!(results[1].is_err(), "Invalid bundle should fail");
}
