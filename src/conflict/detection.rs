//! CFR-001 — Coin conflict detection via the active pool's coin_index.
//!
//! For each spent coin in the incoming bundle, look up `coin_index`. If
//! found, the bundle IDs are added to the conflict set. If no conflicts,
//! the bundle is admitted directly.
//!
//! See: [`docs/requirements/domains/conflict_resolution/specs/CFR-001.md`]
//!
//! Note: The implementation currently lives inline in `Mempool::submit_inner`.
