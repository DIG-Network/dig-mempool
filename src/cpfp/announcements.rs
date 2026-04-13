//! CPF-008 — Cross-bundle announcement validation (no-op).
//!
//! Per SPEC §5.9, assertions referencing non-ancestor bundles are not rejected
//! in the mempool — they may be satisfied by other bundles in the same block.
//! This validation is left to block validation (outside mempool scope).
//!
//! See: [`docs/requirements/domains/cpfp/specs/CPF-008.md`]
