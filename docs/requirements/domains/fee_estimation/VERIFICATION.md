# Fee Estimation — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [FEE-001](NORMATIVE.md#FEE-001) | ✅ | 3-tier minimum fee estimation | 7 tests: public, empty_returns_zero, below_80_returns_zero, tier2_nonzero, tier3_lowest_fpc_plus_one, tier2_proportional, spend_penalty_included. |
| [FEE-002](NORMATIVE.md#FEE-002) | ✅ | Bucket-based fee tracker | 7 tests: default_construction, custom_bucket_count, bucket_boundaries_logarithmic, block_history_bounded, fee_rate_placement, confirmation_tracking, empty_tracker_state. |
| [FEE-003](NORMATIVE.md#FEE-003) | ✅ | Fee rate estimation with confidence | 8 tests: is_public, insufficient_data_returns_none, sufficient_data_returns_some, no_bucket_meets_threshold, returns_fee_rate_type, target_0_treated_as_1, high_target_returns_result, concurrent_read_safe. |
| [FEE-004](NORMATIVE.md#FEE-004) | ✅ | Confirmed block recording + decay | 8 tests: is_public, single_block_recording, empty_bundles_still_appends, decay_applied_per_block, window_eviction, correct_bucket_placement, manual_seeding_works, on_new_block_integration. |
| [FEE-005](NORMATIVE.md#FEE-005) | ❌ | FeeEstimatorState serialization | Round-trip serde test; snapshot includes all required fields. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
