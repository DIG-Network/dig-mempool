# Lifecycle — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [LCY-001](NORMATIVE.md#LCY-001) | ❌ | on_new_block() lifecycle | Integration test: submit items, confirm block, verify removals and retries. |
| [LCY-002](NORMATIVE.md#LCY-002) | ❌ | RetryBundles struct | Unit test: verify struct fields and types. |
| [LCY-003](NORMATIVE.md#LCY-003) | ❌ | Caller workflow sequencing | Integration test: full lifecycle with resubmission. |
| [LCY-004](NORMATIVE.md#LCY-004) | ❌ | clear() for reorg recovery | Unit test: populate mempool, clear, verify empty. |
| [LCY-005](NORMATIVE.md#LCY-005) | ❌ | MempoolEventHook trait | Unit test: register hook, trigger events, verify callbacks. |
| [LCY-006](NORMATIVE.md#LCY-006) | ❌ | RemovalReason enum | Unit test: all variants constructible, Clone + PartialEq. |
| [LCY-007](NORMATIVE.md#LCY-007) | ❌ | snapshot()/restore() persistence | Integration test: round-trip snapshot, verify state equality. |
| [LCY-008](NORMATIVE.md#LCY-008) | ❌ | evict_lowest_percent() | Unit test: eviction ordering, expiry protection, cascade. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
