//! CPF-001..006 — Dependency graph and package fee rate computation.
//!
//! Tracks which active bundles depend on other active bundles (CPFP chains).
//! Package fees aggregate ancestor fees to give a child an accurate
//! fee-per-cost that reflects the full cost of getting the chain confirmed.
//!
//! See: [`docs/requirements/domains/cpfp/`]
//!
//! Note: Graph storage is in `ActivePool` fields `dependencies`/`dependents`.
//! Admission-time logic is inline in `Mempool::submit_inner`.
