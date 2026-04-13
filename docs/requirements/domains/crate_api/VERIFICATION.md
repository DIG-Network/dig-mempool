# Crate API — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [API-001](NORMATIVE.md#API-001) | ✅ | Mempool constructors | 7 tests: new/with_config compile, default config max_cost=8.25T, custom config applied, empty on construction, Send+Sync, independent instances. |
| [API-002](NORMATIVE.md#API-002) | ✅ | MempoolItem struct | 7 tests: all fields accessible, Arc wrapping, virtual_cost=cost+spends*penalty, FPC scaled integer, package=individual for root, SingletonLineageInfo, zero-fee edge case. |
| [API-003](NORMATIVE.md#API-003) | ✅ | MempoolConfig builder | 8 tests: all defaults match spec table, builder chaining, preserves unset, 17 with_* methods, Mempool integration, Clone, max_total_cost=8.25T, feature flags. |
| [API-004](NORMATIVE.md#API-004) | ❌ | MempoolError enum | Unit test: all variants constructible, Clone + PartialEq works. |
| [API-005](NORMATIVE.md#API-005) | ❌ | SubmitResult enum | Unit test: Success and Pending variants constructible. |
| [API-006](NORMATIVE.md#API-006) | ❌ | MempoolStats struct | Unit test: all fields present, stats() returns correct values. |
| [API-007](NORMATIVE.md#API-007) | ❌ | Extension traits | Unit test: traits implementable, methods callable. |
| [API-008](NORMATIVE.md#API-008) | ❌ | Query methods | Unit test: all methods callable, return correct types. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
