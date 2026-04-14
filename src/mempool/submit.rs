//! Transaction submission: `submit()`, `submit_with_policy()`, `submit_batch()`.
//!
//! Also contains `try_extract_singleton_lineage` — a free function used by
//! the submission pipeline to parse singleton top-layer puzzle reveals.
//!
//! See: [ADM-001..008](docs/requirements/domains/admission/)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chia_sdk_driver::SingletonLayer;
use dig_clvm::CoinRecord;
use dig_clvm::{
    clvmr::serde::node_from_bytes, Allocator, Bytes32, Layer, NodePtr, Puzzle, SpendBundle,
};

use crate::error::MempoolError;
use crate::item::MempoolItem;
use crate::pools::active::sha256_bytes;
use crate::submit::SubmitResult;

use super::Mempool;

/// Try to parse a singleton top-layer puzzle reveal and extract lineage info.
///
/// Deserializes `puzzle_bytes` into CLVM, then parses as
/// `SingletonLayer::<Puzzle>`. On success, extracts `launcher_id` and
/// `inner_puzzle_hash` for the `SingletonLineageInfo`.
///
/// Returns `None` if the puzzle is not a singleton or if parsing fails.
///
/// See: [POL-009](docs/requirements/domains/pools/specs/POL-009.md)
pub(crate) fn try_extract_singleton_lineage(
    coin: dig_clvm::Coin,
    puzzle_bytes: &[u8],
) -> Option<crate::item::SingletonLineageInfo> {
    let mut allocator = Allocator::new();
    let ptr: NodePtr = node_from_bytes(&mut allocator, puzzle_bytes).ok()?;
    let puzzle = Puzzle::parse(&allocator, ptr);
    let layer = SingletonLayer::<Puzzle>::parse_puzzle(&allocator, puzzle).ok()??;

    use dig_clvm::clvm_utils::ToTreeHash;
    let inner_puzzle_hash: [u8; 32] = layer.inner_puzzle.tree_hash().into();

    Some(crate::item::SingletonLineageInfo {
        coin_id: coin.coin_id(),
        parent_id: coin.parent_coin_info,
        parent_parent_id: Bytes32::default(), // available from coin_records if needed
        launcher_id: layer.launcher_id,
        inner_puzzle_hash: Bytes32::from(inner_puzzle_hash),
    })
}

impl Mempool {
    /// Validate and submit a spend bundle to the mempool.
    ///
    /// The mempool internally calls `dig_clvm::validate_spend_bundle()` for
    /// CLVM dry-run + BLS signature verification (ADM-002), then runs the
    /// full admission pipeline: dedup (ADM-003), fee check (ADM-004),
    /// virtual cost (ADM-005), timelock resolution (ADM-006), flag extraction
    /// (ADM-007), conflict detection (CFR-001), RBF (CFR-002-004), CPFP
    /// (CPF-001-005), capacity management (POL-002), and optional admission
    /// policy.
    ///
    /// # Parameters
    ///
    /// - `bundle`: The raw `SpendBundle` to validate and admit. Consumed by value.
    /// - `coin_records`: Coin state for every on-chain coin spent by the bundle.
    ///   For CPFP children, include synthetic records from `get_mempool_coin_record()`.
    /// - `current_height`: Current L2 block height (for timelock resolution).
    /// - `current_timestamp`: Current L2 block timestamp.
    ///
    /// # Returns
    ///
    /// - `Ok(SubmitResult::Success)` — admitted to active pool
    /// - `Ok(SubmitResult::Pending { assert_height })` — valid but timelocked
    /// - `Err(MempoolError)` — rejected (see variant for reason)
    ///
    /// # Concurrency
    ///
    /// Takes `&self` (not `&mut self`). Phase 1 (CLVM) runs lock-free;
    /// Phase 2 (insertion) acquires write lock briefly.
    ///
    /// See: [ADM-001](docs/requirements/domains/admission/specs/ADM-001.md)
    pub fn submit(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
    ) -> Result<SubmitResult, MempoolError> {
        self.submit_inner(
            bundle,
            coin_records,
            current_height,
            current_timestamp,
            None,
        )
    }

    /// Internal pipeline shared by `submit()` and `submit_with_policy()`.
    ///
    /// `admission_policy` is invoked for active-pool items after capacity
    /// management passes, immediately before insertion. Pending-pool items
    /// bypass the policy (they are not yet admitted to the active pool).
    fn submit_inner(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
        admission_policy: Option<&dyn crate::traits::AdmissionPolicy>,
    ) -> Result<SubmitResult, MempoolError> {
        // ── Phase 0: Dedup check (ADM-003) ──
        //
        // Check if we've already seen this bundle ID. This runs BEFORE
        // CLVM validation to prevent DoS via repeated submission of
        // expensive-to-validate bundles. The bundle ID is added to the
        // seen-cache immediately, even before validation — so invalid
        // bundles are also cached and rejected quickly on retry.
        //
        // Chia L1 equivalent: seen_bundle_hashes at mempool_manager.py:298
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L298

        let bundle_id = bundle.name();

        {
            // Read lock: check if already seen
            let seen = self.seen_cache.read().unwrap();
            if seen.contains(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }
        {
            // Write lock: add to seen-cache BEFORE validation (DoS protection).
            // Even if CLVM validation fails, the ID stays in the cache to
            // prevent repeated validation of the same bad bundle.
            let mut seen = self.seen_cache.write().unwrap();
            // Re-check under write lock (another thread may have inserted)
            if seen.contains(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
            seen.insert(bundle_id);
        }

        // POL-001 / POL-007: Check active pool, pending pool, and conflict cache.
        //
        // Handles the case where the seen_cache has evicted a bundle ID that is
        // still in another pool. Without these checks, a seen_cache miss would
        // allow re-validation and double-insertion of an already-present bundle.
        //
        // Each check acquires a read lock independently. TOCTOU safety: Phase 2
        // re-checks under the write lock before inserting into each pool.
        //
        // Chia L1: mempool_manager.py checks active pool at line ~560.
        {
            let pool = self.pool.read().unwrap();
            if pool.items.contains_key(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }
        {
            let pending = self.pending.read().unwrap();
            if pending.pending.contains_key(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }
        {
            let conflict = self.conflict.read().unwrap();
            if conflict.contains(&bundle_id) {
                return Err(MempoolError::AlreadySeen(bundle_id));
            }
        }

        // ── Phase 1: Lock-free CLVM validation (ADM-002) ──
        //
        // This is the expensive step. It runs WITHOUT holding the mempool
        // write lock, allowing concurrent validation of multiple submissions.
        //
        // dig-clvm performs:
        //   1. Duplicate spend detection (validate.rs:39-48)
        //   2. Coin existence check (validate.rs:50-62)
        //   3. CLVM execution via chia_consensus::run_spendbundle() (validate.rs:79-102)
        //   4. BLS aggregate signature verification via BlsCache (validate.rs:107-113)
        //   5. Cost enforcement (validate.rs:126-131)
        //   6. Addition/removal extraction (validate.rs:134-146)
        //   7. Conservation check: sum(inputs) >= sum(outputs) (validate.rs:148-159)
        //
        // Chia L1 equivalent: validate_clvm_and_signature() at mempool_manager.py:445
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L445

        // ── CPF-001/CPF-002: Snapshot mempool_coins for ephemeral coin support ──
        //
        // CPFP children spend coins created by unconfirmed parent bundles.
        // These coins are not in `coin_records` (they're not on-chain yet), but
        // the CLVM validator needs to know they exist to avoid CoinNotFound errors.
        //
        // A brief pool read lock snapshots the current set of mempool coin IDs.
        // Phase 2 re-validates under the write lock using the live index.
        // TOCTOU note: if a parent is evicted between this snapshot and Phase 2,
        // the Phase 2 dependency resolution will return CoinNotFound (correct).
        let ephemeral_coins: HashSet<Bytes32> = {
            let pool = self.pool.read().unwrap();
            pool.mempool_coins.keys().copied().collect()
        };

        // Build the validation context from caller-provided chain state.
        // `coin_records` contains on-chain coins; `ephemeral_coins` covers
        // unconfirmed coins from active mempool parents (CPFP support).
        let ctx = dig_clvm::ValidationContext {
            height: current_height as u32,
            timestamp: current_timestamp,
            constants: self.constants.clone(),
            coin_records: coin_records.clone(),
            ephemeral_coins,
        };

        // Use MEMPOOL_MODE for strict validation:
        // - Reject unknown condition opcodes
        // - Stricter cost accounting
        // - Canonical encoding checks (enables dedup eligibility flags)
        //
        // Chia L1: MEMPOOL_MODE at mempool.py:13
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L13
        let config = dig_clvm::ValidationConfig {
            flags: dig_clvm::chia_consensus::flags::MEMPOOL_MODE,
            ..Default::default()
        };

        // Acquire the BLS cache mutex for signature verification.
        // The mutex is only held during the validate_spend_bundle() call.
        // This serializes BLS pairing operations but CLVM execution is the
        // bottleneck, not signature verification.
        let mut bls_cache = self.bls_cache.lock().unwrap();

        // The ? operator converts dig_clvm::ValidationError to MempoolError
        // via the From<ValidationError> impl in error.rs (API-004).
        let spend_result =
            dig_clvm::validate_spend_bundle(&bundle, &ctx, &config, Some(&mut bls_cache))?;

        // Release the BLS cache before further processing
        drop(bls_cache);

        // ── ADM-004: Fee extraction + RESERVE_FEE check ──
        //
        // The fee is already computed by chia-consensus:
        //   fee = removal_amount - addition_amount
        // The RESERVE_FEE condition (opcode 52) is pre-summed in
        // conditions.reserve_fee. If the actual fee is less than the
        // declared minimum, reject the bundle.
        //
        // Chia L1 equivalent: mempool_manager.py:728
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L728
        let fee = spend_result.fee;
        let reserve_fee = spend_result.conditions.reserve_fee;
        if fee < reserve_fee {
            return Err(MempoolError::InsufficientFee {
                required: reserve_fee,
                available: fee,
            });
        }

        // ── ADM-005: Cost and virtual cost computation ──
        //
        // The CLVM cost is from chia-consensus. We also compute virtual cost
        // (adds spend penalty) and check against config.max_bundle_cost.
        //
        // Chia L1: max_tx_clvm_cost check at mempool_manager.py:733
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L733
        let cost = spend_result.conditions.cost;
        if cost > self.config.max_bundle_cost {
            return Err(MempoolError::CostExceeded {
                cost,
                max: self.config.max_bundle_cost,
            });
        }
        let num_spends = bundle.coin_spends.len();
        let virtual_cost = MempoolItem::compute_virtual_cost(cost, num_spends);
        let fee_per_virtual_cost_scaled = MempoolItem::compute_fpc_scaled(fee, virtual_cost);

        // ── ADM-006: Timelock resolution ──
        //
        // Resolve relative timelocks to absolute values using coin records.
        // Then check for impossible constraints and expiry.
        //
        // Chia L1 equivalent: compute_assert_height() at mempool_manager.py:81-126
        // https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool_manager.py#L81
        let mut assert_height: Option<u64> = None;
        let mut assert_seconds: Option<u64> = None;
        let mut assert_before_height: Option<u64> = None;
        let mut assert_before_seconds: Option<u64> = None;

        // Bundle-level absolute timelocks
        if spend_result.conditions.height_absolute > 0 {
            assert_height = Some(
                assert_height
                    .unwrap_or(0)
                    .max(spend_result.conditions.height_absolute as u64),
            );
        }
        if spend_result.conditions.seconds_absolute > 0 {
            assert_seconds = Some(
                assert_seconds
                    .unwrap_or(0)
                    .max(spend_result.conditions.seconds_absolute),
            );
        }
        if let Some(bha) = spend_result.conditions.before_height_absolute {
            let val = bha as u64;
            assert_before_height = Some(assert_before_height.map_or(val, |v: u64| v.min(val)));
        }
        if let Some(bsa) = spend_result.conditions.before_seconds_absolute {
            assert_before_seconds = Some(assert_before_seconds.map_or(bsa, |v: u64| v.min(bsa)));
        }

        // Per-spend relative timelocks resolved to absolute
        for spend_cond in &spend_result.conditions.spends {
            let coin_id = spend_cond.coin_id;
            // Look up the coin record for this spend's coin.
            // If not found (CPFP candidate), use current_height/timestamp
            // matching Chia's ephemeral coin handling (mempool_manager.py:716-722).
            let (confirmed_index, timestamp) = match coin_records.get(&coin_id) {
                Some(record) => (record.confirmed_block_index as u64, record.timestamp),
                None => (current_height, current_timestamp),
            };

            if let Some(hr) = spend_cond.height_relative {
                let resolved = confirmed_index + hr as u64;
                assert_height = Some(assert_height.unwrap_or(0).max(resolved));
            }
            if let Some(sr) = spend_cond.seconds_relative {
                let resolved = timestamp + sr;
                assert_seconds = Some(assert_seconds.unwrap_or(0).max(resolved));
            }
            if let Some(bhr) = spend_cond.before_height_relative {
                let resolved = confirmed_index + bhr as u64;
                assert_before_height =
                    Some(assert_before_height.map_or(resolved, |v: u64| v.min(resolved)));
            }
            if let Some(bsr) = spend_cond.before_seconds_relative {
                let resolved = timestamp + bsr;
                assert_before_seconds =
                    Some(assert_before_seconds.map_or(resolved, |v: u64| v.min(resolved)));
            }
        }

        // Impossible constraint detection
        // Chia: mempool_manager.py:791-796
        if let (Some(ah), Some(abh)) = (assert_height, assert_before_height) {
            if abh <= ah {
                return Err(MempoolError::ImpossibleTimelocks);
            }
        }
        if let (Some(as_), Some(abs)) = (assert_seconds, assert_before_seconds) {
            if abs <= as_ {
                return Err(MempoolError::ImpossibleTimelocks);
            }
        }

        // Expiry check
        if let Some(abh) = assert_before_height {
            if current_height >= abh {
                return Err(MempoolError::Expired);
            }
        }
        if let Some(abs) = assert_before_seconds {
            if current_timestamp >= abs {
                return Err(MempoolError::Expired);
            }
        }

        // Pending determination: if assert_height > current_height, route to pending
        let is_pending = matches!(assert_height, Some(ah) if ah > current_height)
            || matches!(assert_seconds, Some(as_) if as_ > current_timestamp);

        // ── ADM-007: Dedup/FF flag extraction ──
        //
        // Read ELIGIBLE_FOR_DEDUP (0x1) and ELIGIBLE_FOR_FF (0x4) flags from
        // each spend's conditions.flags. These are set by chia-consensus's
        // MempoolVisitor during CLVM execution when MEMPOOL_MODE is active.
        // The mempool reads these flags; it does NOT compute them.
        //
        // eligible_for_dedup: true only if ALL spends have the flag.
        // singleton_lineage: populated if any spend has ELIGIBLE_FOR_FF and
        // the caller has confirmed FF eligibility (not yet wired — needs
        // SingletonLayer::parse_puzzle() from chia-sdk-driver).
        //
        // Chia L1: mempool_manager.py:662-663 (dedup flag check)
        //          mempool_manager.py:666-667 (FF flag check)
        let eligible_for_dedup = spend_result
            .conditions
            .spends
            .iter()
            .all(|s| s.flags & 0x1 != 0)
            || spend_result.conditions.spends.is_empty(); // vacuously true for 0 spends
        let any_ff_eligible = spend_result
            .conditions
            .spends
            .iter()
            .any(|s| s.flags & 0x4 != 0);

        // ── ADM-007 / POL-009: Extract singleton lineage if FF-eligible ──
        //
        // When chia-consensus marks a spend ELIGIBLE_FOR_FF (0x4), it is a singleton
        // top-layer v1.1 spend. Parse the puzzle reveal to extract the `launcher_id`
        // and `inner_puzzle_hash` for the singleton chain index (POL-009).
        //
        // Requires: enable_singleton_ff = true (default), and the puzzle reveal must
        // be a valid curried singleton top-layer application.
        //
        // Detection is gated by `enable_singleton_ff` to allow disabling the feature.
        let singleton_lineage = if any_ff_eligible && self.config.enable_singleton_ff {
            // Find the first FF-eligible spend and try to parse its puzzle as a singleton.
            bundle
                .coin_spends
                .iter()
                .zip(spend_result.conditions.spends.iter())
                .find(|(_, sc)| sc.flags & 0x4 != 0)
                .and_then(|(cs, _)| {
                    try_extract_singleton_lineage(cs.coin, cs.puzzle_reveal.as_ref())
                })
        } else {
            None
        };
        // ── Build MempoolItem (split by routing path) ──
        //
        // `removals` = coin IDs spent by this bundle (from SpendResult.removals).
        // `additions` = coins created by this bundle (from SpendResult.additions).
        //
        // Item construction is split between the pending and active paths because
        // the active path (POL-008) needs to compute cost_saving under the pool
        // write lock — requiring it to be built inside the lock scope.
        //
        // Chia L1 equivalent: Mempool.add_to_pool() at
        // [mempool.py:273](https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/full_node/mempool.py#L273)

        // Extract SpendResult fields (partial moves are safe — spend_result
        // is no longer borrowed from the timelock/flag loops above).
        let removals: Vec<Bytes32> = spend_result.removals.iter().map(|c| c.coin_id()).collect();
        let additions = spend_result.additions;
        let conditions = spend_result.conditions;

        // ── POL-008: Pre-compute dedup keys (lock-free) ──
        //
        // Compute (coin_id, sha256(solution)) keys for each eligible CoinSpend.
        // Done BEFORE `bundle` is consumed (item construction moves it).
        // Only meaningful for active-pool submissions — pending items don't
        // participate in dedup (POL-008 applies to the active pool only).
        //
        // We hash the solution bytes using raw SHA-256 (not CLVM tree hash)
        // to match the Chia L1 `IdenticalSpendDedup` implementation.
        let dedup_keys: Vec<(Bytes32, Bytes32)> =
            if !is_pending && eligible_for_dedup && self.config.enable_identical_spend_dedup {
                bundle
                    .coin_spends
                    .iter()
                    .map(|cs| (cs.coin.coin_id(), sha256_bytes(cs.solution.as_ref())))
                    .collect()
            } else {
                vec![]
            };

        if is_pending {
            // ── Phase 2a: Route to pending pool (POL-004) ──
            //
            // Build the item before acquiring the lock — item construction is cheap
            // and the pending path requires removals/fee/fpc for RBF checks.
            // Pending items do NOT participate in dedup (POL-008 is active pool only).
            //
            // Chia L1: PendingTxCache.add() at pending_tx_cache.py:60-76
            let item = Arc::new(MempoolItem {
                spend_bundle: bundle,
                spend_bundle_id: bundle_id,
                fee,
                cost,
                virtual_cost,
                fee_per_virtual_cost_scaled,
                package_fee: fee,
                package_virtual_cost: virtual_cost,
                package_fee_per_virtual_cost_scaled: fee_per_virtual_cost_scaled,
                descendant_score: fee_per_virtual_cost_scaled,
                additions,
                removals,
                height_added: current_height,
                conditions,
                num_spends,
                assert_height,
                assert_seconds,
                assert_before_height,
                assert_before_seconds,
                depends_on: HashSet::new(),
                depth: 0,
                eligible_for_dedup,
                singleton_lineage: singleton_lineage.clone(),
                cost_saving: 0,
                effective_virtual_cost: virtual_cost,
                dedup_keys: vec![],
            });

            let mut pending = self.pending.write().unwrap();
            // TOCTOU safety: re-check under write lock.
            if pending.pending.contains_key(&bundle_id) {
                return Ok(SubmitResult::Pending {
                    assert_height: assert_height.unwrap_or(0),
                });
            }

            // ── POL-005: Pending-vs-pending conflict detection + RBF ──
            //
            // If this item spends a coin already spent by a pending item,
            // apply RBF rules: superset check, FPC must be strictly higher,
            // fee bump must meet minimum. Failed RBF rejects the new item
            // without touching the conflict cache (pending RBF ≠ active RBF).
            let conflict_ids: Vec<Bytes32> = {
                let mut seen_conflicts = HashSet::new();
                item.removals
                    .iter()
                    .filter_map(|coin_id| pending.pending_coin_index.get(coin_id).copied())
                    .filter(|id| seen_conflicts.insert(*id))
                    .collect()
            };

            if !conflict_ids.is_empty() {
                // Superset rule: new item must spend every coin spent by each
                // conflicting item — it must be a strict superset of their removals.
                for cid in &conflict_ids {
                    let conflict = pending.pending.get(cid).unwrap();
                    for &coin_id in &conflict.removals {
                        if !item.removals.contains(&coin_id) {
                            return Err(MempoolError::RbfNotSuperset);
                        }
                    }
                }

                // FPC rule: aggregate all conflicting items and compare FPC.
                let total_conflict_fee: u64 = conflict_ids
                    .iter()
                    .map(|cid| pending.pending.get(cid).unwrap().fee)
                    .sum();
                let total_conflict_vc: u64 = conflict_ids
                    .iter()
                    .map(|cid| pending.pending.get(cid).unwrap().virtual_cost)
                    .sum();
                let conflict_fpc =
                    MempoolItem::compute_fpc_scaled(total_conflict_fee, total_conflict_vc);
                if item.fee_per_virtual_cost_scaled <= conflict_fpc {
                    return Err(MempoolError::RbfFpcNotHigher);
                }

                // Fee bump rule: new fee must exceed total conflict fee by at least
                // config.min_rbf_fee_bump (default 10M mojos).
                let required = total_conflict_fee.saturating_add(self.config.min_rbf_fee_bump);
                if item.fee < required {
                    return Err(MempoolError::RbfBumpTooLow {
                        required,
                        provided: item.fee,
                    });
                }

                // All RBF rules passed — evict the conflicting pending items.
                for cid in &conflict_ids {
                    pending.remove(cid);
                }
            }

            if pending.pending.len() >= self.config.max_pending_count {
                return Err(MempoolError::PendingPoolFull);
            }
            if pending.pending_cost.saturating_add(virtual_cost) > self.config.max_pending_cost {
                return Err(MempoolError::PendingPoolFull);
            }
            let item_ref = Arc::clone(&item);
            pending.insert(item);
            drop(pending);
            let pending_height = assert_height.unwrap_or(0);
            self.fire_hooks(|h| h.on_pending_added(&item_ref));
            return Ok(SubmitResult::Pending {
                assert_height: pending_height,
            });
        }

        // ── Phase 2b: Insert into active pool ──
        {
            let mut pool = self.pool.write().unwrap();
            // TOCTOU safety: re-check under write lock before inserting.
            // Another thread may have inserted the same bundle between our
            // Phase 0 check and now. Idempotent success avoids a double-insertion.
            if pool.items.contains_key(&bundle_id) {
                return Ok(SubmitResult::Success);
            }

            // ── POL-002: Spend count limit ──
            //
            // Hard cap on total active spends to bound block validation time.
            // Not subject to eviction — reject immediately if the budget is exhausted.
            //
            // Chia L1: maximum_spends check at mempool.py:385-390
            if pool.total_spends.saturating_add(num_spends) > self.config.max_spends_per_block {
                return Err(MempoolError::TooManySpends {
                    count: pool.total_spends.saturating_add(num_spends),
                    max: self.config.max_spends_per_block,
                });
            }

            // ── CPF-002: Dependency resolution ──
            //
            // For each coin this bundle spends (each removal):
            // - If the coin is in `coin_records` (on-chain) → no CPFP dependency.
            // - If the coin is in `pool.mempool_coins` (unconfirmed) → CPFP dep.
            // - If the coin is in neither → reject with CoinNotFound.
            //
            // This is a dig-mempool extension; Chia L1 does not support CPFP.
            let mut depends_on: HashSet<Bytes32> = HashSet::new();
            for &coin_id in &removals {
                if coin_records.contains_key(&coin_id) {
                    continue; // on-chain coin: no CPFP dependency
                }
                match pool.mempool_coins.get(&coin_id) {
                    Some(&parent_id) => {
                        depends_on.insert(parent_id);
                    }
                    None => {
                        return Err(MempoolError::CoinNotFound(coin_id));
                    }
                }
            }

            // ── CPF-002: Compute dependency depth ──
            //
            // depth = 0 for on-chain-only items; depth = 1 + max(parent.depth)
            // for CPFP children. Uses `pool.items` which already holds parents.
            let depth: u32 = if depends_on.is_empty() {
                0
            } else {
                1 + depends_on
                    .iter()
                    .filter_map(|id| pool.items.get(id))
                    .map(|p| p.depth)
                    .max()
                    .unwrap_or(0)
            };

            // ── CPF-003: Maximum dependency depth ──
            //
            // Reject if depth exceeds the configured limit (default 25).
            // Keeps cascade eviction, package fee computation, and block
            // selection traversals bounded.
            if depth > self.config.max_dependency_depth {
                return Err(MempoolError::DependencyTooDeep {
                    depth,
                    max: self.config.max_dependency_depth,
                });
            }

            // ── CPF-004: Defensive cycle check ──
            //
            // Walk the ancestor chain from this bundle's direct parents.
            // If we encounter `bundle_id` itself → DependencyCycle.
            // In the UTXO coinset model, cycles are structurally impossible
            // (requiring a SHA-256 hash collision). This check guards against
            // implementation bugs or corrupted state.
            if !depends_on.is_empty() {
                let mut to_visit: Vec<Bytes32> = depends_on.iter().copied().collect();
                let mut visited: HashSet<Bytes32> = HashSet::new();
                while let Some(ancestor_id) = to_visit.pop() {
                    if ancestor_id == bundle_id {
                        return Err(MempoolError::DependencyCycle);
                    }
                    if !visited.insert(ancestor_id) {
                        continue;
                    }
                    if let Some(ancestor_item) = pool.items.get(&ancestor_id) {
                        to_visit.extend(ancestor_item.depends_on.iter().copied());
                    }
                }
            }

            // ── CPF-005: Package fee rate computation ──
            //
            // Aggregate fees and costs across the entire ancestor chain.
            // Uses each parent's `package_fee` / `package_virtual_cost` so that
            // transitive ancestors are included without explicit traversal.
            let (package_fee, package_virtual_cost) = if depends_on.is_empty() {
                (fee, virtual_cost)
            } else {
                let ancestor_fee: u64 = depends_on
                    .iter()
                    .filter_map(|id| pool.items.get(id))
                    .map(|p| p.package_fee)
                    .sum();
                let ancestor_vc: u64 = depends_on
                    .iter()
                    .filter_map(|id| pool.items.get(id))
                    .map(|p| p.package_virtual_cost)
                    .sum();
                (
                    fee.saturating_add(ancestor_fee),
                    virtual_cost.saturating_add(ancestor_vc),
                )
            };
            let package_fpc_scaled =
                MempoolItem::compute_fpc_scaled(package_fee, package_virtual_cost);

            // ── CFR-001 through CFR-005: Active pool conflict detection + RBF ──
            //
            // For each coin spent by the new bundle, look up `coin_index` to find
            // any existing active bundle spending the same coin. Collect all unique
            // conflicting bundle IDs.
            //
            // If conflicts are found, evaluate RBF rules (CFR-002, CFR-003, CFR-004):
            //   1. Superset rule: new bundle must spend every coin from each conflict.
            //   2. FPC strictly higher: new bundle's fee rate exceeds aggregate rate.
            //   3. Minimum fee bump: absolute fee exceeds sum of conflicts + MIN_RBF.
            //
            // On failure: add new bundle to conflict cache (CFR-005) and return error.
            // On success: remove conflicting items (CFR-006 partial — cascade eviction
            //   of CPFP dependents is deferred to CPF-007).
            //
            // Lock note: `add_to_conflict_cache()` acquires `self.conflict.write()` while
            // `pool` (self.pool.write()) is held. This follows the POL-010 ordering:
            //   pool_lock → conflict_lock.
            //
            // Chia L1 equivalents:
            //   check_removals(): mempool_manager.py:229-292
            //   can_replace():    mempool_manager.py:1077-1126
            {
                // Build a set of dedup keys for O(1) lookup during conflict
                // filtering.  A "conflict" that is actually a dedup-waiter
                // relationship (same coin, same solution, dedup-eligible) must
                // NOT go through RBF — the bundle is admitted as a waiter.
                // Chia handles the same special case at mempool_manager.py:270-288.
                let dedup_key_set: HashSet<(Bytes32, Bytes32)> =
                    dedup_keys.iter().copied().collect();

                let mut seen_conflicts = HashSet::new();
                let conflict_ids: Vec<Bytes32> = removals
                    .iter()
                    .filter_map(|coin_id| {
                        let conflict_bundle_id = pool.coin_index.get(coin_id).copied()?;
                        // If the incoming bundle's spend of this coin is a dedup match
                        // (same solution already indexed in dedup_index), treat as a
                        // waiter, not a conflict — exclude from RBF evaluation.
                        if !dedup_key_set.is_empty() {
                            // Find the solution hash for this coin in the incoming bundle.
                            // dedup_key_set is keyed by (coin_id, sol_hash) so we need
                            // to search for any entry whose first element is *coin_id*.
                            let is_dedup = dedup_keys.iter().any(|(cid, sol_hash)| {
                                cid == coin_id && pool.dedup_index.contains_key(&(*cid, *sol_hash))
                            });
                            if is_dedup {
                                return None; // waiter — not a real conflict
                            }
                        }
                        Some(conflict_bundle_id)
                    })
                    .filter(|id| seen_conflicts.insert(*id))
                    .collect();

                if !conflict_ids.is_empty() {
                    // Fetch all conflicting items for RBF evaluation.
                    // coin_index is kept in sync with items, so all IDs resolve.
                    let conflicting_items: Vec<Arc<MempoolItem>> = conflict_ids
                        .iter()
                        .filter_map(|id| pool.items.get(id).cloned())
                        .collect();

                    let total_conflict_fee: u64 = conflicting_items.iter().map(|i| i.fee).sum();
                    let total_conflict_vc: u64 =
                        conflicting_items.iter().map(|i| i.virtual_cost).sum();

                    // ── CFR-002: Superset rule ──
                    //
                    // Every coin spent by any conflicting bundle must also appear in the
                    // new bundle's removals. Prevents "freeing" a conflicting bundle's
                    // coins while only partially replacing it.
                    let new_removals_set: HashSet<Bytes32> = removals.iter().copied().collect();
                    for conflict_item in &conflicting_items {
                        for &coin_id in &conflict_item.removals {
                            if !new_removals_set.contains(&coin_id) {
                                self.add_to_conflict_cache(bundle, virtual_cost);
                                return Err(MempoolError::RbfNotSuperset);
                            }
                        }
                    }

                    // ── CFR-003: FPC must be strictly higher than aggregate ──
                    //
                    // The new bundle's fee-per-virtual-cost must strictly exceed the
                    // combined FPC of all conflicting items. Uses scaled integer
                    // arithmetic (FPC_SCALE) to avoid floating-point non-determinism.
                    //
                    // Chia L1: uses fee_per_cost (float); we use scaled integers.
                    let conflict_fpc =
                        MempoolItem::compute_fpc_scaled(total_conflict_fee, total_conflict_vc);
                    if fee_per_virtual_cost_scaled <= conflict_fpc {
                        self.add_to_conflict_cache(bundle, virtual_cost);
                        return Err(MempoolError::RbfFpcNotHigher);
                    }

                    // ── CFR-004: Minimum absolute fee bump ──
                    //
                    // The new bundle's absolute fee must exceed the sum of all conflicting
                    // fees by at least `min_rbf_fee_bump` (default 10M mojos, matching
                    // Chia's MEMPOOL_MIN_FEE_INCREASE). Prevents "penny-shaving" DoS.
                    let required_fee =
                        total_conflict_fee.saturating_add(self.config.min_rbf_fee_bump);
                    if fee < required_fee {
                        self.add_to_conflict_cache(bundle, virtual_cost);
                        return Err(MempoolError::RbfBumpTooLow {
                            required: required_fee,
                            provided: fee,
                        });
                    }

                    // ── CFR-006 + CPF-007: Remove conflicting items with cascade ──
                    //
                    // All RBF conditions satisfied — evict the conflicting active items
                    // and recursively evict all their CPFP dependents (CPF-007).
                    // Cascaded children are irrecoverably invalid (their input coins no
                    // longer exist) and do NOT go to the conflict cache.
                    //
                    // Chia L1 equivalent: remove_from_pool() at mempool.py:303-349
                    for conflict_id in &conflict_ids {
                        pool.cascade_evict(conflict_id);
                    }
                }
            }

            // ── POL-008: Compute cost_saving under pool write lock ──
            //
            // Check how many of this item's dedup keys are already borne by
            // another active bundle. Each matching key saves approximately
            // `cost / num_spends` in effective block cost (uniform distribution
            // approximation — chia-consensus 0.26 does not expose per-spend cost).
            //
            // This must be computed under the write lock so that the dedup_index
            // state and item construction are atomic.
            let num_deduped = dedup_keys
                .iter()
                .filter(|k| pool.dedup_index.contains_key(k))
                .count();
            let per_spend_cost = if num_spends > 0 {
                cost / num_spends as u64
            } else {
                0
            };
            let cost_saving = per_spend_cost.saturating_mul(num_deduped as u64);
            let effective_virtual_cost = virtual_cost.saturating_sub(cost_saving);

            // ── CPF-008: Cross-bundle announcement validation (trivially passes) ──
            //
            // For CPFP items (non-empty depends_on), verify that any ancestor
            // announcements the child asserts can be satisfied by the ancestor chain.
            // Per spec section 5.9: assertions referencing non-ancestor bundles are
            // NOT rejected — they may be satisfied by other bundles in the same block.
            // Therefore this step never rejects; it is a best-effort consistency check.
            // (See CPF-008 spec for full details.)
            //
            // Full announcement validation will be implemented when block selection
            // and CPFP semantics are exercised end-to-end (CPF-008 acceptance tests).

            // Build the item inside the lock so cost_saving and CPFP fields are
            // consistent with the pool state at insertion time.
            let item = Arc::new(MempoolItem {
                spend_bundle: bundle,
                spend_bundle_id: bundle_id,
                fee,
                cost,
                virtual_cost,
                fee_per_virtual_cost_scaled,
                package_fee,
                package_virtual_cost,
                package_fee_per_virtual_cost_scaled: package_fpc_scaled,
                descendant_score: fee_per_virtual_cost_scaled,
                additions,
                removals,
                height_added: current_height,
                conditions,
                num_spends,
                assert_height,
                assert_seconds,
                assert_before_height,
                assert_before_seconds,
                depends_on,
                depth,
                eligible_for_dedup,
                singleton_lineage,
                cost_saving,
                effective_virtual_cost,
                dedup_keys,
            });

            // ── POL-002: Capacity management — evict if needed ──
            //
            // If the new item doesn't fit within max_total_cost, evict the
            // lowest-descendant_score items until there is room. Returns
            // MempoolFull if the new item can't displace any candidate.
            //
            // Chia L1 equivalent: add_to_pool() eviction loop at mempool.py:395
            if pool.total_cost.saturating_add(virtual_cost) > self.config.max_total_cost {
                pool.evict_for(
                    &item,
                    self.config.max_total_cost,
                    current_height,
                    self.config.expiry_protection_blocks,
                )?;
            }

            // ── API-007: Custom admission policy check ──
            //
            // Called under the pool write lock after all standard checks pass,
            // so the policy sees a consistent view of existing items and the
            // new item is guaranteed not to conflict.
            if let Some(policy) = admission_policy {
                let existing: Vec<Arc<MempoolItem>> = pool.items.values().cloned().collect();
                policy
                    .check(&item, &existing)
                    .map_err(MempoolError::PolicyRejected)?;
            }

            let item_ref = Arc::clone(&item);
            pool.insert(item);

            // ── CPF-006: Update ancestor descendant_scores ──
            //
            // After inserting the new item, walk up its ancestor chain and update
            // each ancestor's `descendant_score` to max(current_score, child_pkg_fpc).
            // This protects valuable parents from capacity eviction.
            pool.update_descendant_scores_on_add(&bundle_id);

            // ── LCY-005: Fire on_item_added hook ──
            drop(pool);
            self.fire_hooks(|h| h.on_item_added(&item_ref));
        }

        Ok(SubmitResult::Success)
    }

    /// Submit with a custom admission policy applied after standard validation.
    ///
    /// Identical to `submit()` except the provided `policy` is invoked after
    /// all standard checks pass (Phase 2, step 16). If the policy returns
    /// `Err(reason)`, the bundle is rejected with `MempoolError::PolicyRejected`.
    ///
    /// # Parameters
    ///
    /// Same as `submit()` plus:
    /// - `policy`: A `&dyn AdmissionPolicy` that inspects the validated item
    ///   and current pool state to make a domain-specific admission decision.
    ///
    /// See: [API-007](docs/requirements/domains/crate_api/specs/API-007.md)
    pub fn submit_with_policy(
        &self,
        bundle: SpendBundle,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
        policy: &dyn crate::traits::AdmissionPolicy,
    ) -> Result<SubmitResult, MempoolError> {
        self.submit_inner(
            bundle,
            coin_records,
            current_height,
            current_timestamp,
            Some(policy),
        )
    }

    /// Validate and submit multiple bundles in batch.
    ///
    /// Each bundle goes through the full admission pipeline independently.
    /// Results are returned in the same order as inputs. Earlier entries in
    /// the batch can create coins that later entries depend on (CPFP), and
    /// dedup applies across entries (identical bundles in the same batch
    /// will have the second rejected as AlreadySeen).
    ///
    /// # Concurrency Note
    ///
    /// Currently processes sequentially. Parallel Phase 1 (CLVM validation
    /// across bundles) will be added when rayon or similar is integrated.
    /// The sequential approach is correct and sufficient for initial release.
    ///
    /// See: [ADM-008](docs/requirements/domains/admission/specs/ADM-008.md)
    pub fn submit_batch(
        &self,
        bundles: Vec<SpendBundle>,
        coin_records: &HashMap<Bytes32, CoinRecord>,
        current_height: u64,
        current_timestamp: u64,
    ) -> Vec<Result<SubmitResult, MempoolError>> {
        bundles
            .into_iter()
            .map(|bundle| self.submit(bundle, coin_records, current_height, current_timestamp))
            .collect()
    }
}
