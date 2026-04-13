# Lifecycle — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 3.5, 3.8, 3.9, 3.10, 9

---

## &sect;1 Block Confirmation

<a id="LCY-001"></a>**LCY-001** `on_new_block(height, timestamp, spent_coin_ids, confirmed_bundles)` MUST remove confirmed items (those whose spent coins overlap with `spent_coin_ids`), cascade-evict their dependents, remove expired items plus dependents, collect pending promotions and conflict retries, update the fee estimator, and return `RetryBundles`.
> **Spec:** [`LCY-001.md`](specs/LCY-001.md)

<a id="LCY-002"></a>**LCY-002** `RetryBundles` MUST contain `conflict_retries: Vec<SpendBundle>`, `pending_promotions: Vec<SpendBundle>`, and `cascade_evicted: Vec<Bytes32>`.
> **Spec:** [`LCY-002.md`](specs/LCY-002.md)

<a id="LCY-003"></a>**LCY-003** The caller workflow MUST follow the sequence: `on_new_block()` -> `submit()` retries -> `select_for_block()`. Promoted items MUST only be eligible after resubmission with fresh coin records.
> **Spec:** [`LCY-003.md`](specs/LCY-003.md)

---

## &sect;2 Reset

<a id="LCY-004"></a>**LCY-004** `clear()` MUST reset all mempool state for reorg recovery, including active pool, pending pool, conflict cache, seen-cache, and dependency graph. Event hooks and configuration MUST be preserved.
> **Spec:** [`LCY-004.md`](specs/LCY-004.md)

---

## &sect;3 Event Hooks

<a id="LCY-005"></a>**LCY-005** The `MempoolEventHook` trait MUST define callback methods: `on_item_added`, `on_item_removed` (with `RemovalReason`), `on_block_selected`, `on_conflict_cached`, and `on_pending_added`. Hooks MUST be called synchronously under the write lock.
> **Spec:** [`LCY-005.md`](specs/LCY-005.md)

<a id="LCY-006"></a>**LCY-006** `RemovalReason` MUST be an enum with variants: `Confirmed`, `ReplacedByFee`, `CascadeEvicted`, `Expired`, `CapacityEviction`, `ExplicitRemoval`, and `Cleared`. It MUST derive `Debug + Clone + PartialEq`.
> **Spec:** [`LCY-006.md`](specs/LCY-006.md)

---

## &sect;4 Persistence

<a id="LCY-007"></a>**LCY-007** `snapshot()` MUST return a `MempoolSnapshot` with `serde::Serialize + Deserialize`. `restore()` MUST accept a `MempoolSnapshot` and replace all current state. The snapshot MUST include active, pending, conflict, and fee estimator state. The snapshot MUST NOT include the seen-cache or BLS cache.
> **Spec:** [`LCY-007.md`](specs/LCY-007.md)

---

## &sect;5 Memory Pressure

<a id="LCY-008"></a>**LCY-008** `evict_lowest_percent(percent: u8)` MUST evict the lowest fee-rate items from the active pool by `descendant_score`, respecting expiry protection and cascade-evicting dependents of removed items.
> **Spec:** [`LCY-008.md`](specs/LCY-008.md)
