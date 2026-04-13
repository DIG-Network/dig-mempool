//! ADM-001..008 — Two-phase admission pipeline.
//!
//! Orchestrates the full admission flow:
//! - Phase 0: Dedup check (ADM-003)
//! - Phase 1: Lock-free CLVM validation (ADM-002), fee extraction (ADM-004),
//!   virtual cost (ADM-005), timelock resolution (ADM-006),
//!   dedup/FF flags (ADM-007)
//! - Phase 2: Dependency resolution (CPF-002), conflict detection (CFR-001..005),
//!   RBF (CFR-002..006), pool insertion (POL-001..008)
//! - Batch submission (ADM-008)
//!
//! See: [`docs/requirements/domains/admission/`]
//!
//! Note: The full pipeline currently lives in `Mempool::submit_inner` in
//! `crate::mempool`. It will be migrated here incrementally.
