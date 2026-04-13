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
- [ ] ADM-002 — Internal CLVM validation via `dig_clvm::validate_spend_bundle()`
- [ ] ADM-003 — Dedup check via `SpendBundle::name()` + seen-cache (DoS protection)
- [ ] ADM-004 — Fee extraction from `SpendResult.fee` + RESERVE_FEE check
- [ ] ADM-005 — Virtual cost: `cost + (num_spends * SPEND_PENALTY_COST)`
- [ ] ADM-006 — Timelock resolution (relative to absolute, impossible constraints, expiry, pending)
- [ ] ADM-007 — Dedup/FF flag extraction from `OwnedSpendConditions.flags`
- [ ] ADM-008 — `submit_batch()` concurrent Phase 1, sequential Phase 2

## Phase 2: Pool Management

- [ ] POL-001 — Active pool storage (HashMap + coin_index)
- [ ] POL-002 — Active pool capacity management (evict by descendant_score)
- [ ] POL-003 — Expiry protection (skip protected items during eviction)
- [ ] POL-004 — Pending pool (separate HashMap, count + cost limits)
- [ ] POL-005 — Pending pool deduplication (pending_coin_index, pending-vs-pending RBF)
- [ ] POL-006 — Conflict cache (HashMap, count + cost limits)
- [ ] POL-007 — Seen cache (LRU bounded set, pre-validation insertion)
- [ ] POL-008 — Identical spend dedup index (cost adjustment tracking)
- [ ] POL-009 — Singleton tracking (launcher_id -> bundle chain ordering)
- [ ] POL-010 — Concurrency (RwLock per pool, Mutex for BLS, Send + Sync)

## Phase 3: Conflict Resolution / RBF

- [ ] CFR-001 — Conflict detection via coin_index (active pool only)
- [ ] CFR-002 — RBF superset rule
- [ ] CFR-003 — RBF higher fee-per-virtual-cost requirement
- [ ] CFR-004 — RBF minimum fee bump (10M mojos)
- [ ] CFR-005 — Conflict cache on RBF failure
- [ ] CFR-006 — RBF + CPFP cascade eviction interaction

## Phase 4: CPFP Dependencies

- [ ] CPF-001 — mempool_coins index (coin_id -> creating bundle_id)
- [ ] CPF-002 — Dependency resolution (mempool_coins lookup, dependency graph edges)
- [ ] CPF-003 — Maximum dependency depth enforcement (default 25)
- [ ] CPF-004 — Defensive cycle detection
- [ ] CPF-005 — Package fee rate computation (ancestor aggregation)
- [ ] CPF-006 — Descendant score tracking (eviction protection for parents)
- [ ] CPF-007 — Cascade eviction (recursive removal of dependents)
- [ ] CPF-008 — Cross-bundle announcement validation (CPFP chains)

## Phase 5: Block Candidate Selection

- [ ] SEL-001 — `select_for_block()` entry point (active pool only)
- [ ] SEL-002 — Pre-selection filtering (expired, future-timelocked)
- [ ] SEL-003 — Strategy 1: fee-per-cost density sort
- [ ] SEL-004 — Strategy 2: absolute fee whale sort
- [ ] SEL-005 — Strategy 3: compact high-value sort
- [ ] SEL-006 — Strategy 4: age-weighted anti-starvation sort
- [ ] SEL-007 — Best-selection comparator (highest fees, lowest cost, fewest bundles)
- [ ] SEL-008 — Final topological ordering (parents before children, FPC descending)

## Phase 6: Fee Estimation

- [ ] FEE-001 — `estimate_min_fee()` 3-tier utilization system
- [ ] FEE-002 — FeeTracker bucket-based tracker (rolling window, log-spaced buckets)
- [ ] FEE-003 — `estimate_fee_rate(target_blocks)` with 85% confidence
- [ ] FEE-004 — `record_confirmed_block()` with exponential decay (0.998)
- [ ] FEE-005 — FeeEstimatorState serialization for persistence

## Phase 7: Lifecycle / Events / Persistence

- [ ] LCY-001 — `on_new_block()` (remove confirmed, cascade evict, remove expired, collect retries)
- [ ] LCY-002 — RetryBundles struct (conflict_retries, pending_promotions, cascade_evicted)
- [ ] LCY-003 — Caller workflow sequencing (on_new_block -> submit retries -> select_for_block)
- [ ] LCY-004 — `clear()` for reorg recovery
- [ ] LCY-005 — MempoolEventHook trait (5 callbacks, called under write lock)
- [ ] LCY-006 — RemovalReason enum (7 variants)
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
