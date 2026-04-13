# Lifecycle — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [LCY-001](NORMATIVE.md#LCY-001) | ✅ | on_new_block() lifecycle | 9 tests: confirmed removal, cascade on confirm, expired by height, not-yet-expired, pending promotions, conflict retries, multiple confirmed, unrelated coins ignored, empty pool. |
| [LCY-002](NORMATIVE.md#LCY-002) | ✅ | RetryBundles struct | 6 tests: struct public, conflict_retries field, pending_promotions field, cascade_evicted field, empty on noop block, confirmed item removed. |
| [LCY-003](NORMATIVE.md#LCY-003) | ✅ | Caller workflow sequencing | 6 tests: promoted-not-eligible-before-resubmit, resubmitted-eligible, failed-resubmit-harmless, select-before-resubmit-safe, conflict-retry-workflow, full-workflow. |
| [LCY-004](NORMATIVE.md#LCY-004) | ✅ | clear() for reorg recovery | 8 tests: empty after clear, active items removed, pending cleared, conflict cache cleared, seen cache cleared, hooks preserved, config preserved, removal hooks fired with Cleared reason. |
| [LCY-005](NORMATIVE.md#LCY-005) | ✅ | MempoolEventHook trait | 10 tests: default noop, on_item_added, on_item_removed confirmed, on_block_selected, on_conflict_cached, on_pending_added, multiple hooks, cascade evict fires hook, send+sync, add_event_hook public. |
| [LCY-006](NORMATIVE.md#LCY-006) | ✅ | RemovalReason enum | 10 tests: all variants, clone, partial_eq, debug, replacement_id field, parent_id field, confirmed reason, cascade_evicted parent_id correct, cleared reason, publicly exported. |
| [LCY-007](NORMATIVE.md#LCY-007) | ✅ | snapshot()/restore() persistence | 9 tests: is_public, round_trip_preserves_state, active_items_preserved, pending_items_preserved, conflict_cache_preserved, fee_estimator_preserved, seen_cache_excluded, json_serialization, indexes_rebuilt. |
| [LCY-008](NORMATIVE.md#LCY-008) | ✅ | evict_lowest_percent() | 8 tests: percent_0_noop, lowest_score_first, percent_100_evicts_all, expiry_protected_skipped, cascade_evicts_dependents, capacity_eviction_hook, cascade_hook_with_parent_id, is_public. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
