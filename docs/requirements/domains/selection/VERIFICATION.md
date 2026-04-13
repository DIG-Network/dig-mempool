# Block Candidate Selection — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [SEL-001](NORMATIVE.md#SEL-001) | ⚠️ | Entry point signature | Signature accepts max_block_cost, height, timestamp. Returns Vec<Arc<MempoolItem>> from active pool only. |
| [SEL-002](NORMATIVE.md#SEL-002) | ⚠️ | Pre-selection filtering | Expired, future-timelocked, and about-to-expire items excluded before strategy runs. |
| [SEL-003](NORMATIVE.md#SEL-003) | ⚠️ | Strategy 1: density sort | Package FPC desc, fee desc, virtual_cost asc, height_added asc, bundle_id asc. CPFP-aware greedy. |
| [SEL-004](NORMATIVE.md#SEL-004) | ⚠️ | Strategy 2: whale sort | Package fee desc, package FPC desc, virtual_cost asc, height_added asc, bundle_id asc. CPFP-aware greedy. |
| [SEL-005](NORMATIVE.md#SEL-005) | ⚠️ | Strategy 3: compact sort | Package FPC desc, virtual_cost asc, fee desc, height_added asc, bundle_id asc. CPFP-aware greedy. |
| [SEL-006](NORMATIVE.md#SEL-006) | ⚠️ | Strategy 4: age-weighted sort | height_added asc, package FPC desc, fee desc, bundle_id asc. CPFP-aware greedy. |
| [SEL-007](NORMATIVE.md#SEL-007) | ⚠️ | Best selection comparator | Compare 4 candidates: highest fees, lowest virtual cost, fewest bundles. |
| [SEL-008](NORMATIVE.md#SEL-008) | ⚠️ | Final ordering | Topological (parents first), then FPC desc within layer. Deterministic tiebreakers. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
