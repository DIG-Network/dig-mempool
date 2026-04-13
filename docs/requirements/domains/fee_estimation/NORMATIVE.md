# Fee Estimation — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Sections 3.6, 10

---

## &sect;1 Minimum Fee Estimation

<a id="FEE-001"></a>**FEE-001** `estimate_min_fee(cost, num_spends)` MUST implement a 3-tier fee system based on mempool utilization: returns 0 when utilization is below 80%, returns `virtual_cost * full_mempool_min_fpc_scaled / FPC_SCALE` when utilization is 80%-100%, and returns `virtual_cost * (lowest_fpc + 1) / FPC_SCALE` when utilization is at or above 100%.
> **Spec:** [`FEE-001.md`](specs/FEE-001.md)

---

## &sect;2 Fee Tracker

<a id="FEE-002"></a>**FEE-002** `FeeTracker` MUST implement a bucket-based fee rate tracker using a rolling window of recent blocks and logarithmically spaced fee-rate buckets, tracking confirmation rates at 1, 2, 5, and 10 block targets per bucket.
> **Spec:** [`FEE-002.md`](specs/FEE-002.md)

---

## &sect;3 Fee Rate Estimation

<a id="FEE-003"></a>**FEE-003** `estimate_fee_rate(target_blocks)` MUST return `Option<FeeRate>` (chia-protocol type) using an 85% confidence threshold across fee-rate buckets. MUST return `None` when insufficient data has been collected (fewer than `fee_estimator_window / 2` blocks tracked).
> **Spec:** [`FEE-003.md`](specs/FEE-003.md)

---

## &sect;4 Confirmed Block Recording

<a id="FEE-004"></a>**FEE-004** `record_confirmed_block(height, bundles)` MUST feed `ConfirmedBundleInfo` data to the `FeeTracker`, and MUST be called by `on_new_block()`. Older observations MUST be decayed with an exponential decay factor of 0.998 per block.
> **Spec:** [`FEE-004.md`](specs/FEE-004.md)

---

## &sect;5 Persistence

<a id="FEE-005"></a>**FEE-005** `FeeEstimatorState` MUST implement `serde::Serialize` and `serde::Deserialize` for snapshot/restore persistence. The serialized state MUST include bucket data, block history, and current height.
> **Spec:** [`FEE-005.md`](specs/FEE-005.md)
