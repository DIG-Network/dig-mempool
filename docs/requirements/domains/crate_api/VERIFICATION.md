# Crate API — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [API-001](NORMATIVE.md#API-001) | ✅ | Mempool constructors | 7 tests: new/with_config compile, default config max_cost=8.25T, custom config applied, empty on construction, Send+Sync, independent instances. |
| [API-002](NORMATIVE.md#API-002) | ✅ | MempoolItem struct | 7 tests: all fields accessible, Arc wrapping, virtual_cost=cost+spends*penalty, FPC scaled integer, package=individual for root, SingletonLineageInfo, zero-fee edge case. |
| [API-003](NORMATIVE.md#API-003) | ✅ | MempoolConfig builder | 8 tests: all defaults match spec table, builder chaining, preserves unset, 17 with_* methods, Mempool integration, Clone, max_total_cost=8.25T, feature flags. |
| [API-004](NORMATIVE.md#API-004) | ✅ | MempoolError enum | 7 tests: all 24 variants constructible, Clone round-trip, PartialEq equality/inequality, Display formatting, From\<ValidationError\> conversion, Error trait, structured data types. |
| [API-005](NORMATIVE.md#API-005) | ✅ | SubmitResult enum | 6 tests: Success/Pending variants, pattern matching, inside Result, Clone, Debug. |
| [API-006](NORMATIVE.md#API-006) | ✅ | MempoolStats struct | 6 tests: all 14 fields accessible, empty stats correct, max_cost from config, Clone, Debug, shared-ref callable. |
| [API-007](NORMATIVE.md#API-007) | ✅ | Extension traits | 8 tests: AdmissionPolicy accept/reject/object-safe, BlockSelectionStrategy impl/object-safe, MempoolEventHook defaults, RemovalReason 7 variants + derives. |
| [API-008](NORMATIVE.md#API-008) | ✅ | Query methods | 13 tests: all 14 methods compile with correct signatures, return empty/default on empty pool, all &self (read-only), CPFP coin queries return Option. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
