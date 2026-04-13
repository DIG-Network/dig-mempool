# Fee Estimation — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [FEE-001](NORMATIVE.md#FEE-001) | ❌ | 3-tier minimum fee estimation | Unit tests for each tier boundary; utilization thresholds at 79%, 80%, 99%, 100%. |
| [FEE-002](NORMATIVE.md#FEE-002) | ❌ | Bucket-based fee tracker | Unit tests for bucket placement, rolling window eviction, logarithmic spacing. |
| [FEE-003](NORMATIVE.md#FEE-003) | ❌ | Fee rate estimation with confidence | Unit tests for 85% threshold; None on insufficient data; FeeRate type. |
| [FEE-004](NORMATIVE.md#FEE-004) | ❌ | Confirmed block recording + decay | Unit tests for decay factor application; integration with on_new_block(). |
| [FEE-005](NORMATIVE.md#FEE-005) | ❌ | FeeEstimatorState serialization | Round-trip serde test; snapshot includes all required fields. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
