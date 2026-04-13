# Crate API — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [API-001](NORMATIVE.md#API-001) | ❌ | Mempool constructors | Unit test: construct with new() and with_config(), verify defaults. |
| [API-002](NORMATIVE.md#API-002) | ❌ | MempoolItem struct | Unit test: all fields accessible, stored as Arc. |
| [API-003](NORMATIVE.md#API-003) | ❌ | MempoolConfig builder | Unit test: builder pattern, default values correct. |
| [API-004](NORMATIVE.md#API-004) | ❌ | MempoolError enum | Unit test: all variants constructible, Clone + PartialEq works. |
| [API-005](NORMATIVE.md#API-005) | ❌ | SubmitResult enum | Unit test: Success and Pending variants constructible. |
| [API-006](NORMATIVE.md#API-006) | ❌ | MempoolStats struct | Unit test: all fields present, stats() returns correct values. |
| [API-007](NORMATIVE.md#API-007) | ❌ | Extension traits | Unit test: traits implementable, methods callable. |
| [API-008](NORMATIVE.md#API-008) | ❌ | Query methods | Unit test: all methods callable, return correct types. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
