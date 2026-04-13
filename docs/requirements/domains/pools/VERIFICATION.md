# Pool Management — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [POL-001](NORMATIVE.md#POL-001) | ⚠️ | Active pool storage | HashMap<Bytes32, Arc<MempoolItem>> + coin_index HashMap. |
| [POL-002](NORMATIVE.md#POL-002) | ⚠️ | Active pool capacity management | Evict lowest descendant_score. Reject if new FPC <= lowest score. |
| [POL-003](NORMATIVE.md#POL-003) | ⚠️ | Expiry protection | Skip expiry-protected items during eviction. Expiring vs expiring FPC comparison. |
| [POL-004](NORMATIVE.md#POL-004) | ⚠️ | Pending pool | Separate HashMap, count + cost limits, PendingPoolFull error. |
| [POL-005](NORMATIVE.md#POL-005) | ⚠️ | Pending pool deduplication | pending_coin_index for conflict detection, RBF rules. |
| [POL-006](NORMATIVE.md#POL-006) | ⚠️ | Conflict cache | HashMap<Bytes32, SpendBundle>, count + cost limits, retry on confirmation. |
| [POL-007](NORMATIVE.md#POL-007) | ⚠️ | Seen cache | LRU set, pre-validation insertion, max_seen_cache_size=10000. |
| [POL-008](NORMATIVE.md#POL-008) | ⚠️ | Identical spend dedup index | (coin_id, solution_hash) -> bundle_id for cost adjustment. |
| [POL-009](NORMATIVE.md#POL-009) | ⚠️ | Singleton tracking | launcher_id -> Vec<bundle_id> in lineage order. |
| [POL-010](NORMATIVE.md#POL-010) | ⚠️ | Concurrency | RwLock per pool, Mutex for BLS cache, Send + Sync. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
