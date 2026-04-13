//! Conflict detection and Replace-by-Fee (RBF) rules.
//!
//! Sub-modules:
//! - `detection`: O(1) coin conflict detection via `coin_index` (CFR-001)
//! - `rbf`: Superset rule, FPC comparison, fee bump enforcement (CFR-002..006)
//!
//! Note: The implementation currently lives inline in `Mempool::submit_inner`.
//! Functions here will replace that inline code incrementally.

pub(crate) mod detection;
pub(crate) mod rbf;
