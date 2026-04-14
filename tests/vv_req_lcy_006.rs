//! REQUIREMENT: LCY-006 — RemovalReason Enum
//!
//! Proves RemovalReason:
//! - All seven variants are constructible
//! - Derives Debug, Clone, PartialEq
//! - Clone produces equal value
//! - PartialEq correctly compares values (including fields)
//! - Debug formatting produces non-empty output
//! - Confirmed reason passed to hook when item is block-confirmed
//! - CascadeEvicted { parent_id } passed with correct parent ID
//! - ReplacedByFee has replacement_id field (structural test)
//! - Cleared reason passed to hook when clear() is called
//!
//! Reference: docs/requirements/domains/lifecycle/specs/LCY-006.md

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dig_clvm::{
    clvmr::{serde::node_to_bytes, Allocator},
    tree_hash, Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle, TreeHash,
};
use dig_constants::DIG_TESTNET;
use dig_mempool::Mempool;
use dig_mempool::{MempoolEventHook, RemovalReason};
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

fn make_pass_through_puzzle(amount: u64) -> (Program, Bytes32) {
    let mut a = Allocator::new();
    let nil = a.nil();
    let amount_atom = a.new_atom(&clvm_encode_u64(amount)).unwrap();
    let ph_atom = a.new_atom(NIL_PUZZLE_HASH.as_ref()).unwrap();
    let op_atom = a.new_atom(&[51u8]).unwrap();
    let tail = a.new_pair(amount_atom, nil).unwrap();
    let mid = a.new_pair(ph_atom, tail).unwrap();
    let cond = a.new_pair(op_atom, mid).unwrap();
    let cond_list = a.new_pair(cond, nil).unwrap();
    let q = a.new_atom(&[1u8]).unwrap();
    let prog = a.new_pair(q, cond_list).unwrap();
    let bytes = node_to_bytes(&a, prog).unwrap();
    let puzzle = Program::new(bytes.into());
    let hash: TreeHash = tree_hash(&a, prog);
    (puzzle, Bytes32::from(hash))
}

/// Minimal hook that records removal events.
struct RemovalHook {
    events: Mutex<Vec<(Bytes32, RemovalReason)>>,
}

impl RemovalHook {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
        })
    }
}

impl MempoolEventHook for RemovalHook {
    fn on_item_removed(&self, bundle_id: &Bytes32, reason: RemovalReason) {
        self.events.lock().unwrap().push((*bundle_id, reason));
    }
}

/// All seven RemovalReason variants are constructible.
///
/// Proves LCY-006: "All seven variants are present."
#[test]
fn vv_req_lcy_006_all_variants_constructible() {
    let _confirmed = RemovalReason::Confirmed;
    let _rbf = RemovalReason::ReplacedByFee {
        replacement_id: Bytes32::from([0x01; 32]),
    };
    let _cascade = RemovalReason::CascadeEvicted {
        parent_id: Bytes32::from([0x02; 32]),
    };
    let _expired = RemovalReason::Expired;
    let _capacity = RemovalReason::CapacityEviction;
    let _explicit = RemovalReason::ExplicitRemoval;
    let _cleared = RemovalReason::Cleared;
    // Compilation is the proof.
}

/// Clone produces an equal value.
///
/// Proves LCY-006: "Derives Clone."
#[test]
fn vv_req_lcy_006_clone_works() {
    let reason = RemovalReason::CascadeEvicted {
        parent_id: Bytes32::from([0xAB; 32]),
    };
    let cloned = reason.clone();
    assert_eq!(reason, cloned, "cloned RemovalReason must equal original");

    let reason2 = RemovalReason::ReplacedByFee {
        replacement_id: Bytes32::from([0xCD; 32]),
    };
    assert_eq!(reason2.clone(), reason2);
}

/// PartialEq correctly compares variants including field values.
///
/// Proves LCY-006: "Derives PartialEq."
#[test]
fn vv_req_lcy_006_partial_eq_works() {
    let r1 = RemovalReason::Confirmed;
    let r2 = RemovalReason::Confirmed;
    assert_eq!(r1, r2, "identical Confirmed variants must be equal");

    let r3 = RemovalReason::Expired;
    assert_ne!(r1, r3, "Confirmed != Expired");

    let r4 = RemovalReason::CascadeEvicted {
        parent_id: Bytes32::from([0x01; 32]),
    };
    let r5 = RemovalReason::CascadeEvicted {
        parent_id: Bytes32::from([0x01; 32]),
    };
    assert_eq!(r4, r5, "same CascadeEvicted must be equal");

    let r6 = RemovalReason::CascadeEvicted {
        parent_id: Bytes32::from([0x02; 32]),
    };
    assert_ne!(
        r4, r6,
        "CascadeEvicted with different parent_id must not be equal"
    );
}

/// Debug formatting produces a non-empty, readable string.
///
/// Proves LCY-006: "Derives Debug."
#[test]
fn vv_req_lcy_006_debug_formatting() {
    let r = RemovalReason::Confirmed;
    let s = format!("{r:?}");
    assert!(!s.is_empty(), "Debug output must be non-empty");
    assert!(s.contains("Confirmed"), "Debug must name the variant");

    let r2 = RemovalReason::CascadeEvicted {
        parent_id: Bytes32::from([0xFF; 32]),
    };
    let s2 = format!("{r2:?}");
    assert!(
        s2.contains("CascadeEvicted"),
        "Debug must name CascadeEvicted"
    );
    assert!(s2.contains("parent_id"), "Debug must include field name");
}

/// ReplacedByFee contains replacement_id field.
///
/// Proves LCY-006: "ReplacedByFee contains replacement_id: Bytes32."
#[test]
fn vv_req_lcy_006_replaced_by_fee_has_replacement_id() {
    let id = Bytes32::from([0x42; 32]);
    let reason = RemovalReason::ReplacedByFee { replacement_id: id };
    if let RemovalReason::ReplacedByFee { replacement_id } = reason {
        assert_eq!(replacement_id, id);
    } else {
        panic!("must be ReplacedByFee variant");
    }
}

/// CascadeEvicted contains parent_id field.
///
/// Proves LCY-006: "CascadeEvicted contains parent_id: Bytes32."
#[test]
fn vv_req_lcy_006_cascade_evicted_has_parent_id() {
    let id = Bytes32::from([0x55; 32]);
    let reason = RemovalReason::CascadeEvicted { parent_id: id };
    if let RemovalReason::CascadeEvicted { parent_id } = reason {
        assert_eq!(parent_id, id);
    } else {
        panic!("must be CascadeEvicted variant");
    }
}

/// Hook receives Confirmed reason when a confirmed block removes an item.
///
/// Proves LCY-006: "Confirmed: item's coins were spent in a confirmed block."
#[test]
fn vv_req_lcy_006_confirmed_reason_on_block() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = RemovalHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let coin = make_coin(0x01, 1000);
    let (bundle, cr) = nil_bundle(coin);
    let bundle_id = bundle.name();
    mempool.submit(bundle, &cr, 0, 0).unwrap();

    mempool.on_new_block(1, 100, &[coin.coin_id()], &[]);

    let events = hook.events.lock().unwrap();
    let event = events.iter().find(|(id, _)| *id == bundle_id).unwrap();
    assert_eq!(
        event.1,
        RemovalReason::Confirmed,
        "must receive Confirmed reason"
    );
}

/// Hook receives CascadeEvicted { parent_id } with correct parent when child is cascade-evicted.
///
/// Proves LCY-006: "CascadeEvicted contains the removed parent's bundle ID."
#[test]
fn vv_req_lcy_006_cascade_evicted_parent_id_correct() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = RemovalHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    // Parent bundle (creates output coin)
    let (puzzle, puzzle_hash) = make_pass_through_puzzle(500);
    let parent_coin = Coin::new(Bytes32::from([0x01; 32]), puzzle_hash, 500);
    let output_coin = Coin::new(parent_coin.coin_id(), NIL_PUZZLE_HASH, 500);
    let parent_bundle = SpendBundle::new(
        vec![CoinSpend::new(parent_coin, puzzle, Program::default())],
        Signature::default(),
    );
    let parent_id = parent_bundle.name();
    let mut cr = HashMap::new();
    cr.insert(parent_coin.coin_id(), coin_record(parent_coin));
    mempool.submit(parent_bundle, &cr, 0, 0).unwrap();

    // Child bundle (CPFP)
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

    // Confirm the parent.
    mempool.on_new_block(1, 100, &[parent_coin.coin_id()], &[]);

    let events = hook.events.lock().unwrap();
    let child_event = events.iter().find(|(id, _)| *id == child_id).unwrap();

    assert_eq!(
        child_event.1,
        RemovalReason::CascadeEvicted { parent_id },
        "child must have CascadeEvicted {{ parent_id: parent_id }}"
    );
}

/// Hook receives Cleared reason for all items when clear() is called.
///
/// Proves LCY-006: "Cleared: mempool was cleared for reorg recovery via clear()."
#[test]
fn vv_req_lcy_006_cleared_reason_on_clear() {
    let mempool = Mempool::new(DIG_TESTNET);
    let hook = RemovalHook::new();
    mempool.add_event_hook(Arc::clone(&hook) as Arc<dyn MempoolEventHook>);

    let coin1 = make_coin(0x01, 1000);
    let coin2 = make_coin(0x02, 2000);
    let (b1, cr1) = nil_bundle(coin1);
    let (b2, cr2) = nil_bundle(coin2);
    let id1 = b1.name();
    let id2 = b2.name();
    let mut combined = cr1;
    combined.extend(cr2);
    mempool.submit(b1, &combined, 0, 0).unwrap();
    mempool.submit(b2, &combined, 0, 0).unwrap();

    // Clear the events from on_item_added (not applicable here, but for clarity)
    // Note: RemovalHook only records removals, so no clear needed.
    mempool.clear();

    let events = hook.events.lock().unwrap();
    assert_eq!(events.len(), 2, "must fire 2 Cleared events: {events:?}");

    let ids: Vec<Bytes32> = events.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&id1));
    assert!(ids.contains(&id2));

    for (_, reason) in events.iter() {
        assert_eq!(
            *reason,
            RemovalReason::Cleared,
            "all reasons must be Cleared"
        );
    }
}

/// RemovalReason is exported from the crate root (publicly accessible).
///
/// Proves LCY-006: "RemovalReason is publicly exported from the crate."
#[test]
fn vv_req_lcy_006_publicly_exported() {
    // If the import at the top of this file compiles, the type is exported.
    let _r: RemovalReason = RemovalReason::Confirmed;
    // Compilation is the proof.
}
