//! REQUIREMENT: FEE-001 — Minimum Fee Estimation (estimate_min_fee)
//!
//! Proves estimate_min_fee(cost, num_spends):
//! - Is publicly exported on Mempool
//! - Returns 0 when mempool is empty
//! - Returns 0 when utilization is below 80%
//! - Returns nonzero minimum when utilization is 80-100% (tier 2)
//! - Returns virtual_cost * (lowest_fpc + 1) / FPC_SCALE when at/over 100% (tier 3)
//! - Uses virtual_cost (not raw cost) — higher num_spends → higher fee estimate
//!
//! Reference: docs/requirements/domains/fee_estimation/specs/FEE-001.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{FPC_SCALE, MempoolConfig};
use dig_mempool::Mempool;
use hex_literal::hex;

const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

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

fn nil_bundle(parent: u8) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let coin = Coin::new(Bytes32::from([parent; 32]), NIL_PUZZLE_HASH, 1000);
    let bundle = SpendBundle::new(
        vec![CoinSpend::new(coin, Program::default(), Program::default())],
        Signature::default(),
    );
    let mut cr = HashMap::new();
    cr.insert(coin.coin_id(), coin_record(coin));
    (bundle, cr)
}

/// Discover the virtual_cost of a nil bundle by submitting one to a probe mempool.
fn nil_bundle_virtual_cost() -> u64 {
    let (bundle, cr) = nil_bundle(0xFE);
    let probe = Mempool::new(DIG_TESTNET);
    probe.submit(bundle, &cr, 0, 0).unwrap();
    probe.stats().total_cost
}

/// estimate_min_fee() is publicly accessible.
///
/// Proves FEE-001: "estimate_min_fee() is publicly exported on the Mempool struct."
#[test]
fn vv_req_fee_001_is_public() {
    let mempool = Mempool::new(DIG_TESTNET);
    // Calling it compiles → proves it's public.
    let _fee = mempool.estimate_min_fee(1_000_000, 1);
}

/// Empty mempool always returns 0.
///
/// Proves FEE-001: "Returns 0 when the active pool is empty regardless of tier."
#[test]
fn vv_req_fee_001_empty_pool_returns_zero() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.estimate_min_fee(1_000_000, 1), 0);
    assert_eq!(mempool.estimate_min_fee(10_000_000_000, 10), 0);
    assert_eq!(mempool.estimate_min_fee(0, 0), 0);
}

/// Below 80% utilization, minimum fee is 0.
///
/// Proves FEE-001: "Returns 0 when mempool utilization is below 80%."
#[test]
fn vv_req_fee_001_below_80_pct_returns_zero() {
    let actual_cost = nil_bundle_virtual_cost();

    // max_cost >> actual_cost → utilization << 80%.
    let max_cost = actual_cost * 1000; // ~0.1% utilization
    let config = MempoolConfig::default().with_max_total_cost(max_cost);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (bundle, cr) = nil_bundle(0x01);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let stats = mempool.stats();
    assert!(
        stats.utilization < 0.80,
        "utilization must be < 80%: {}",
        stats.utilization
    );

    assert_eq!(
        mempool.estimate_min_fee(1_000_000, 0),
        0,
        "fee must be 0 when utilization < 80%"
    );
}

/// At 80-100% utilization, returns nonzero minimum fee (tier 2).
///
/// Proves FEE-001: "Returns virtual_cost * full_mempool_min_fpc_scaled / FPC_SCALE
/// when utilization is 80-100%."
#[test]
fn vv_req_fee_001_tier2_returns_nonzero_min() {
    let actual_cost = nil_bundle_virtual_cost();

    // Set max_cost so utilization ≈ 90% (tier 2: 80-100%).
    // actual_cost / max_cost = 0.90 → max_cost = actual_cost / 0.90 ≈ actual_cost * 10/9
    let max_cost = actual_cost * 10 / 9;
    let fpc_scaled = 5 * FPC_SCALE; // default: 5 mojos per cost unit
    let config = MempoolConfig::default()
        .with_max_total_cost(max_cost)
        .with_full_mempool_min_fpc_scaled(fpc_scaled);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (bundle, cr) = nil_bundle(0x01);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let stats = mempool.stats();
    assert!(
        stats.utilization >= 0.80 && stats.utilization < 1.0,
        "utilization must be 80-100%: {}",
        stats.utilization
    );

    // With num_spends=0, virtual_cost = cost.
    let query_cost = 1_000_000u64;
    let expected = (query_cost as u128) * fpc_scaled / FPC_SCALE;

    let actual = mempool.estimate_min_fee(query_cost, 0);
    assert_eq!(
        actual,
        expected.min(u64::MAX as u128) as u64,
        "tier 2 fee must be virtual_cost * fpc_scaled / FPC_SCALE"
    );
    assert!(actual > 0, "tier 2 fee must be nonzero");
}

/// At 100%+ utilization, returns virtual_cost * (lowest_fpc + 1) / FPC_SCALE (tier 3).
///
/// Proves FEE-001: "Returns virtual_cost * (lowest_fpc + 1) / FPC_SCALE when
/// utilization is at or above 100%."
#[test]
fn vv_req_fee_001_tier3_returns_lowest_fpc_plus_one() {
    let actual_cost = nil_bundle_virtual_cost();

    // Set max_cost = actual_cost → utilization = 100% (tier 3).
    let config = MempoolConfig::default().with_max_total_cost(actual_cost);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (bundle, cr) = nil_bundle(0x01);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let stats = mempool.stats();
    assert!(
        stats.utilization >= 1.0,
        "utilization must be >= 100%: {}",
        stats.utilization
    );

    let lowest_fpc = stats.min_fpc_scaled; // 0 for nil bundles (fee=0)
    let query_cost = 1_000_000_000_000u64; // large cost to avoid zero result
    let expected = (query_cost as u128) * (lowest_fpc + 1) / FPC_SCALE;

    let actual = mempool.estimate_min_fee(query_cost, 0);
    assert_eq!(
        actual,
        expected.min(u64::MAX as u128) as u64,
        "tier 3 fee must be virtual_cost * (lowest_fpc + 1) / FPC_SCALE: lowest_fpc={lowest_fpc}"
    );
}

/// Tier 2 fee scales proportionally with cost (higher cost → higher min fee).
///
/// Proves FEE-001: "Uses virtual_cost (not raw cost) in all fee calculations."
#[test]
fn vv_req_fee_001_tier2_proportional_to_cost() {
    let actual_cost = nil_bundle_virtual_cost();

    let max_cost = actual_cost * 10 / 9;
    let config = MempoolConfig::default().with_max_total_cost(max_cost);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (bundle, cr) = nil_bundle(0x01);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    assert!(mempool.stats().utilization >= 0.80);

    // 10x cost → 10x fee (proportional, with num_spends=0 so virtual_cost = cost)
    let fee_low = mempool.estimate_min_fee(1_000_000, 0);
    let fee_high = mempool.estimate_min_fee(10_000_000, 0);

    assert!(
        fee_low > 0,
        "fee at tier 2 must be nonzero for cost=1M: {fee_low}"
    );
    // Allow for integer rounding: fee_high / fee_low should be ≈ 10.
    assert!(
        fee_high >= fee_low * 9 && fee_high <= fee_low * 11,
        "fee must scale ~linearly with cost: low={fee_low}, high={fee_high}"
    );
}

/// More spends → higher min fee (spend penalty in virtual cost).
///
/// Proves FEE-001: "Uses virtual_cost (not raw cost) — higher num_spends → higher
/// fee estimate."
#[test]
fn vv_req_fee_001_virtual_cost_includes_spend_penalty() {
    let actual_cost = nil_bundle_virtual_cost();

    let max_cost = actual_cost * 10 / 9;
    let config = MempoolConfig::default().with_max_total_cost(max_cost);
    let mempool = Mempool::with_config(DIG_TESTNET, config);

    let (bundle, cr) = nil_bundle(0x01);
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    assert!(mempool.stats().utilization >= 0.80);

    // Same raw cost but more spends → higher virtual_cost → higher fee.
    let fee_few = mempool.estimate_min_fee(1_000_000, 1);
    let fee_many = mempool.estimate_min_fee(1_000_000, 20);

    assert!(
        fee_many > fee_few,
        "more spends must yield higher estimate: few={fee_few}, many={fee_many}"
    );
}
