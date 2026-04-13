//! MempoolItem — the central data type for a validated, admitted transaction.
//!
//! # Overview
//!
//! A `MempoolItem` represents a spend bundle that has passed CLVM validation
//! and been admitted to the mempool. Items are immutable once created and are
//! stored internally as `Arc<MempoolItem>` for cheap sharing across indexes
//! and API consumers.
//!
//! # Chia L1 Correspondence
//!
//! This struct flattens Chia's `MempoolItem` ([mempool_item.py:45-120]) and
//! `BundleCoinSpend` ([mempool_item.py:25-42]) into a single type, with
//! extensions for CPFP package fees, descendant scoring, and singleton
//! fast-forward tracking.
//!
//! # Key Differences from Chia
//!
//! - **Integer-scaled FPC**: Chia uses `fee / virtual_cost` as a float
//!   ([mempool_item.py:76-77]). We use `fee * FPC_SCALE / virtual_cost`
//!   as u128 for determinism across nodes.
//! - **CPFP package fields**: `package_fee`, `package_virtual_cost`,
//!   `package_fee_per_virtual_cost_scaled` aggregate ancestor chain metrics
//!   for child-pays-for-parent block selection.
//! - **Descendant score**: `max(own_fpc, max(descendant.package_fpc))` protects
//!   low-fee parents with valuable children from eviction.
//! - **Dependency tracking**: `depends_on` and `depth` track CPFP chains.
//!
//! # Spec Reference
//!
//! - [SPEC.md Section 2.2](../docs/resources/SPEC.md) — MempoolItem definition
//! - [API-002](../docs/requirements/domains/crate_api/specs/API-002.md) — Requirement spec
//!
//! [mempool_item.py:45-120]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L45
//! [mempool_item.py:25-42]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L25
//! [mempool_item.py:76-77]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L76

use std::collections::HashSet;

use dig_clvm::chia_consensus::owned_conditions::OwnedSpendBundleConditions;
use dig_clvm::{Bytes32, Coin, SpendBundle};

use crate::config::{FPC_SCALE, SPEND_PENALTY_COST};

/// Singleton lineage information for fast-forward optimization.
///
/// Tracks the state of a singleton coin through its lineage chain.
/// Extends Chia's `UnspentLineageInfo` ([mempool_item.py:18-22]) with
/// `launcher_id` and `inner_puzzle_hash` for puzzle-level identification.
///
/// # Usage
///
/// Set on `MempoolItem.singleton_lineage` when the spend is detected as a
/// singleton via the `ELIGIBLE_FOR_FF` flag from chia-consensus's
/// `MempoolVisitor`. The caller extracts lineage data using
/// `SingletonLayer::parse_puzzle()` from chia-sdk-driver.
///
/// [mempool_item.py:18-22]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L18
#[derive(Debug, Clone)]
pub struct SingletonLineageInfo {
    /// The current unspent singleton coin ID.
    pub coin_id: Bytes32,
    /// The parent of the current singleton.
    pub parent_id: Bytes32,
    /// The grandparent (parent's parent).
    pub parent_parent_id: Bytes32,
    /// The singleton's launcher ID (immutable identity across all versions).
    pub launcher_id: Bytes32,
    /// The singleton's inner puzzle hash.
    pub inner_puzzle_hash: Bytes32,
}

/// A validated, admitted transaction in the mempool.
///
/// Immutable once created. Stored as `Arc<MempoolItem>` in all mempool indexes.
///
/// # Field Groups
///
/// | Group | Fields | Source |
/// |-------|--------|--------|
/// | Identity | `spend_bundle`, `spend_bundle_id` | Caller submission + `SpendBundle::name()` |
/// | Individual metrics | `fee`, `cost`, `virtual_cost`, `fee_per_virtual_cost_scaled` | dig-clvm `SpendResult` + computation |
/// | Package metrics (CPFP) | `package_fee`, `package_virtual_cost`, `package_fee_per_virtual_cost_scaled` | Ancestor aggregation |
/// | Eviction | `descendant_score` | max(own_fpc, descendant package_fpc) |
/// | State deltas | `additions`, `removals` | dig-clvm `SpendResult` |
/// | Metadata | `height_added`, `conditions`, `num_spends` | Caller + dig-clvm |
/// | Timelocks | `assert_height`, `assert_before_height`, `assert_before_seconds` | Timelock resolution |
/// | Dependencies | `depends_on`, `depth` | CPFP resolution |
/// | Optimization | `eligible_for_dedup`, `singleton_lineage` | chia-consensus flags |
///
/// # See Also
///
/// - [`config::SPEND_PENALTY_COST`] — penalty per spend in virtual cost
/// - [`config::FPC_SCALE`] — scaling factor for integer FPC arithmetic
/// - [SPEC.md Section 2.2](../docs/resources/SPEC.md)
pub struct MempoolItem {
    // ── Identity ──
    /// The original spend bundle submitted by the caller.
    /// Type: `chia-protocol::SpendBundle` (via dig-clvm re-export).
    pub spend_bundle: SpendBundle,

    /// Canonical bundle hash computed via `SpendBundle::name()`.
    /// Used as the primary key in all mempool indexes.
    /// Deterministic across all nodes using Chia's Streamable serialization.
    pub spend_bundle_id: Bytes32,

    // ── Individual Cost & Fee ──
    /// Implicit fee: `sum(inputs) - sum(outputs)`.
    /// Computed by dig-clvm as `SpendResult.fee`.
    /// Source: `conditions.removal_amount - conditions.addition_amount`.
    /// Always >= 0 (conservation check enforced by dig-clvm).
    pub fee: u64,

    /// CLVM execution cost from `OwnedSpendBundleConditions.cost`.
    /// Equals `execution_cost + condition_cost` (computed by chia-consensus).
    /// Does NOT include byte cost (applied during block building, not mempool).
    /// Chia ref: `MempoolItem.cost` property at [mempool_item.py:84-85].
    ///
    /// [mempool_item.py:84-85]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L84
    pub cost: u64,

    /// Virtual cost: `cost + (num_spends * SPEND_PENALTY_COST)`.
    /// Penalizes transactions with many inputs to prevent spam.
    /// All fee-rate comparisons use virtual cost, not raw cost.
    /// Chia ref: `MempoolItem.virtual_cost` at [mempool_item.py:92-93].
    ///
    /// [mempool_item.py:92-93]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L92
    pub virtual_cost: u64,

    /// Fee-per-virtual-cost scaled by `FPC_SCALE` (10^12) for integer precision.
    /// Formula: `(fee as u128 * FPC_SCALE) / (virtual_cost as u128)`.
    /// Chia uses float division ([mempool_item.py:76-77]); we use integer math
    /// for determinism. Higher value = more attractive for block inclusion.
    ///
    /// [mempool_item.py:76-77]: https://github.com/Chia-Network/chia-blockchain/blob/6e7a4954edccd8ab83fcacf938cfc42ddfcad7f2/chia/types/mempool_item.py#L76
    pub fee_per_virtual_cost_scaled: u128,

    // ── Package Cost & Fee (CPFP) ──
    /// Sum of fees across this item and all its CPFP ancestors.
    /// For root items (no dependencies): `package_fee == fee`.
    /// For CPFP children: includes parent, grandparent, etc.
    /// Used by block selection to evaluate CPFP packages holistically.
    pub package_fee: u64,

    /// Sum of virtual costs across this item and all CPFP ancestors.
    /// For root items: `package_virtual_cost == virtual_cost`.
    pub package_virtual_cost: u64,

    /// Package fee-per-virtual-cost, scaled.
    /// `(package_fee * FPC_SCALE) / package_virtual_cost`.
    /// This is the metric used by CPFP-aware block selection strategies.
    pub package_fee_per_virtual_cost_scaled: u128,

    // ── Descendant Score (Eviction) ──
    /// Maximum of (own FPC, any descendant chain's package FPC).
    /// Protects low-fee parents with valuable CPFP children from eviction.
    /// Updated when children are added or removed from the dependency graph.
    /// Used as sort key during capacity eviction (lowest score evicted first).
    /// See: [CPF-006](../docs/requirements/domains/cpfp/specs/CPF-006.md)
    pub descendant_score: u128,

    // ── State Deltas ──
    /// Coins created by this bundle (from CREATE_COIN conditions).
    /// Each coin: `Coin { parent_coin_info, puzzle_hash, amount }`.
    /// Registered in `mempool_coins` index for CPFP child resolution.
    /// Source: `SpendResult.additions` from dig-clvm.
    pub additions: Vec<Coin>,

    /// Coin IDs consumed (spent) by this bundle.
    /// Registered in `coin_index` for conflict detection.
    /// Source: `SpendResult.removals` mapped through `Coin::coin_id()`.
    pub removals: Vec<Bytes32>,

    // ── Metadata ──
    /// L2 block height when this item was admitted to the mempool.
    /// Used as tiebreaker in sort orders for deterministic selection.
    pub height_added: u64,

    /// Full parsed CLVM conditions from `SpendResult.conditions`.
    /// Contains per-spend data: CREATE_COIN, timelocks, AGG_SIG, flags.
    /// The mempool reads fields like `reserve_fee`, `removal_amount`,
    /// `addition_amount`, and per-spend `flags` (ELIGIBLE_FOR_DEDUP/FF).
    /// Type from chia-consensus (via dig-clvm re-export).
    pub conditions: OwnedSpendBundleConditions,

    /// Number of coin spends in the bundle.
    /// `spend_bundle.coin_spends.len()`.
    /// Used in virtual cost computation and spend-count limits.
    pub num_spends: usize,

    // ── Timelocks ──
    // Resolved from relative to absolute during admission (Section 5.6).
    // `None` means no timelock of that type is present.
    /// Earliest height at which this bundle is valid.
    /// Resolved from per-spend `height_relative` + `height_absolute`.
    /// Items with `assert_height > current_height` are stored in pending pool.
    pub assert_height: Option<u64>,

    /// Latest height at which this bundle is valid (expiry).
    /// Resolved from per-spend `before_height_relative` + `before_height_absolute`.
    /// Items past expiry are removed during `on_new_block()`.
    /// Protected from eviction within `expiry_protection_blocks` of expiry.
    pub assert_before_height: Option<u64>,

    /// Latest timestamp at which this bundle is valid (expiry).
    /// Resolved from per-spend `before_seconds_relative` + `before_seconds_absolute`.
    pub assert_before_seconds: Option<u64>,

    // ── Dependencies (CPFP) ──
    /// Bundle IDs this item directly depends on (spends coins they created).
    /// Empty for items spending only on-chain confirmed coins.
    /// Used by cascade eviction: removing a parent removes all dependents.
    /// See: [CPF-002](../docs/requirements/domains/cpfp/specs/CPF-002.md)
    pub depends_on: HashSet<Bytes32>,

    /// Depth in the dependency chain.
    /// 0 = no dependencies, 1 = spends a mempool coin, etc.
    /// Max enforced by `config.max_dependency_depth` (default 25).
    pub depth: u32,

    // ── Optimization Flags ──
    /// Whether all spends in this bundle are eligible for identical-spend dedup.
    /// Read from `OwnedSpendConditions.flags & ELIGIBLE_FOR_DEDUP` (0x1).
    /// Set by chia-consensus's `MempoolVisitor` during CLVM execution.
    /// The mempool reads this flag; it does NOT compute it.
    pub eligible_for_dedup: bool,

    /// Singleton lineage info for fast-forward optimization, if applicable.
    /// Set when `OwnedSpendConditions.flags & ELIGIBLE_FOR_FF` (0x4) is present
    /// and the caller confirms FF eligibility via `supports_fast_forward()`.
    /// `None` for non-singleton spends.
    pub singleton_lineage: Option<SingletonLineageInfo>,
}

impl MempoolItem {
    /// Compute scaled fee-per-virtual-cost using integer arithmetic.
    ///
    /// Returns `(fee * FPC_SCALE) / virtual_cost`, or 0 if virtual_cost is 0.
    /// This avoids floating-point for cross-node determinism.
    ///
    /// Chia equivalent: `fee / virtual_cost` (float, mempool_item.py:76-77).
    pub fn compute_fpc_scaled(fee: u64, virtual_cost: u64) -> u128 {
        if virtual_cost == 0 {
            return 0;
        }
        (fee as u128 * FPC_SCALE) / (virtual_cost as u128)
    }

    /// Compute virtual cost from raw CLVM cost and spend count.
    ///
    /// `virtual_cost = cost + (num_spends * SPEND_PENALTY_COST)`
    ///
    /// The spend penalty discourages many-input transactions that are
    /// cheap in CLVM cost but expensive in validation/bandwidth.
    ///
    /// Chia ref: `mempool_item.py:92-93`
    pub fn compute_virtual_cost(cost: u64, num_spends: usize) -> u64 {
        cost + (num_spends as u64 * SPEND_PENALTY_COST)
    }

    /// Create a minimal MempoolItem for unit testing.
    ///
    /// Constructs an item with the given fee, cost, and spend count,
    /// using empty/default values for all other fields. No CPFP dependencies.
    /// Package fields equal individual fields (root item behavior).
    ///
    /// # Warning
    ///
    /// This is for tests only. Production code should construct items through
    /// the admission pipeline which populates all fields from dig-clvm output.
    pub fn new_for_test(fee: u64, cost: u64, num_spends: usize) -> Self {
        let virtual_cost = Self::compute_virtual_cost(cost, num_spends);
        let fpc_scaled = Self::compute_fpc_scaled(fee, virtual_cost);

        Self {
            spend_bundle: SpendBundle::new(vec![], dig_clvm::Signature::default()),
            spend_bundle_id: Bytes32::default(),
            fee,
            cost,
            virtual_cost,
            fee_per_virtual_cost_scaled: fpc_scaled,
            package_fee: fee,
            package_virtual_cost: virtual_cost,
            package_fee_per_virtual_cost_scaled: fpc_scaled,
            descendant_score: fpc_scaled,
            additions: vec![],
            removals: vec![],
            height_added: 0,
            conditions: OwnedSpendBundleConditions {
                spends: vec![],
                reserve_fee: 0,
                height_absolute: 0,
                seconds_absolute: 0,
                before_height_absolute: None,
                before_seconds_absolute: None,
                agg_sig_unsafe: vec![],
                cost: 0,
                removal_amount: 0,
                addition_amount: 0,
                validated_signature: false,
                execution_cost: 0,
                condition_cost: 0,
            },
            num_spends,
            assert_height: None,
            assert_before_height: None,
            assert_before_seconds: None,
            depends_on: HashSet::new(),
            depth: 0,
            eligible_for_dedup: false,
            singleton_lineage: None,
        }
    }
}
