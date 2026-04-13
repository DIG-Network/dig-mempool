//! ADM-006 — Timelock resolution (relative → absolute).
//!
//! Resolves per-spend relative timelocks using coin record confirmation data,
//! checks for impossible constraints, and determines the final absolute
//! timelock values for the bundle.
//!
//! See: [`docs/requirements/domains/admission/specs/ADM-006.md`]
//!
//! Note: The implementation currently lives inline in `Mempool::submit_inner`.
//! Functions here will replace that inline code incrementally.
