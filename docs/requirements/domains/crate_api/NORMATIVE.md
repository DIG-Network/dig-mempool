# Crate API — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 2, 3, 4

---

## &sect;1 Construction

<a id="API-001"></a>**API-001** `Mempool::new(constants)` MUST create a mempool with default `MempoolConfig` for the given network constants. `Mempool::with_config(constants, config)` MUST create a mempool with custom configuration.
> **Spec:** [`API-001.md`](specs/API-001.md)

---

## &sect;2 Core Types

<a id="API-002"></a>**API-002** `MempoolItem` MUST be a public struct stored as `Arc<MempoolItem>` containing all specified fields: identity (spend_bundle, spend_bundle_id), cost/fee (fee, cost, virtual_cost, fee_per_virtual_cost_scaled), package fields (package_fee, package_virtual_cost, package_fee_per_virtual_cost_scaled), descendant_score, state deltas (additions, removals), metadata (height_added, conditions, num_spends), timelocks (assert_height, assert_before_height, assert_before_seconds), dependencies (depends_on, depth), deduplication (eligible_for_dedup), and singleton (singleton_lineage).
> **Spec:** [`API-002.md`](specs/API-002.md)

<a id="API-003"></a>**API-003** `MempoolConfig` MUST be a public struct with a builder pattern (`with_*` methods) and documented default values for all fields.
> **Spec:** [`API-003.md`](specs/API-003.md)

---

## &sect;3 Error Types

<a id="API-004"></a>**API-004** `MempoolError` MUST be a public enum deriving `Clone + PartialEq` with all specified variants. `ValidationError` from dig-clvm MUST be stored as `String` to satisfy `Clone + PartialEq`.
> **Spec:** [`API-004.md`](specs/API-004.md)

---

## &sect;4 Result Types

<a id="API-005"></a>**API-005** `SubmitResult` MUST be a public enum with variants `Success` and `Pending { assert_height: u64 }`.
> **Spec:** [`API-005.md`](specs/API-005.md)

<a id="API-006"></a>**API-006** `MempoolStats` MUST be a public struct with all specified fields providing a snapshot of current mempool state.
> **Spec:** [`API-006.md`](specs/API-006.md)

---

## &sect;5 Extension Traits

<a id="API-007"></a>**API-007** `AdmissionPolicy` MUST be a public trait with a `check()` method. `BlockSelectionStrategy` MUST be a public trait with a `select()` method.
> **Spec:** [`API-007.md`](specs/API-007.md)

---

## &sect;6 Query Methods

<a id="API-008"></a>**API-008** The `Mempool` struct MUST expose all specified query methods: `get()`, `contains()`, `active_bundle_ids()`, `pending_bundle_ids()`, `active_items()`, `dependents_of()`, `ancestors_of()`, `len()`, `pending_len()`, `conflict_len()`, `is_empty()`, `stats()`, `get_mempool_coin_record()`, `get_mempool_coin_creator()`.
> **Spec:** [`API-008.md`](specs/API-008.md)
