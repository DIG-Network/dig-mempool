# Implementation Order

Phased checklist for dig-mempool requirements. Work top-to-bottom within each phase.
After completing a requirement: write tests, verify they pass, update TRACKING.yaml, VERIFICATION.md, and check off here.

**A requirement is NOT complete until comprehensive tests verify it.**

---

## Phase 0: Foundation (API Types & Config)

- [x] API-001 — Mempool constructor (`new`, `with_config`)
- [x] API-002 — MempoolItem struct with all fields (Arc-wrapped)
- [x] API-003 — MempoolConfig with builder pattern and defaults
- [x] API-004 — MempoolError enum (Clone + PartialEq, 24 variants)
- [x] API-005 — SubmitResult enum (Success, Pending)
- [x] API-006 — MempoolStats struct (13 fields)
- [x] API-007 — Extension traits (AdmissionPolicy, BlockSelectionStrategy, MempoolEventHook)
- [x] API-008 — Query methods (get, contains, active_items, stats, etc.)

## Phase 1: Admission Pipeline

- [x] ADM-001 — `submit()` entry point signature
- [x] ADM-002 — Internal CLVM validation via `dig_clvm::validate_spend_bundle()`
- [x] ADM-003 — Dedup check via `SpendBundle::name()` + seen-cache (DoS protection)
- [x] ADM-004 — Fee extraction from `SpendResult.fee` + RESERVE_FEE check
- [x] ADM-005 — Virtual cost: `cost + (num_spends * SPEND_PENALTY_COST)`
- [x] ADM-006 — Timelock resolution (relative to absolute, impossible constraints, expiry, pending)
- [x] ADM-007 — Dedup/FF flag extraction from `OwnedSpendConditions.flags`
- [x] ADM-008 — `submit_batch()` concurrent Phase 1, sequential Phase 2

## Phase 2: Pool Management

- [x] POL-001 — Active pool storage (HashMap + coin_index)
- [x] POL-002 — Active pool capacity management (evict by descendant_score)
- [x] POL-003 — Expiry protection (skip protected items during eviction)
- [x] POL-004 — Pending pool (separate HashMap, count + cost limits)
- [x] POL-005 — Pending pool deduplication (pending_coin_index, pending-vs-pending RBF)
- [x] POL-006 — Conflict cache (HashMap, count + cost limits)
- [x] POL-007 — Seen cache (LRU bounded set, pre-validation insertion)
- [x] POL-008 — Identical spend dedup index (cost adjustment tracking)
- [ ] POL-009 — Singleton tracking (launcher_id -> bundle chain ordering)
- [x] POL-010 — Concurrency (RwLock per pool, Mutex for BLS, Send + Sync)

## Phase 3: Conflict Resolution / RBF

- [x] CFR-001 — Conflict detection via coin_index (active pool only)
- [x] CFR-002 — RBF superset rule
- [x] CFR-003 — RBF higher fee-per-virtual-cost requirement
- [x] CFR-004 — RBF minimum fee bump (10M mojos)
- [x] CFR-005 — Conflict cache on RBF failure
- [x] CFR-006 — RBF + CPFP cascade eviction interaction

## Phase 4: CPFP Dependencies

- [x] CPF-001 — mempool_coins index (coin_id -> creating bundle_id)
- [x] CPF-002 — Dependency resolution (mempool_coins lookup, dependency graph edges)
- [x] CPF-003 — Maximum dependency depth enforcement (default 25)
- [x] CPF-004 — Defensive cycle detection
- [x] CPF-005 — Package fee rate computation (ancestor aggregation)
- [x] CPF-006 — Descendant score tracking (eviction protection for parents)
- [x] CPF-007 — Cascade eviction (recursive removal of dependents)
- [x] CPF-008 — Cross-bundle announcement validation (CPFP chains)

## Phase 5: Block Candidate Selection

- [x] SEL-001 — `select_for_block()` entry point (active pool only)
- [x] SEL-002 — Pre-selection filtering (expired, future-timelocked)
- [x] SEL-003 — Strategy 1: fee-per-cost density sort
- [x] SEL-004 — Strategy 2: absolute fee whale sort
- [x] SEL-005 — Strategy 3: compact high-value sort
- [x] SEL-006 — Strategy 4: age-weighted anti-starvation sort
- [x] SEL-007 — Best-selection comparator (highest fees, lowest cost, fewest bundles)
- [x] SEL-008 — Final topological ordering (parents before children, FPC descending)

## Phase 6: Fee Estimation

- [ ] FEE-001 — `estimate_min_fee()` 3-tier utilization system
- [ ] FEE-002 — FeeTracker bucket-based tracker (rolling window, log-spaced buckets)
- [ ] FEE-003 — `estimate_fee_rate(target_blocks)` with 85% confidence
- [ ] FEE-004 — `record_confirmed_block()` with exponential decay (0.998)
- [ ] FEE-005 — FeeEstimatorState serialization for persistence

## Phase 7: Lifecycle / Events / Persistence

- [x] LCY-001 — `on_new_block()` (remove confirmed, cascade evict, remove expired, collect retries)
- [x] LCY-002 — RetryBundles struct (conflict_retries, pending_promotions, cascade_evicted)
- [x] LCY-003 — Caller workflow sequencing (on_new_block -> submit retries -> select_for_block)
- [x] LCY-004 — `clear()` for reorg recovery
- [x] LCY-005 — MempoolEventHook trait (5 callbacks, called under write lock)
- [x] LCY-006 — RemovalReason enum (7 variants)
- [ ] LCY-007 — `snapshot()` / `restore()` persistence (MempoolSnapshot with serde)
- [ ] LCY-008 — `evict_lowest_percent()` memory pressure eviction

---

## Summary

| Phase | Domain | Count |
|-------|--------|-------|
| 0 | Crate API | 8 |
| 1 | Admission | 8 |
| 2 | Pools | 10 |
| 3 | Conflict Resolution | 6 |
| 4 | CPFP | 8 |
| 5 | Selection | 8 |
| 6 | Fee Estimation | 5 |
| 7 | Lifecycle | 8 |
| **Total** | | **61** |
