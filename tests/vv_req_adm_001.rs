//! REQUIREMENT: ADM-001 — submit() Entry Point Signature
//!
//! Test-driven verification that `Mempool::submit()` exists with the correct
//! signature: `(&self, SpendBundle, &HashMap<Bytes32, CoinRecord>, u64, u64)
//! -> Result<SubmitResult, MempoolError>`.
//!
//! ## What this proves
//!
//! - `submit()` method exists on `Mempool` with exact signature from spec
//! - Takes `&self` (not `&mut self`) — interior mutability for concurrency
//! - `SpendBundle` is consumed by value (ownership transferred)
//! - `coin_records` is borrowed as `&HashMap<Bytes32, CoinRecord>`
//! - Returns `Result<SubmitResult, MempoolError>`
//! - `submit_with_policy()` variant exists and accepts `&dyn AdmissionPolicy`
//!
//! ## Scope Note
//!
//! ADM-001 only requires the **signature** to exist. The full admission pipeline
//! (CLVM validation, dedup, fee checks, etc.) will be wired in ADM-002 through
//! ADM-007. For now, submit() can return a placeholder result or error.
//!
//! ## Chia L1 Correspondence
//!
//! Mirrors `MempoolManager.add_spend_bundle()`:
//! https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L538
//!
//! Reference: docs/requirements/domains/admission/specs/ADM-001.md

use std::collections::HashMap;

use dig_clvm::{Bytes32, SpendBundle};
use dig_constants::DIG_TESTNET;
use dig_mempool::traits::AdmissionPolicy;
use dig_mempool::{CoinRecord, Mempool, MempoolError, SubmitResult};

/// Test: submit() compiles with the specified signature.
///
/// Proves ADM-001 acceptance criterion: "The method submit() exists on
/// Mempool with the exact signature specified."
///
/// We call submit() with the correct types. Compilation proves the
/// signature is correct. The result (success or error) depends on the
/// pipeline implementation (ADM-002+).
#[test]
fn vv_req_adm_001_submit_signature_compiles() {
    let mempool = Mempool::new(DIG_TESTNET);
    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // Call submit() — we only care that this compiles with correct types.
    // The result may be Ok or Err depending on validation (ADM-002).
    let _result: Result<SubmitResult, MempoolError> = mempool.submit(bundle, &coin_records, 0, 0);
}

/// Test: submit() takes &self (not &mut self).
///
/// Proves ADM-001 acceptance criterion: "The method takes &self (not
/// &mut self) for interior mutability support."
///
/// We call submit() twice on the same shared reference. If it required
/// &mut self, this wouldn't compile.
#[test]
fn vv_req_adm_001_submit_takes_shared_ref() {
    let mempool = Mempool::new(DIG_TESTNET);
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // Two calls on same &self — proves &self not &mut self
    let _r1 = mempool.submit(
        SpendBundle::new(vec![], dig_clvm::Signature::default()),
        &coin_records,
        0,
        0,
    );
    let _r2 = mempool.submit(
        SpendBundle::new(vec![], dig_clvm::Signature::default()),
        &coin_records,
        0,
        0,
    );
}

/// Test: submit() consumes SpendBundle by value.
///
/// Proves ADM-001 acceptance criterion: "SpendBundle is consumed by value."
/// After passing `bundle` to submit(), it can't be used again (moved).
/// This is verified at compile time — if SpendBundle were borrowed, we
/// could use it after the call.
#[test]
fn vv_req_adm_001_submit_consumes_bundle() {
    let mempool = Mempool::new(DIG_TESTNET);
    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    // bundle is moved into submit() — can't be used after this line
    let _result = mempool.submit(bundle, &coin_records, 0, 0);
    // let _ = bundle.name(); // This would fail to compile — bundle was moved
}

/// Test: submit_with_policy() variant exists.
///
/// Proves ADM-001 acceptance criterion: "submit_with_policy() exists
/// and accepts &dyn AdmissionPolicy."
///
/// This variant applies a caller-provided admission policy after all
/// standard checks pass.
#[test]
fn vv_req_adm_001_submit_with_policy_exists() {
    struct AcceptAll;
    impl AdmissionPolicy for AcceptAll {
        fn check(
            &self,
            _item: &dig_mempool::MempoolItem,
            _existing: &[std::sync::Arc<dig_mempool::MempoolItem>],
        ) -> Result<(), String> {
            Ok(())
        }
    }

    let mempool = Mempool::new(DIG_TESTNET);
    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();
    let policy = AcceptAll;

    // Call submit_with_policy() — compilation proves the signature
    let _result: Result<SubmitResult, MempoolError> =
        mempool.submit_with_policy(bundle, &coin_records, 0, 0, &policy);
}

/// Test: submit() returns Result<SubmitResult, MempoolError>.
///
/// Proves the return type is correct by pattern matching on both Ok and Err.
#[test]
fn vv_req_adm_001_return_type_matchable() {
    let mempool = Mempool::new(DIG_TESTNET);
    let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();

    let result = mempool.submit(bundle, &coin_records, 0, 0);

    // Pattern match proves the return type
    match result {
        Ok(SubmitResult::Success) => { /* active pool admission */ }
        Ok(SubmitResult::Pending { assert_height: _ }) => { /* pending pool */ }
        Err(_e) => { /* validation or admission failure */ }
    }
}

/// Test: submit() is callable from a different thread.
///
/// Proves the Mempool is Send + Sync and submit() works across threads.
/// This validates the interior mutability design (Decision #1).
#[test]
fn vv_req_adm_001_submit_concurrent() {
    use std::sync::Arc;
    use std::thread;

    let mempool = Arc::new(Mempool::new(DIG_TESTNET));
    let coin_records: HashMap<Bytes32, CoinRecord> = HashMap::new();
    let cr = Arc::new(coin_records);

    let m1 = Arc::clone(&mempool);
    let cr1 = Arc::clone(&cr);
    let h1 = thread::spawn(move || {
        let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
        m1.submit(bundle, &cr1, 0, 0)
    });

    let m2 = Arc::clone(&mempool);
    let cr2 = Arc::clone(&cr);
    let h2 = thread::spawn(move || {
        let bundle = SpendBundle::new(vec![], dig_clvm::Signature::default());
        m2.submit(bundle, &cr2, 0, 0)
    });

    // Both threads complete without deadlock
    let _r1 = h1.join().unwrap();
    let _r2 = h2.join().unwrap();
}
