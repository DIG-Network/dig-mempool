//! CPF-007 — Cascade eviction of dependent bundles.
//!
//! When a bundle is removed (e.g., by RBF), all its transitive dependents
//! must also be removed (children before parents, DFS).
//!
//! See: [`docs/requirements/domains/cpfp/specs/CPF-007.md`]
//!
//! Note: Implementation lives in `ActivePool::cascade_evict` (src/pools/active.rs).
