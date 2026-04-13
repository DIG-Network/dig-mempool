# Conflict Resolution — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [CFR-001](NORMATIVE.md#CFR-001) | ⚠️ | Conflict detection via coin_index | coin_index HashMap lookup for each removal; active pool only. |
| [CFR-002](NORMATIVE.md#CFR-002) | ⚠️ | RBF superset rule | All conflicting removals must be subset of new removals. |
| [CFR-003](NORMATIVE.md#CFR-003) | ⚠️ | RBF fee-per-virtual-cost higher | Scaled integer FPC comparison; no float. |
| [CFR-004](NORMATIVE.md#CFR-004) | ⚠️ | RBF minimum fee bump | Absolute fee delta >= MIN_RBF_FEE_BUMP (10M mojos). |
| [CFR-005](NORMATIVE.md#CFR-005) | ⚠️ | Conflict cache on RBF failure | Bundle added to conflict cache; error still returned. |
| [CFR-006](NORMATIVE.md#CFR-006) | ⚠️ | RBF + CPFP cascade eviction | Recursive dependent removal; no conflict cache for children. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
