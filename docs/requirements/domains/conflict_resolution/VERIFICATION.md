# Conflict Resolution — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [CFR-001](NORMATIVE.md#CFR-001) | ✅ | Conflict detection via coin_index | 6 tests: empty pool no conflict, single conflict, multiple conflicts, pending not indexed, conflict cache not indexed, coin_index cleaned on removal. Dedup-waiter bypass: eligible_for_dedup && dedup_index.contains_key((coin_id, sol_hash)) → skip conflict (matches Chia mempool_manager.py:270-288). |
| [CFR-002](NORMATIVE.md#CFR-002) | ✅ | RBF superset rule | 3 tests: superset passes, missing removal fails, multiple conflicts all covered. Iterates all conflicting items' removals; first missing coin triggers RbfNotSuperset + conflict cache. |
| [CFR-003](NORMATIVE.md#CFR-003) | ✅ | RBF fee-per-virtual-cost higher | 4 tests: higher FPC passes, equal FPC rejected, lower FPC rejected, aggregate FPC compared. Scaled integer arithmetic (FPC_SCALE=1e12); strict greater-than comparison against aggregate. |
| [CFR-004](NORMATIVE.md#CFR-004) | ✅ | RBF minimum fee bump | 3 tests: exact minimum passes, below minimum fails (RbfBumpTooLow with required/provided), aggregate fee compared. saturating_add prevents overflow; configurable min_rbf_fee_bump. |
| [CFR-005](NORMATIVE.md#CFR-005) | ✅ | Conflict cache on RBF failure | 3 tests: caches on superset failure, FPC failure, bump failure. add_to_conflict_cache() called before returning error; lock ordering pool→conflict (POL-010). |
| [CFR-006](NORMATIVE.md#CFR-006) | ✅ | Conflicting items removed on successful RBF | 3 tests: single conflict removed, two conflicts both removed, coin_index updated after RBF. pool.remove() cleans coin_index + accumulators. CPFP cascade deferred to CPF-007. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
