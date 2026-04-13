# Admission Pipeline — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [ADM-001](NORMATIVE.md#ADM-001) | ✅ | submit() entry point signature | 6 tests: signature compiles, &self not &mut, bundle consumed by value, submit_with_policy exists, return type matchable, concurrent access. |
| [ADM-002](NORMATIVE.md#ADM-002) | ✅ | Internal CLVM validation via dig-clvm | 5 tests: empty bundle passes, missing coin rejected, BLS cache reuse, concurrent validation, error type conversion. MEMPOOL_MODE + BlsCache wired. |
| [ADM-003](NORMATIVE.md#ADM-003) | ✅ | Dedup check via seen-cache | 4 tests: duplicate rejected, invalid bundle cached, LRU eviction allows retry, bundle ID via name(). SeenCache FIFO with configurable max_size. |
| [ADM-004](NORMATIVE.md#ADM-004) | ✅ | Fee extraction and RESERVE_FEE check | fee from SpendResult.fee. InsufficientFee if fee < reserve_fee. |
| [ADM-005](NORMATIVE.md#ADM-005) | ❌ | Virtual cost computation | virtual_cost = cost + num_spends * SPEND_PENALTY_COST. CostExceeded if over limit. |
| [ADM-006](NORMATIVE.md#ADM-006) | ❌ | Timelock resolution | Relative to absolute. ImpossibleTimelocks, Expired, and Pending routing. |
| [ADM-007](NORMATIVE.md#ADM-007) | ❌ | Dedup/FF flag extraction | Reads ELIGIBLE_FOR_DEDUP and ELIGIBLE_FOR_FF from conditions.flags. |
| [ADM-008](NORMATIVE.md#ADM-008) | ❌ | Batch submission | Concurrent Phase 1, sequential Phase 2. Results ordered. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
