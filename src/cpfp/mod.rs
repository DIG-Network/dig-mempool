//! Child-Pays-For-Parent (CPFP) dependency tracking.
//!
//! Sub-modules:
//! - `dependency`: dependency graph, package fee rates (CPF-001..005)
//! - `cascade`: recursive dependent eviction (CPF-007)
//! - `announcements`: cross-bundle announcement validation no-op (CPF-008)
//!
//! Note: The graph storage lives in `ActivePool` (src/pools/active.rs).
//! The admission-time logic (dependency resolution, depth enforcement, cycle
//! detection, package fee computation, descendant score updates) currently
//! lives inline in `Mempool::submit_inner` and will be migrated here.

pub(crate) mod announcements;
pub(crate) mod cascade;
pub(crate) mod dependency;
