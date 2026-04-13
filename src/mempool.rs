//! Core Mempool struct and constructors.

use std::sync::RwLock;

use dig_constants::NetworkConstants;

use crate::config::MempoolConfig;
use crate::stats::MempoolStats;

/// Fee-prioritized, conflict-aware transaction mempool.
///
/// Thread-safe via interior mutability (`&self` methods with internal `RwLock`).
/// Implements `Send + Sync`.
pub struct Mempool {
    #[allow(dead_code)]
    constants: NetworkConstants,
    config: MempoolConfig,
    // Active pool item count — will be expanded with full pool data structures
    active_count: RwLock<usize>,
}

impl Mempool {
    /// Create a mempool with default configuration for the given network.
    pub fn new(constants: NetworkConstants) -> Self {
        Self::with_config(constants, MempoolConfig::default())
    }

    /// Create a mempool with custom configuration.
    pub fn with_config(constants: NetworkConstants, config: MempoolConfig) -> Self {
        Self {
            constants,
            config,
            active_count: RwLock::new(0),
        }
    }

    /// Number of active items in the mempool.
    pub fn len(&self) -> usize {
        *self.active_count.read().unwrap()
    }

    /// Whether the active mempool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Aggregate mempool statistics.
    pub fn stats(&self) -> MempoolStats {
        MempoolStats::empty(self.config.max_total_cost)
    }
}
