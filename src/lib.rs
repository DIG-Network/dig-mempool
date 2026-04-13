//! dig-mempool: DIG L2 Transaction Mempool
//!
//! Fee-prioritized, conflict-aware transaction pool with CPFP support.
//! Accepts SpendBundle submissions, validates via dig-clvm, manages lifecycle,
//! and outputs selected MempoolItems for block candidate production.

mod config;
mod error;
mod mempool;
mod stats;

pub use config::{MempoolConfig, FPC_SCALE, MEMPOOL_BLOCK_BUFFER};
pub use error::MempoolError;
pub use mempool::Mempool;
pub use stats::MempoolStats;

// Re-export key types from dig-clvm for convenience
pub use dig_clvm::{Bytes32, Coin, CoinRecord, SpendBundle, SpendResult};
pub use dig_constants::NetworkConstants;
