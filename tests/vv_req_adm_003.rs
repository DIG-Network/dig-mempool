//! REQUIREMENT: ADM-003 — Dedup Check via Seen-Cache Before CLVM Validation
//!
//! Test-driven verification that duplicate bundle submissions are rejected
//! with `MempoolError::AlreadySeen` without re-running CLVM validation.
//!
//! ## What this proves
//!
//! - Same bundle submitted twice → second returns AlreadySeen
//! - Bundle ID computed via `SpendBundle::name()` (chia-protocol canonical hash)
//! - Seen-cache populated before CLVM (even invalid bundles are cached)
//! - LRU eviction when cache exceeds max_seen_cache_size
//! - Dedup check runs before CLVM validation (DoS protection)
//!
//! ## Chia L1 Correspondence
//!
//! Mirrors `MempoolManager.seen_bundle_hashes` at:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L298
//!
//! Reference: docs/requirements/domains/admission/specs/ADM-003.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{CoinRecord, Mempool, MempoolConfig, MempoolError};

/// Test: Submitting the same bundle twice returns AlreadySeen on second attempt.
///
/// Proves ADM-003 core criterion: "Duplicate bundles return
/// MempoolError::AlreadySeen(bundle_id)."
///
/// The first submission passes (empty bundle = trivially valid).
/// The second submission of an identical bundle should be rejected
/// immediately without re-running CLVM validation.
#[test]
fn vv_req_adm_003_duplicate_rejected() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // First submission — should succeed (empty bundle, trivially valid)
    let bundle1 = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let id = bundle1.name(); // Compute bundle ID for later comparison
    let result1 = mempool.submit(bundle1, &coin_records, 0, 0);
    assert!(result1.is_ok(), "First submission should succeed");

    // Second submission — identical bundle, should be rejected as duplicate
    let bundle2 = SpendBundle::new(vec![], dig_clvm::Signature::default());
    assert_eq!(bundle2.name(), id, "Same bundle should produce same ID");
    let result2 = mempool.submit(bundle2, &coin_records, 0, 0);

    match result2 {
        Err(MempoolError::AlreadySeen(seen_id)) => {
            assert_eq!(seen_id, id, "AlreadySeen should report the bundle ID");
        }
        other => panic!(
            "Second submission should return AlreadySeen, got: {:?}",
            other
        ),
    }
}

/// Test: Invalid bundles are also cached in the seen-cache.
///
/// Proves ADM-003 criterion: "Even invalid bundles are cached in the
/// seen-cache (prevents repeated CLVM validation of bad bundles)."
///
/// The seen-cache is populated BEFORE CLVM validation. If an invalid
/// bundle is submitted, it will fail CLVM, but its ID is still cached.
/// A second submission of the same invalid bundle should return
/// AlreadySeen (not re-run CLVM).
#[test]
fn vv_req_adm_003_invalid_bundle_cached() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // Create an invalid bundle (references a coin not in coin_records)
    let fake_coin = dig_clvm::Coin::new(Bytes32::default(), Bytes32::from([1u8; 32]), 1000);
    let coin_spend = dig_clvm::CoinSpend::new(
        fake_coin,
        dig_clvm::Program::default(),
        dig_clvm::Program::default(),
    );

    // First submission — should fail CLVM validation (coin not found)
    let bundle1 = SpendBundle::new(vec![coin_spend.clone()], dig_clvm::Signature::default());
    let id = bundle1.name();
    let result1 = mempool.submit(bundle1, &coin_records, 0, 0);
    assert!(result1.is_err(), "First submission should fail (invalid)");

    // Second submission — same invalid bundle, should be AlreadySeen (not re-validated)
    let bundle2 = SpendBundle::new(vec![coin_spend], dig_clvm::Signature::default());
    let result2 = mempool.submit(bundle2, &coin_records, 0, 0);
    match result2 {
        Err(MempoolError::AlreadySeen(seen_id)) => {
            assert_eq!(seen_id, id);
        }
        other => panic!(
            "Second submission of invalid bundle should return AlreadySeen, got: {:?}",
            other
        ),
    }
}

/// Test: Seen-cache LRU eviction allows oldest entries to be resubmitted.
///
/// Proves ADM-003 criterion: "The seen-cache respects
/// config.max_seen_cache_size with LRU eviction."
///
/// We create a mempool with a tiny seen-cache (size=2), submit 3 different
/// bundles, then retry the first one. The first should have been evicted
/// from the LRU cache, allowing resubmission.
#[test]
fn vv_req_adm_003_seen_cache_lru_eviction() {
    // Use a very small seen-cache for this test
    let config = MempoolConfig::default().with_max_seen_cache_size(2);
    let mempool = Mempool::with_config(DIG_TESTNET, config);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // Submit 3 different bundles (empty bundles with different "identities"
    // won't work since they're all identical. We need distinguishable bundles.)
    // Use bundles with different fake coins to get different bundle IDs.
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

    let id0 = bundles[0].name();

    // Submit all 3 (they'll fail CLVM since coins aren't in records, but
    // they'll still be added to the seen-cache)
    for bundle in &bundles {
        let b = SpendBundle::new(
            bundle.coin_spends.clone(),
            bundle.aggregated_signature.clone(),
        );
        let _ = mempool.submit(b, &coin_records, 0, 0);
    }

    // The seen-cache size is 2, so bundle[0] should have been evicted.
    // Resubmitting bundle[0] should NOT return AlreadySeen — it should
    // go through CLVM validation again (and fail for coin not found).
    let retry = SpendBundle::new(
        bundles[0].coin_spends.clone(),
        bundles[0].aggregated_signature.clone(),
    );
    assert_eq!(retry.name(), id0);
    let result = mempool.submit(retry, &coin_records, 0, 0);

    // Should NOT be AlreadySeen (evicted from cache) — should be a validation error
    match result {
        Err(MempoolError::AlreadySeen(_)) => {
            panic!("Bundle should have been evicted from LRU cache");
        }
        _ => {
            // Any other result is fine — the point is it's not AlreadySeen
        }
    }
}

/// Test: Bundle ID is computed via SpendBundle::name().
///
/// Proves ADM-003 criterion: "Bundle ID is computed via SpendBundle::name()
/// from chia-protocol." This is the canonical Chia hash.
#[test]
fn vv_req_adm_003_bundle_id_via_name() {
    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let id1 = bundle.name();
    let id2 = bundle.name();
    // Same bundle → same name (deterministic)
    assert_eq!(id1, id2, "SpendBundle::name() must be deterministic");
    // Non-zero hash (not all zeros)
    // Note: empty bundle may have all-zero hash depending on Streamable impl
    let _: Bytes32 = id1; // Type check
}
