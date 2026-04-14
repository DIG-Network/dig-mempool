//! REQUIREMENT: API-008 — Query Methods
//!
//! Test-driven verification that all 14+ query methods exist on `Mempool`
//! with the correct signatures and return types.
//!
//! ## What this proves
//!
//! - All methods compile with correct parameter and return types
//! - Methods work on `&self` (read-only, thread-safe)
//! - Empty mempool returns sensible defaults (empty vecs, None, 0, true)
//! - CPFP coin queries have correct signatures
//!
//! Reference: docs/requirements/domains/crate_api/specs/API-008.md

use std::collections::HashMap;
use std::sync::Arc;

use dig_clvm::{Coin, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::item::MempoolItem;
use dig_mempool::{Bytes32, CoinRecord, Mempool, MempoolStats};
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

/// Test: get() returns None for unknown bundle ID on empty mempool.
///
/// Proves `get()` exists with signature `(&self, &Bytes32) -> Option<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_get_returns_none() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: Option<Arc<MempoolItem>> = mempool.get(&Bytes32::default());
    assert!(result.is_none());
}

/// Test: contains() returns false for unknown bundle ID.
///
/// Proves `contains()` exists with signature `(&self, &Bytes32) -> bool`.
#[test]
fn vv_req_api_008_contains_returns_false() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: bool = mempool.contains(&Bytes32::default());
    assert!(!result);
}

/// Test: active_bundle_ids() returns empty vec on empty mempool.
///
/// Proves signature `(&self) -> Vec<Bytes32>`.
#[test]
fn vv_req_api_008_active_bundle_ids_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let ids: Vec<Bytes32> = mempool.active_bundle_ids();
    assert!(ids.is_empty());
}

/// Test: pending_bundle_ids() returns empty vec.
///
/// Proves signature `(&self) -> Vec<Bytes32>`.
#[test]
fn vv_req_api_008_pending_bundle_ids_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let ids: Vec<Bytes32> = mempool.pending_bundle_ids();
    assert!(ids.is_empty());
}

/// Test: active_items() returns empty vec.
///
/// Proves signature `(&self) -> Vec<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_active_items_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let items: Vec<Arc<MempoolItem>> = mempool.active_items();
    assert!(items.is_empty());
}

/// Test: dependents_of() returns empty vec for unknown bundle.
///
/// Proves signature `(&self, &Bytes32) -> Vec<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_dependents_of_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let deps: Vec<Arc<MempoolItem>> = mempool.dependents_of(&Bytes32::default());
    assert!(deps.is_empty());
}

/// Test: ancestors_of() returns empty vec for unknown bundle.
///
/// Proves signature `(&self, &Bytes32) -> Vec<Arc<MempoolItem>>`.
#[test]
fn vv_req_api_008_ancestors_of_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    let ancestors: Vec<Arc<MempoolItem>> = mempool.ancestors_of(&Bytes32::default());
    assert!(ancestors.is_empty());
}

/// Test: len(), pending_len(), conflict_len() return 0.
///
/// Proves count methods exist with `(&self) -> usize` signatures.
#[test]
fn vv_req_api_008_count_methods() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.len(), 0);
    assert_eq!(mempool.pending_len(), 0);
    assert_eq!(mempool.conflict_len(), 0);
}

/// Test: is_empty() returns true on empty mempool.
///
/// Proves signature `(&self) -> bool`.
#[test]
fn vv_req_api_008_is_empty() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert!(mempool.is_empty());
}

/// Test: stats() returns MempoolStats.
///
/// Proves signature `(&self) -> MempoolStats`.
/// Already tested in API-006, but included here for API surface completeness.
#[test]
fn vv_req_api_008_stats() {
    let mempool = Mempool::new(DIG_TESTNET);
    let _stats: MempoolStats = mempool.stats();
}

/// Test: get_mempool_coin_record() returns None for unknown coin.
///
/// Proves signature `(&self, &Bytes32) -> Option<CoinRecord>`.
/// This method is for CPFP: look up coins created by active mempool items.
#[test]
fn vv_req_api_008_get_mempool_coin_record() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: Option<CoinRecord> = mempool.get_mempool_coin_record(&Bytes32::default());
    assert!(result.is_none());
}

/// Test: get_mempool_coin_creator() returns None for unknown coin.
///
/// Proves signature `(&self, &Bytes32) -> Option<Bytes32>`.
/// Returns the bundle ID that created a mempool coin (for CPFP dependency building).
#[test]
fn vv_req_api_008_get_mempool_coin_creator() {
    let mempool = Mempool::new(DIG_TESTNET);
    let result: Option<Bytes32> = mempool.get_mempool_coin_creator(&Bytes32::default());
    assert!(result.is_none());
}

/// Test: All query methods work on shared reference (&self).
///
/// Proves all queries use `&self` (not `&mut self`), enabling concurrent
/// reads. This is critical for the interior mutability design (Decision #1).
#[test]
fn vv_req_api_008_all_read_only() {
    let mempool = Mempool::new(DIG_TESTNET);
    // Call all methods on the same &self — no &mut needed
    let _ = mempool.get(&Bytes32::default());
    let _ = mempool.contains(&Bytes32::default());
    let _ = mempool.active_bundle_ids();
    let _ = mempool.pending_bundle_ids();
    let _ = mempool.active_items();
    let _ = mempool.dependents_of(&Bytes32::default());
    let _ = mempool.ancestors_of(&Bytes32::default());
    let _ = mempool.len();
    let _ = mempool.pending_len();
    let _ = mempool.conflict_len();
    let _ = mempool.is_empty();
    let _ = mempool.stats();
    let _ = mempool.get_mempool_coin_record(&Bytes32::default());
    let _ = mempool.get_mempool_coin_creator(&Bytes32::default());
}

// ── Behavioral tests on populated mempool ──────────────────────────────────
// Phase 2 (POL-001 through POL-010) is now complete. These tests verify
// that all query methods return correct data for a populated pool.

/// Test: get() returns the Arc<MempoolItem> for a submitted bundle.
///
/// Proves API-008: "Returns the Arc<MempoolItem> for the given bundle ID."
#[test]
fn vv_req_api_008_get_returns_item() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    let id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    let item: Option<Arc<MempoolItem>> = mempool.get(&id);
    assert!(item.is_some(), "get() must return the submitted item");
    assert_eq!(item.unwrap().spend_bundle_id, id);
}

/// Test: contains() returns true for a submitted bundle.
///
/// Proves API-008: "Returns true only for items in the active pool."
#[test]
fn vv_req_api_008_contains_returns_true() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin = make_coin(0x01, 100);
    let (bundle, cr) = nil_bundle(coin);
    let id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    assert!(
        mempool.contains(&id),
        "contains() must return true for active item"
    );
    assert!(
        !mempool.contains(&Bytes32::default()),
        "contains() must return false for unknown ID"
    );
}

/// Test: active_bundle_ids() returns all submitted IDs.
///
/// Proves API-008: "Returns all bundle IDs currently in the active pool."
#[test]
fn vv_req_api_008_active_bundle_ids_populated() {
    let mempool = Mempool::new(DIG_TESTNET);
    let mut expected_ids = vec![];
    for i in 0x01..=0x03u8 {
        let coin = make_coin(i, 100 * i as u64);
        let (bundle, cr) = nil_bundle(coin);
        expected_ids.push(bundle.name());
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    let ids = mempool.active_bundle_ids();
    assert_eq!(ids.len(), 3, "must return all 3 bundle IDs");
    for id in &expected_ids {
        assert!(
            ids.contains(id),
            "ID {:?} must be in active_bundle_ids()",
            id
        );
    }
}

/// Test: active_items() returns all active MempoolItem Arc references.
///
/// Proves API-008: "Returns Vec<Arc<MempoolItem>> for all active items."
#[test]
fn vv_req_api_008_active_items_populated() {
    let mempool = Mempool::new(DIG_TESTNET);
    for i in 0x01..=0x02u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    let items: Vec<Arc<MempoolItem>> = mempool.active_items();
    assert_eq!(items.len(), 2, "active_items() must return 2 items");
}

/// Test: dependents_of() returns direct children of a bundle.
///
/// Proves API-008: "Returns items that depend on (spend coins from) the given bundle."
#[test]
fn vv_req_api_008_dependents_of_returns_children() {
    use dig_clvm::{clvmr::Allocator, tree_hash, TreeHash};

    let mempool = Mempool::new(DIG_TESTNET);

    // Parent: pass-through puzzle creating one output coin.
    let mut a = Allocator::new();
    let nil = a.nil();
    let amount = a.new_atom(&[100u8]).unwrap();
    let ph = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();
    let op51 = a.new_atom(&[51u8]).unwrap();
    let tail = a.new_pair(amount, nil).unwrap();
    let mid = a.new_pair(ph, tail).unwrap();
    let cond = a.new_pair(op51, mid).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let hash: TreeHash = tree_hash(&a, prog);
    let bytes = dig_clvm::clvmr::serde::node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let puzzle_hash = Bytes32::from(hash);

    let parent_coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 100);
    let output_coin = Coin::new(parent_coin.coin_id(), NIL_PUZZLE_HASH, 100);
    let mut parent_cr = HashMap::new();
    parent_cr.insert(
        parent_coin.coin_id(),
        CoinRecord {
            coin: parent_coin,
            coinbase: false,
            confirmed_block_index: 1,
            spent: false,
            spent_block_index: 0,
            timestamp: 100,
        },
    );
    let parent_bundle = SpendBundle::new(
        vec![CoinSpend::new(parent_coin, puzzle, Program::default())],
        Signature::default(),
    );
    let parent_id = parent_bundle.name();
    mempool.submit(parent_bundle, &parent_cr, 0, 0).unwrap();

    // Child: spends the parent's output (mempool coin).
    let child_bundle = SpendBundle::new(
        vec![CoinSpend::new(
            output_coin,
            Program::default(),
            Program::default(),
        )],
        Signature::default(),
    );
    let child_id = child_bundle.name();
    mempool.submit(child_bundle, &HashMap::new(), 0, 0).unwrap();

    let deps = mempool.dependents_of(&parent_id);
    assert_eq!(deps.len(), 1, "parent must have exactly 1 dependent");
    assert_eq!(
        deps[0].spend_bundle_id, child_id,
        "dependent must be the child"
    );

    let ancestors = mempool.ancestors_of(&child_id);
    assert_eq!(ancestors.len(), 1, "child must have exactly 1 ancestor");
    assert_eq!(
        ancestors[0].spend_bundle_id, parent_id,
        "ancestor must be the parent"
    );
}

/// Test: len() and is_empty() track pool state correctly.
///
/// Proves API-008: "len() returns the number of active items."
#[test]
fn vv_req_api_008_len_tracks_state() {
    let mempool = Mempool::new(DIG_TESTNET);
    assert_eq!(mempool.len(), 0);
    assert!(mempool.is_empty());

    for i in 0x01..=0x03u8 {
        let coin = make_coin(i, 100);
        let (bundle, cr) = nil_bundle(coin);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }

    assert_eq!(mempool.len(), 3);
    assert!(!mempool.is_empty());
}
