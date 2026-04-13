# Block Candidate Selection — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [SEL-001](NORMATIVE.md#SEL-001) | ✅ | Entry point signature | 6 tests: empty pool, active-only, zero budget, spend limit, idempotent, conflict-free output. |
| [SEL-002](NORMATIVE.md#SEL-002) | ✅ | Pre-selection filtering | 7 tests: expired-by-height, expired-by-seconds, future-timelocked, no-timelocks, boundaries, mixed. |
| [SEL-003](NORMATIVE.md#SEL-003) | ✅ | Strategy 1: density sort | 5 tests: higher FPC wins, generous budget, top-2 by FPC, deterministic, conflict-free output. |
| [SEL-004](NORMATIVE.md#SEL-004) | ✅ | Strategy 2: whale sort | 4 tests: whale wins, high-fee among many, whale+small coexist, conflict-free output. |
| [SEL-005](NORMATIVE.md#SEL-005) | ✅ | Strategy 3: compact sort | 4 tests: many small items, generous budget, FPC primary key, deterministic. |
| [SEL-006](NORMATIVE.md#SEL-006) | ✅ | Strategy 4: age-weighted sort | 5 tests: oldest wins, generous budget, height_added recorded, same-height, deterministic. |
| [SEL-007](NORMATIVE.md#SEL-007) | ✅ | Best selection comparator | 5 tests: all-empty, single item, highest fees wins, maximises total fees, deterministic. |
| [SEL-008](NORMATIVE.md#SEL-008) | ✅ | Final ordering | 4 tests: parent before child, layer-0 FPC order, 3-level chain, deterministic. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
