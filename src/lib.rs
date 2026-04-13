//! # dig-mempool: DIG L2 Transaction Mempool
//!
//! Fee-prioritized, conflict-aware transaction pool with CPFP support
//! for the DIG Network Layer 2 blockchain.
//!
//! ## Crate Boundary
//!
//! **Inputs:** Raw `SpendBundle` + `CoinRecord`s (via `submit()` / `submit_batch()`)
//! **Outputs:** `Vec<Arc<MempoolItem>>` (via `select_for_block()`)
//!
//! The mempool validates bundles internally via `dig-clvm::validate_spend_bundle()`
//! (CLVM dry-run + BLS signature verification), then manages their lifecycle:
//! ordering, conflicts, timelocks, CPFP dependencies, eviction, and block
//! candidate selection.
//!
//! The mempool does **not** perform block building, block validation, or
//! singleton lineage rebasing. Those are the caller's responsibility.
//!
//! ## Architecture
//!
//! - [`Mempool`] — Core struct. Thread-safe via interior mutability (`&self` + `RwLock`).
//! - [`MempoolItem`] — Validated transaction stored as `Arc<MempoolItem>`.
//! - [`MempoolConfig`] — All tuneable parameters with builder pattern.
//! - [`MempoolError`] — Error enum (`Clone + PartialEq` for testability).
//! - [`MempoolStats`] — Aggregate statistics snapshot.
//!
//! ## Design Derivation
//!
//! Derived from Chia's production mempool:
//! - [`mempool.py`](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py)
//! - [`mempool_manager.py`](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py)
//!
//! ## Spec Reference
//!
//! - [SPEC.md](docs/resources/SPEC.md) — Master specification (2200+ lines)
//! - [Requirements](docs/requirements/README.md) — 61 requirements across 8 domains

/// Mempool configuration, constants, and builder pattern.
/// See: [API-003](docs/requirements/domains/crate_api/specs/API-003.md)
pub mod config;

/// Mempool error types (Clone + PartialEq for testability).
/// See: [API-004](docs/requirements/domains/crate_api/specs/API-004.md)
mod error;

/// MempoolItem — the central validated transaction type, stored as Arc.
/// See: [API-002](docs/requirements/domains/crate_api/specs/API-002.md)
pub mod item;

/// Admission pipeline: CLVM validation, timelock resolution, fee extraction.
/// See: [ADM-001..008](docs/requirements/domains/admission/)
mod admission;

/// Conflict detection and Replace-by-Fee (RBF) rules.
/// See: [CFR-001..006](docs/requirements/domains/conflict_resolution/)
mod conflict;

/// Child-Pays-For-Parent (CPFP) dependency tracking and cascade eviction.
/// See: [CPF-001..008](docs/requirements/domains/cpfp/)
mod cpfp;

/// Core Mempool struct and constructors.
/// See: [API-001](docs/requirements/domains/crate_api/specs/API-001.md)
mod mempool;

/// Pool storage: active pool, pending pool, conflict cache, seen cache.
/// See: [POL-001..010](docs/requirements/domains/pools/)
mod pools;

/// Block candidate selection: greedy strategies + topological ordering.
/// See: [SEL-001..008](docs/requirements/domains/selection/)
mod selection;

/// Fee estimation: FeeTracker, BlockFeeData, FeeTrackerStats, estimate_fee_rate.
/// See: [FEE-002..005](docs/requirements/domains/fee_estimation/)
pub mod fee;

/// Aggregate mempool statistics snapshot.
/// See: [API-006](docs/requirements/domains/crate_api/specs/API-006.md)
mod stats;

/// Submission result types (Success / Pending).
/// See: [API-005](docs/requirements/domains/crate_api/specs/API-005.md)
mod submit;

/// Extension traits: AdmissionPolicy, BlockSelectionStrategy, MempoolEventHook, RemovalReason.
/// See: [API-007](docs/requirements/domains/crate_api/specs/API-007.md)
pub mod traits;

pub use config::{MempoolConfig, FPC_SCALE, MEMPOOL_BLOCK_BUFFER};
pub use error::MempoolError;
pub use fee::{BlockFeeData, FeeEstimatorState, FeeTrackerStats, SerializedBucket};
pub use item::{MempoolItem, SingletonLineageInfo};
pub use mempool::{Mempool, MempoolSnapshot};
pub use stats::MempoolStats;
pub use submit::{ConfirmedBundleInfo, RetryBundles, SubmitResult};
pub use traits::{MempoolEventHook, RemovalReason};

// Re-export key types from dig-clvm for convenience.
// These are the Chia ecosystem types the mempool operates on.
//
// Rationale (Rule 6 — Re-export, don't redefine):
// Consumers of dig-mempool should not need to add dig-clvm or
// chia-protocol to their Cargo.toml for basic mempool operations.
// These re-exports provide the essential types needed for submit()
// and select_for_block() without additional dependencies.
pub use dig_clvm::{Bytes32, Coin, CoinRecord, SpendBundle, SpendResult};
pub use dig_constants::NetworkConstants;
