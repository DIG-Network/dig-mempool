//! Two-phase admission pipeline and supporting logic.
//!
//! The admission pipeline is the core of `Mempool::submit()`:
//! - Phase 1 (lock-free): CLVM validation, timelock resolution, fee extraction
//! - Phase 2 (write lock): dependency resolution, conflict detection, RBF, insertion
//!
//! Sub-modules:
//! - `timelock`: Relative→absolute timelock resolution (ADM-006)
//! - `dedup`: Dedup/FF flag extraction (ADM-007)
//! - `pipeline`: Full two-phase pipeline orchestration (ADM-001..008)
//!
//! The pipeline logic itself lives in `crate::mempool` (Mempool::submit_inner)
//! and will be migrated here incrementally.

pub(crate) mod dedup;
pub(crate) mod pipeline;
pub(crate) mod timelock;
