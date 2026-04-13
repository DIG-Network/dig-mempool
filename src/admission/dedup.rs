//! ADM-007 — Dedup/FF flag extraction from OwnedSpendConditions.
//!
//! Reads `ELIGIBLE_FOR_DEDUP` (0x1) and `ELIGIBLE_FOR_FF` (0x4) flags set
//! by chia-consensus's MempoolVisitor during CLVM execution.
//!
//! See: [`docs/requirements/domains/admission/specs/ADM-007.md`]
//!
//! Note: The implementation currently lives inline in `Mempool::submit_inner`.
//! Functions here will replace that inline code incrementally.
