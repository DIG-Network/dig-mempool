# Child-Pays-For-Parent (CPFP) — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 5.8, 5.9, 5.17, 9.2

---

## &sect;1 Mempool Coins Index

<a id="CPF-001"></a>**CPF-001** When a `MempoolItem` is added to the active pool, the mempool MUST register every coin in the item's `additions` into the `mempool_coins` index (`HashMap<Bytes32, Bytes32>`, mapping coin_id to creating bundle_id). Entries MUST be removed when the creating item is removed or evicted.
> **Spec:** [`CPF-001.md`](specs/CPF-001.md)

---

## &sect;2 Dependency Resolution

<a id="CPF-002"></a>**CPF-002** During Phase 2, for each removal that is NOT in the caller's `coin_records`, the mempool MUST look up `mempool_coins` to find the creating bundle. If found, a dependency edge MUST be recorded in the dependency graph (`dependencies` and `dependents` maps). If not found, the mempool MUST reject the bundle with `MempoolError::CoinNotFound`.
> **Spec:** [`CPF-002.md`](specs/CPF-002.md)

---

## &sect;3 Maximum Dependency Depth

<a id="CPF-003"></a>**CPF-003** After resolving dependencies, the mempool MUST compute the item's depth in the dependency chain. If the depth exceeds `config.max_dependency_depth` (default 25), the bundle MUST be rejected with `MempoolError::DependencyTooDeep`.
> **Spec:** [`CPF-003.md`](specs/CPF-003.md)

---

## &sect;4 Defensive Cycle Detection

<a id="CPF-004"></a>**CPF-004** The mempool MUST perform a defensive cycle check on the dependency graph after inserting a new edge. If a cycle is detected, the bundle MUST be rejected with `MempoolError::DependencyCycle`. This should be unreachable in the UTXO model but guards against implementation bugs.
> **Spec:** [`CPF-004.md`](specs/CPF-004.md)

---

## &sect;5 Package Fee Rate Computation

<a id="CPF-005"></a>**CPF-005** For items with dependencies, the mempool MUST compute package fee rates: `package_fee = fee + sum(ancestor.fee)`, `package_virtual_cost = virtual_cost + sum(ancestor.virtual_cost)`, `package_fee_per_virtual_cost_scaled = (package_fee * FPC_SCALE) / package_virtual_cost`. Items with no dependencies MUST have `package_fee == fee` and `package_virtual_cost == virtual_cost`.
> **Spec:** [`CPF-005.md`](specs/CPF-005.md)

---

## &sect;6 Descendant Score Tracking

<a id="CPF-006"></a>**CPF-006** Each `MempoolItem` MUST maintain a `descendant_score` equal to the maximum of its own `fee_per_virtual_cost_scaled` and the `package_fee_per_virtual_cost_scaled` of any descendant chain. The score MUST be updated when children are added or removed. It is used for eviction ordering to protect low-fee parents with valuable children.
> **Spec:** [`CPF-006.md`](specs/CPF-006.md)

---

## &sect;7 Cascade Eviction

<a id="CPF-007"></a>**CPF-007** Removing a parent item MUST recursively remove all its dependents (direct and transitive). The dependency graph, `mempool_coins`, and `coin_index` MUST all be cleaned. Removed items MUST be reported via `RemovalReason::CascadeEvicted` event hooks.
> **Spec:** [`CPF-007.md`](specs/CPF-007.md)

---

## &sect;8 Cross-Bundle Announcement Validation

<a id="CPF-008"></a>**CPF-008** For CPFP items (non-empty `depends_on`), the mempool MUST validate cross-bundle announcements: `ASSERT_COIN_ANNOUNCEMENT` and `ASSERT_PUZZLE_ANNOUNCEMENT` conditions MUST be checked against ancestor bundles' `CREATE_COIN_ANNOUNCEMENT` and `CREATE_PUZZLE_ANNOUNCEMENT` conditions. `RECEIVE_MESSAGE` conditions MUST be checked against ancestors' `SEND_MESSAGE` conditions. Announcement IDs MUST be computed using `announcement_id()` from `chia-sdk-types`.
> **Spec:** [`CPF-008.md`](specs/CPF-008.md)
