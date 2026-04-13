//! dig-mempool: DIG L2 Transaction Mempool
//!
//! Fee-prioritized, conflict-aware transaction pool with CPFP support.
//! Accepts SpendBundle submissions, validates via dig-clvm, manages lifecycle,
//! and outputs selected MempoolItems for block candidate production.

/// Mempool configuration, constants, and builder pattern.
/// See: [API-003](docs/requirements/domains/crate_api/specs/API-003.md)
pub mod config;

/// Mempool error types (Clone + PartialEq for testability).
/// See: [API-004](docs/requirements/domains/crate_api/specs/API-004.md)
mod error;

/// MempoolItem — the central validated transaction type, stored as Arc.
/// See: [API-002](docs/requirements/domains/crate_api/specs/API-002.md)
pub mod item;

/// Core Mempool struct and constructors.
/// See: [API-001](docs/requirements/domains/crate_api/specs/API-001.md)
mod mempool;

/// Aggregate mempool statistics snapshot.
/// See: [API-006](docs/requirements/domains/crate_api/specs/API-006.md)
mod stats;

pub use config::{MempoolConfig, FPC_SCALE, MEMPOOL_BLOCK_BUFFER};
pub use error::MempoolError;
pub use item::{MempoolItem, SingletonLineageInfo};
pub use mempool::Mempool;
pub use stats::MempoolStats;

// Re-export key types from dig-clvm for convenience.
// These are the Chia ecosystem types the mempool operates on.
// Rule 6: Re-export, don't redefine.
pub use dig_clvm::{Bytes32, Coin, CoinRecord, SpendBundle, SpendResult};
pub use dig_constants::NetworkConstants;
