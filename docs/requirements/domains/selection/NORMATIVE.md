# Block Candidate Selection — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 6.1-6.9

---

## &sect;1 Entry Point

<a id="SEL-001"></a>**SEL-001** `select_for_block()` MUST accept `max_block_cost: u64`, `height: u64`, and `timestamp: u64`, and MUST return `Vec<Arc<MempoolItem>>` containing only items from the active pool.
> **Spec:** [`SEL-001.md`](specs/SEL-001.md)

---

## &sect;2 Pre-Selection Filtering

<a id="SEL-002"></a>**SEL-002** `select_for_block()` MUST exclude items with `assert_before_height <= height`, items with `assert_before_seconds <= timestamp`, and items with `assert_height > height` (future-timelocked) from the candidate set before running any selection strategy.
> **Spec:** [`SEL-002.md`](specs/SEL-002.md)

---

## &sect;3 Selection Strategies

<a id="SEL-003"></a>**SEL-003** Strategy 1 (fee-per-cost density) MUST sort candidates by `package_fee_per_virtual_cost_scaled` descending, then `fee` descending, then `virtual_cost` ascending, then `height_added` ascending, then `spend_bundle_id` ascending, and MUST use CPFP-aware greedy accumulation.
> **Spec:** [`SEL-003.md`](specs/SEL-003.md)

<a id="SEL-004"></a>**SEL-004** Strategy 2 (absolute fee whale) MUST sort candidates by `package_fee` descending, then `package_fee_per_virtual_cost_scaled` descending, then `virtual_cost` ascending, then `height_added` ascending, then `spend_bundle_id` ascending, and MUST use CPFP-aware greedy accumulation.
> **Spec:** [`SEL-004.md`](specs/SEL-004.md)

<a id="SEL-005"></a>**SEL-005** Strategy 3 (compact high-value) MUST sort candidates by `package_fee_per_virtual_cost_scaled` descending, then `virtual_cost` ascending, then `fee` descending, then `height_added` ascending, then `spend_bundle_id` ascending, and MUST use CPFP-aware greedy accumulation.
> **Spec:** [`SEL-005.md`](specs/SEL-005.md)

<a id="SEL-006"></a>**SEL-006** Strategy 4 (age-weighted) MUST sort candidates by `height_added` ascending (oldest first), then `package_fee_per_virtual_cost_scaled` descending, then `fee` descending, then `spend_bundle_id` ascending, and MUST use CPFP-aware greedy accumulation.
> **Spec:** [`SEL-006.md`](specs/SEL-006.md)

---

## &sect;4 Best Selection

<a id="SEL-007"></a>**SEL-007** The best-selection comparator MUST compare the four strategy candidate sets by: (1) highest total fees, then (2) lowest total virtual cost, then (3) fewest bundles, and MUST return the winning set.
> **Spec:** [`SEL-007.md`](specs/SEL-007.md)

---

## &sect;5 Final Ordering

<a id="SEL-008"></a>**SEL-008** The final output MUST be in topological order (parents before children), with items at the same topological depth sorted by `fee_per_virtual_cost_scaled` descending, then `height_added` ascending, then `spend_bundle_id` ascending. All sort orders MUST be deterministic via `height_added` and `spend_bundle_id` tiebreakers.
> **Spec:** [`SEL-008.md`](specs/SEL-008.md)
