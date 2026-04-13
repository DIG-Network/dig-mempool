# Admission Pipeline — Normative Requirements

> **Master spec:** [SPEC.md](../../../resources/SPEC.md) — Section 5

---

## &sect;1 Entry Point

<a id="ADM-001"></a>**ADM-001** `submit()` MUST accept a `SpendBundle`, `&HashMap<Bytes32, CoinRecord>`, `current_height: u64`, and `current_timestamp: u64`, and MUST return `Result<SubmitResult, MempoolError>`.
> **Spec:** [`ADM-001.md`](specs/ADM-001.md)

<a id="ADM-002"></a>**ADM-002** `submit()` MUST call `dig_clvm::validate_spend_bundle()` internally for CLVM dry-run and BLS aggregate signature verification. Invalid or incorrectly-signed bundles MUST be rejected before entering the pool.
> **Spec:** [`ADM-002.md`](specs/ADM-002.md)

---

## &sect;2 Deduplication

<a id="ADM-003"></a>**ADM-003** `submit()` MUST compute the bundle ID via `SpendBundle::name()` and check it against the seen-cache, active items, pending items, and conflict cache **before** CLVM validation. Duplicate bundles MUST return `MempoolError::AlreadySeen`. The bundle ID MUST be added to the seen-cache immediately for DoS protection.
> **Spec:** [`ADM-003.md`](specs/ADM-003.md)

---

## &sect;3 Fee and Cost Extraction

<a id="ADM-004"></a>**ADM-004** After successful CLVM validation, the fee MUST be extracted from `SpendResult.fee` (= `conditions.removal_amount - conditions.addition_amount`). If `fee < conditions.reserve_fee`, `submit()` MUST return `MempoolError::InsufficientFee`.
> **Spec:** [`ADM-004.md`](specs/ADM-004.md)

<a id="ADM-005"></a>**ADM-005** Virtual cost MUST be computed as `cost + (num_spends * SPEND_PENALTY_COST)`. Fee-per-virtual-cost MUST be computed as `(fee * FPC_SCALE) / virtual_cost` using integer arithmetic. If `cost > config.max_bundle_cost`, `submit()` MUST return `MempoolError::CostExceeded`.
> **Spec:** [`ADM-005.md`](specs/ADM-005.md)

---

## &sect;4 Timelock Resolution

<a id="ADM-006"></a>**ADM-006** `submit()` MUST resolve relative timelocks to absolute values using coin_records. Per-spend `height_relative` MUST be resolved to `coin_record.confirmed_block_index + n`. Per-spend `seconds_relative` MUST be resolved to `coin_record.timestamp + n`. Impossible constraints (`assert_before_height <= assert_height`) MUST return `MempoolError::ImpossibleTimelocks`. Already-expired bundles MUST return `MempoolError::Expired`. Future-timelocked bundles MUST be routed to the pending pool.
> **Spec:** [`ADM-006.md`](specs/ADM-006.md)

---

## &sect;5 Flag Extraction

<a id="ADM-007"></a>**ADM-007** `submit()` MUST read the `ELIGIBLE_FOR_DEDUP` (0x1) and `ELIGIBLE_FOR_FF` (0x4) flags from `OwnedSpendConditions.flags` in the validated `SpendResult.conditions`. The mempool MUST NOT compute these flags itself; they are set by chia-consensus's `MempoolVisitor` during CLVM execution. `eligible_for_dedup` MUST be `true` only if ALL spends in the bundle have the `ELIGIBLE_FOR_DEDUP` flag set.
> **Spec:** [`ADM-007.md`](specs/ADM-007.md)

---

## &sect;6 Batch Submission

<a id="ADM-008"></a>**ADM-008** `submit_batch()` MUST validate all bundles concurrently in Phase 1 (CLVM validation, lock-free) and then insert sequentially in Phase 2 (single write lock acquisition). Earlier entries in the batch MUST be able to create coins that later entries depend on (CPFP). Results MUST be returned in the same order as inputs.
> **Spec:** [`ADM-008.md`](specs/ADM-008.md)
