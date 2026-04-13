# Conflict Resolution — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 5.10, 5.11, 9.2

---

## &sect;1 Conflict Detection

<a id="CFR-001"></a>**CFR-001** For each removal in a submitted bundle, the mempool MUST check the active pool's `coin_index` (`HashMap<Bytes32, Bytes32>`, mapping coin_id to spending bundle_id). Any match indicates a conflict with an existing active item. Pending and conflict-cache items MUST NOT participate in conflict detection.
> **Spec:** [`CFR-001.md`](specs/CFR-001.md)

---

## &sect;2 RBF Superset Rule

<a id="CFR-002"></a>**CFR-002** When a new bundle conflicts with one or more active items, RBF MUST require that the new bundle spends a superset of all conflicting bundles' removals. If any conflicting bundle contains a removal not present in the new bundle, the replacement MUST be rejected with `MempoolError::RbfNotSuperset`.
> **Spec:** [`CFR-002.md`](specs/CFR-002.md)

---

## &sect;3 RBF Fee-Per-Virtual-Cost

<a id="CFR-003"></a>**CFR-003** The new bundle's fee-per-virtual-cost MUST be strictly higher than the aggregate fee-per-cost of all conflicting bundles (`conflicting_fees / conflicting_cost`). If not, the replacement MUST be rejected with `MempoolError::RbfFpcNotHigher`.
> **Spec:** [`CFR-003.md`](specs/CFR-003.md)

---

## &sect;4 RBF Minimum Fee Bump

<a id="CFR-004"></a>**CFR-004** The new bundle's fee MUST be at least `conflicting_fees + MIN_RBF_FEE_BUMP` (default 10,000,000 mojos). If not, the replacement MUST be rejected with `MempoolError::RbfBumpTooLow`.
> **Spec:** [`CFR-004.md`](specs/CFR-004.md)

---

## &sect;5 Conflict Cache on RBF Failure

<a id="CFR-005"></a>**CFR-005** When a bundle fails RBF (any of CFR-002, CFR-003, or CFR-004 rejects), the bundle MUST be added to the conflict cache if space permits (count < `max_conflict_count` and cost < `max_conflict_cost`). The appropriate `MempoolError` variant MUST still be returned.
> **Spec:** [`CFR-005.md`](specs/CFR-005.md)

---

## &sect;6 RBF + CPFP Cascade Eviction

<a id="CFR-006"></a>**CFR-006** When RBF successfully replaces a parent item that has CPFP dependents, all dependents MUST be recursively cascade-evicted. Cascade-evicted children MUST NOT be added to the conflict cache. They MUST be reported via `RemovalReason::CascadeEvicted` event hooks and included in `RetryBundles::cascade_evicted`.
> **Spec:** [`CFR-006.md`](specs/CFR-006.md)
