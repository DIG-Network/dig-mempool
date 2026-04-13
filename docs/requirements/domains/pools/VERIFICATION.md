# Pool Management — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [POL-001](NORMATIVE.md#POL-001) | ✅ | Active pool storage | 15 tests: get/contains after submit, len increments, active_items/bundle_ids, stats.active_count, Arc sharing, removals populated, height_added, mempool_coin_creator. ActivePool struct with items/coin_index/mempool_coins HashMaps + accumulators. |
| [POL-002](NORMATIVE.md#POL-002) | ✅ | Active pool capacity management | 6 tests: no eviction when space available, lowest score evicted, MempoolFull when FPC too low, MempoolFull on empty pool too small, TooManySpends, minimal eviction. ActivePool::evict_for() + is_expiry_protected() scaffold. |
| [POL-003](NORMATIVE.md#POL-003) | ✅ | Expiry protection | 7 tests: protected item skipped by non-expiring new item, far-future expiry evictable, no-expiry items always evictable, expiring evicts lower-FPC expiring, MempoolFull when all protected, configurable window, boundary inclusive. Two-pass evict_for() with is_expiry_protected() gate. |
| [POL-004](NORMATIVE.md#POL-004) | ✅ | Pending pool | 11 tests: timelocked routing, separate from active, pending_len/stats/bundle_ids, count limit, cost limit, drain_pending promotions, not-yet-mature, removal cleanup, cost tracking. PendingPool struct + drain_pending() + assert_seconds in MempoolItem + pending_cost in MempoolStats. |
| [POL-005](NORMATIVE.md#POL-005) | ✅ | Pending pool deduplication | 8 tests: index populated, conflict detected, RBF succeeds, not superset, FPC too low, fee bump too low, index cleaned on promotion, no conflict cache. pending_coin_index maintained on insert/remove; conflict detection + RBF (superset/FPC/bump) in submit() pending path; get_pending_coin_spender() method. |
| [POL-006](NORMATIVE.md#POL-006) | ⚠️ | Conflict cache | HashMap<Bytes32, SpendBundle>, count + cost limits, retry on confirmation. |
| [POL-007](NORMATIVE.md#POL-007) | ⚠️ | Seen cache | LRU set, pre-validation insertion, max_seen_cache_size=10000. |
| [POL-008](NORMATIVE.md#POL-008) | ⚠️ | Identical spend dedup index | (coin_id, solution_hash) -> bundle_id for cost adjustment. |
| [POL-009](NORMATIVE.md#POL-009) | ⚠️ | Singleton tracking | launcher_id -> Vec<bundle_id> in lineage order. |
| [POL-010](NORMATIVE.md#POL-010) | ⚠️ | Concurrency | RwLock per pool, Mutex for BLS cache, Send + Sync. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
