# Pool Management — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 8, 5.13

---

## &sect;1 Active Pool

<a id="POL-001"></a>**POL-001** The active pool MUST store items in a `HashMap<Bytes32, Arc<MempoolItem>>` keyed by bundle ID, with a secondary `coin_index: HashMap<Bytes32, Bytes32>` mapping each spent coin ID to its spending bundle ID.
> **Spec:** [`POL-001.md`](specs/POL-001.md)

<a id="POL-002"></a>**POL-002** When `total_cost + new_item.virtual_cost > max_total_cost`, the active pool MUST evict lowest `descendant_score` items until sufficient space is freed. If the new item's FPC is lower than or equal to the lowest `descendant_score`, the item MUST be rejected with `MempoolFull`.
> **Spec:** [`POL-002.md`](specs/POL-002.md)

<a id="POL-003"></a>**POL-003** Items within `expiry_protection_blocks` of their `assert_before_height` MUST be skipped during eviction. Expiring items may only be evicted by other expiring items with higher FPC.
> **Spec:** [`POL-003.md`](specs/POL-003.md)

---

## &sect;2 Pending Pool

<a id="POL-004"></a>**POL-004** The pending pool MUST be a separate `HashMap<Bytes32, Arc<MempoolItem>>` with `max_pending_count` (default 3000) and `max_pending_cost` limits. Items exceeding either limit MUST be rejected with `PendingPoolFull`.
> **Spec:** [`POL-004.md`](specs/POL-004.md)

<a id="POL-005"></a>**POL-005** The pending pool MUST maintain a `pending_coin_index: HashMap<Bytes32, Bytes32>` for pending-vs-pending conflict detection. Pending conflicts MUST be resolved via RBF rules.
> **Spec:** [`POL-005.md`](specs/POL-005.md)

---

## &sect;3 Conflict Cache

<a id="POL-006"></a>**POL-006** The conflict cache MUST be a `HashMap<Bytes32, SpendBundle>` with `max_conflict_count` (default 1000) and `max_conflict_cost` limits. Bundles that fail RBF are stored here for retry after the conflicting item is confirmed.
> **Spec:** [`POL-006.md`](specs/POL-006.md)

---

## &sect;4 Seen Cache

<a id="POL-007"></a>**POL-007** The seen cache MUST be an LRU-bounded set with `max_seen_cache_size` (default 10000) capacity. Bundle IDs MUST be added to the seen cache before CLVM validation begins (DoS protection).
> **Spec:** [`POL-007.md`](specs/POL-007.md)

---

## &sect;5 Auxiliary Indices

<a id="POL-008"></a>**POL-008** The identical spend dedup index MUST be a `HashMap<(Bytes32, Bytes32), Bytes32>` mapping `(coin_id, solution_hash)` to the first bundle ID that established the cost, for cost adjustment tracking.
> **Spec:** [`POL-008.md`](specs/POL-008.md)

<a id="POL-009"></a>**POL-009** The singleton tracking index MUST be a `HashMap<Bytes32, Vec<Bytes32>>` mapping `launcher_id` to an ordered list of bundle IDs in lineage order, for singleton chain ordering during block selection.
> **Spec:** [`POL-009.md`](specs/POL-009.md)

---

## &sect;6 Concurrency

<a id="POL-010"></a>**POL-010** The mempool MUST use `RwLock` for `pool_lock`, `pending_lock`, `conflict_lock`, and `seen_lock`, and `Mutex` for `bls_lock` (BLS cache). The `Mempool` struct MUST be `Send + Sync`.
> **Spec:** [`POL-010.md`](specs/POL-010.md)
