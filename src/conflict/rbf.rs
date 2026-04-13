//! CFR-002..006 — Replace-by-Fee (RBF) rules.
//!
//! When a conflict is detected, the incoming bundle must satisfy all RBF rules:
//! - CFR-002: Superset rule (removals ⊇ all conflict removals)
//! - CFR-003: FPC strictly higher than aggregate conflict FPC
//! - CFR-004: Minimum absolute fee bump (default 10M mojos)
//! - CFR-005: Cache failed bundles in conflict cache
//! - CFR-006: Remove conflicting items + cascade-evict dependents
//!
//! See: [`docs/requirements/domains/conflict_resolution/`]
//!
//! Note: The implementation currently lives inline in `Mempool::submit_inner`.
