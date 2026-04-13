//! REQUIREMENT: POL-010 — Concurrency
//!
//! Test-driven verification of the mempool's fine-grained locking model.
//!
//! ## What this proves
//!
//! - `Mempool` is `Send + Sync` (compile-time check)
//! - All public methods take `&self`, not `&mut self` (interior mutability)
//! - Multiple reader threads access the pool concurrently without deadlock
//! - A submitter thread and reader threads run concurrently without deadlock
//! - BLS cache (Mutex) serializes concurrent submissions — both succeed
//! - Multiple concurrent submitters + readers complete under sustained load
//! - `submit_batch()` correctly inserts all bundles in a single call
//! - Phase 1 (CLVM validation) does not stall concurrent reads
//!
//! ## Architecture verified
//!
//! The Mempool struct holds:
//! - `pool: RwLock<ActivePool>`    — protects items, coin_index, etc.
//! - `pending: RwLock<PendingPool>` — protects pending items
//! - `conflict: RwLock<ConflictCache>` — protects conflict cache
//! - `seen_cache: RwLock<SeenCache>` — protects seen-bundle dedup
//! - `bls_cache: Mutex<BlsCache>` — serializes BLS verification
//!
//! Reference: docs/requirements/domains/pools/specs/POL-010.md

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use dig_clvm::{Bytes32, Coin, CoinRecord, CoinSpend, Program, Signature, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::{Mempool, SubmitResult};
use hex_literal::hex;

/// SHA-256 tree hash of `Program::default()` = the nil atom (0x80).
const NIL_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"
));

/// Build a deterministic nil-puzzle bundle using `parent_prefix` as all 32
/// bytes of the parent coin ID. Each distinct prefix produces a distinct
/// `SpendBundle::name()`, enabling collision-free multi-bundle tests.
fn nil_bundle(parent_prefix: u8, amount: u64) -> (SpendBundle, HashMap<Bytes32, CoinRecord>) {
    let coin = Coin::new(Bytes32::from([parent_prefix; 32]), NIL_PUZZLE_HASH, amount);
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
    (bundle, cr)
}

// ── Send + Sync compile check ─────────────────────────────────────────────

/// Compile-time check: `Mempool` implements `Send + Sync + 'static`.
///
/// Proves POL-010: "The Mempool struct MUST implement Send + Sync."
/// No runtime assertions — if this function compiles, the requirement is met.
/// A violation causes: "the trait `Send` is not implemented for Mempool".
#[test]
fn vv_req_pol_010_send_sync_compile_check() {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<Mempool>();
}

// ── Interior mutability ───────────────────────────────────────────────────

/// Compile-time check: all public methods take `&self`.
///
/// Proves POL-010: "All public methods accept &self (interior mutability)."
/// Calls every significant public method via a shared reference `&Mempool`.
/// If any method required `&mut self`, this test would fail to compile.
#[test]
fn vv_req_pol_010_interior_mutability() {
    let mempool = Mempool::new(DIG_TESTNET);

    // Acquire a shared reference — only &self methods are callable
    let shared: &Mempool = &mempool;

    // Query methods (read-only)
    let _ = shared.len();
    let _ = shared.is_empty();
    let _ = shared.stats();
    let _ = shared.get(&Bytes32::default());
    let _ = shared.contains(&Bytes32::default());
    let _ = shared.active_bundle_ids();
    let _ = shared.pending_bundle_ids();
    let _ = shared.active_items();
    let _ = shared.pending_len();
    let _ = shared.conflict_len();
    let _ = shared.get_mempool_coin_creator(&Bytes32::default());
    let _ = shared.get_pending_coin_spender(&Bytes32::default());
    let _ = shared.dedup_index_len();
    let _ = shared.get_dedup_bearer(&Bytes32::default(), &Bytes32::default());

    // Mutating methods (still take &self due to interior mutability)
    let (bundle, cr) = nil_bundle(0x01, 1);
    let _ = shared.submit(bundle, &cr, 0, 0);
    shared.clear();
}

// ── Concurrent reads ──────────────────────────────────────────────────────

/// Integration test: 8 reader threads access the mempool concurrently.
///
/// Proves POL-010: "select_for_block(), queries (get, contains, stats) only
/// acquire read locks, allowing full concurrency with other reads."
/// All threads complete without deadlock; final item count is stable.
#[test]
fn vv_req_pol_010_concurrent_reads() {
    let mempool = Arc::new(Mempool::new(DIG_TESTNET));

    // Populate the pool
    for i in 0u8..8 {
        let (bundle, cr) = nil_bundle(i, 1);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }
    let expected_len = mempool.len();
    assert_eq!(expected_len, 8);

    // Spawn 8 reader threads, each issuing 50 queries
    let handles: Vec<_> = (0..8u8)
        .map(|_| {
            let m = Arc::clone(&mempool);
            thread::spawn(move || {
                for _ in 0..50 {
                    let _ = m.len();
                    let _ = m.is_empty();
                    let _ = m.stats();
                    let _ = m.active_items();
                    let _ = m.active_bundle_ids();
                    let _ = m.contains(&Bytes32::default());
                    let _ = m.get(&Bytes32::default());
                    let _ = m.pending_len();
                    let _ = m.conflict_len();
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("Reader thread panicked");
    }

    // Pool should be unchanged — reads are non-destructive
    assert_eq!(
        mempool.len(),
        expected_len,
        "Pool size should be unchanged after concurrent reads"
    );
}

// ── Concurrent submit + read ──────────────────────────────────────────────

/// Integration test: writer and reader threads run concurrently without deadlock.
///
/// Proves POL-010: "Phase 2 (write lock)... All Phase 2 operations are fast
/// HashMap inserts/removes — no CLVM execution under this lock."
/// The reader thread issues 200 rapid queries while the writer submits
/// 16 bundles. Neither deadlocks.
#[test]
fn vv_req_pol_010_concurrent_submit_and_read() {
    let mempool = Arc::new(Mempool::new(DIG_TESTNET));

    let m_reader = Arc::clone(&mempool);
    let reader = thread::spawn(move || {
        for _ in 0..200 {
            let _ = m_reader.len();
            let _ = m_reader.stats();
            let _ = m_reader.active_items();
            let _ = m_reader.active_bundle_ids();
        }
    });

    let m_writer = Arc::clone(&mempool);
    let writer = thread::spawn(move || {
        for i in 0u8..16 {
            let (bundle, cr) = nil_bundle(i, 1);
            let _ = m_writer.submit(bundle, &cr, 0, 0);
        }
    });

    reader.join().expect("Reader thread panicked");
    writer.join().expect("Writer thread panicked");
    // No assertion on final count — the goal is deadlock-freedom
}

// ── BLS cache: concurrent submissions serialize through Mutex ─────────────

/// Integration test: two concurrent submissions both need BLS verification.
///
/// Proves POL-010: "bls_lock is a Mutex protecting the BLS cache."
/// Both threads submit different bundles concurrently. The BLS Mutex
/// serializes access; both submissions complete successfully.
#[test]
fn vv_req_pol_010_concurrent_bls_cache_access() {
    let mempool = Arc::new(Mempool::new(DIG_TESTNET));

    let m1 = Arc::clone(&mempool);
    let m2 = Arc::clone(&mempool);

    let (b1, cr1) = nil_bundle(0x01, 1);
    let (b2, cr2) = nil_bundle(0x02, 1);

    let t1 = thread::spawn(move || m1.submit(b1, &cr1, 0, 0));
    let t2 = thread::spawn(move || m2.submit(b2, &cr2, 0, 0));

    let r1 = t1.join().expect("Thread 1 panicked");
    let r2 = t2.join().expect("Thread 2 panicked");

    assert!(
        r1.is_ok(),
        "Concurrent submission 1 failed: {:?}",
        r1
    );
    assert!(
        r2.is_ok(),
        "Concurrent submission 2 failed: {:?}",
        r2
    );
    assert_eq!(
        mempool.len(),
        2,
        "Both concurrently submitted bundles should be in the pool"
    );
}

// ── Lock ordering: no deadlock under sustained concurrent load ─────────────

/// Integration test: 4 reader threads + 4 submitter threads run simultaneously.
///
/// Proves POL-010: "Lock ordering prevents deadlocks."
/// Lock acquisition order (pool_lock → pending_lock → conflict_lock) is
/// respected by the codebase. This test verifies no deadlock occurs under
/// realistic concurrent load (64 submissions + 400 reads, interleaved).
#[test]
fn vv_req_pol_010_lock_ordering_no_deadlock() {
    let mempool = Arc::new(Mempool::new(DIG_TESTNET));
    let mut handles: Vec<_> = Vec::new();

    // 4 reader threads, each doing 100 queries
    for _ in 0..4 {
        let m = Arc::clone(&mempool);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                let _ = m.len();
                let _ = m.stats();
                let _ = m.active_bundle_ids();
                let _ = m.pending_len();
                let _ = m.conflict_len();
            }
        }));
    }

    // 4 submitter threads, each submitting 16 bundles
    // Prefixes: 0x00..0x0F, 0x10..0x1F, 0x20..0x2F, 0x30..0x3F
    for s in 0..4u8 {
        let m = Arc::clone(&mempool);
        handles.push(thread::spawn(move || {
            let base = s * 16;
            for i in 0u8..16 {
                let (bundle, cr) = nil_bundle(base + i, 1);
                let _ = m.submit(bundle, &cr, 0, 0);
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread panicked — possible deadlock or data race");
    }

    // All 64 bundles should be admitted (no conflicts, distinct coins)
    assert_eq!(
        mempool.len(),
        64,
        "All 64 bundles from 4 submitter threads should be in pool"
    );
}

// ── submit_batch correctness ──────────────────────────────────────────────

/// Integration test: `submit_batch()` correctly submits all bundles.
///
/// Proves POL-010 (implementation notes): "submit_batch() amortizes lock
/// acquisition across all insertions in a single call."
/// All 10 bundles succeed; pool count equals 10 after the call.
#[test]
fn vv_req_pol_010_batch_correct_insertion() {
    let mempool = Mempool::new(DIG_TESTNET);

    let mut bundles = Vec::new();
    let mut coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    for i in 0u8..10 {
        let (bundle, cr) = nil_bundle(i, 1);
        coin_records.extend(cr);
        bundles.push(bundle);
    }

    let results = mempool.submit_batch(bundles, &coin_records, 0, 0);

    assert_eq!(results.len(), 10, "submit_batch should return one result per bundle");

    let success_count = results
        .iter()
        .filter(|r| matches!(r, Ok(SubmitResult::Success)))
        .count();
    assert_eq!(
        success_count,
        10,
        "All 10 bundles should succeed; results: {:?}",
        results
    );
    assert_eq!(
        mempool.len(),
        10,
        "Active pool should contain all 10 submitted bundles"
    );
}

// ── Phase 1 lock-free: pool readable during CLVM validation ───────────────

/// Integration test: the active pool is readable while another submission
/// is in flight (verifies Phase 1 does not hold pool_lock).
///
/// Proves POL-010: "CLVM validation (Phase 1) does NOT hold pool_lock."
/// We pre-populate the pool, then concurrently submit a new bundle (which
/// triggers CLVM validation) while a reader counts pool items. The reader
/// should complete without blocking on the writer's CLVM work.
///
/// Note: timing-based tests can be non-deterministic. This test focuses
/// on deadlock freedom and correctness of the final state.
#[test]
fn vv_req_pol_010_phase1_does_not_stall_readers() {
    let mempool = Arc::new(Mempool::new(DIG_TESTNET));

    // Pre-populate with 4 items
    for i in 0u8..4 {
        let (bundle, cr) = nil_bundle(i, 1);
        mempool.submit(bundle, &cr, 0, 0).unwrap();
    }
    assert_eq!(mempool.len(), 4);

    // Reader: reads the pool 100 times while the writer is active
    let m_reader = Arc::clone(&mempool);
    let reader = thread::spawn(move || {
        let mut observations = Vec::with_capacity(100);
        for _ in 0..100 {
            observations.push(m_reader.len());
        }
        observations
    });

    // Writer: submits a new bundle (triggers Phase 1 CLVM validation)
    let m_writer = Arc::clone(&mempool);
    let writer = thread::spawn(move || {
        let (bundle, cr) = nil_bundle(0xFF, 1);
        m_writer.submit(bundle, &cr, 0, 0).unwrap();
    });

    let observations = reader.join().expect("Reader panicked");
    writer.join().expect("Writer panicked");

    // Reader should have observed 4 or 5 items (4 before + 1 after writer)
    // Never less than 4 (no removals), never more than 5 (only one insert)
    for &count in &observations {
        assert!(
            count == 4 || count == 5,
            "Reader observed unexpected count {} (expected 4 or 5)",
            count
        );
    }
    assert_eq!(mempool.len(), 5, "Final pool should have 5 items");
}
