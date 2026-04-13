# dt-wf-implement — Workflow: Implement Against Spec

**You should already have a failing test from dt-wf-test.** Your goal now is to make that test pass with the minimal correct implementation.

## Step 0: Verify Failing Test

Before writing any implementation code, confirm the test fails:

```bash
cargo test vv_req_prefix_nnn
```

If the test passes, stop — either the requirement is already implemented or the test is wrong.

## Step 1: SocratiCode + GitNexus Checks

Before writing implementation code:

```
codebase_search { query: "function or type being implemented" }
codebase_graph_query { filePath: "file to modify" }
```

If modifying existing code:
```
gitnexus_impact({target: "symbol", direction: "upstream"})
```

## Step 2: Use Chia Crates First

Check crates in this order before writing custom code:

| Priority | Crate | Provides |
|----------|-------|----------|
| 1 | `dig-clvm` | `validate_spend_bundle()`, `SpendResult`, `ValidationContext` |
| 2 | `chia-consensus` | `OwnedSpendBundleConditions`, opcodes, flags, `check_time_locks()` |
| 3 | `chia-protocol` | `SpendBundle::name()`, `Coin::coin_id()`, `FeeRate` |
| 4 | `chia-sdk-types` | `Condition<T>`, `announcement_id()` |
| 5 | `chia-sdk-driver` | `SingletonLayer`, `supports_fast_forward()` |
| 6 | `chia-sdk-coinset` | `CoinRecord` |
| 7 | `chia-bls` | `BlsCache` |

Only write custom logic when no upstream crate provides the needed functionality.

## Step 3: Smallest Change Principle

- **Match the spec exactly.** Implement what the dedicated spec says, nothing more.
- **Make the failing test pass.** That is the only goal.
- **No features beyond the requirement.** If ADM-001 says "submit entry point", build the entry point. Do not add caching, logging, or metrics.
- **No speculative abstractions.** No traits "for future use." No generic parameters unless the spec requires them.

## Step 4: Module Placement

| Domain | Directory | Files |
|--------|-----------|-------|
| API types | `src/` | `item.rs`, `error.rs`, `lib.rs` |
| Admission | `src/admission/` | `pipeline.rs`, `timelock.rs` |
| Conflict | `src/conflict/` | `detection.rs`, `rbf.rs` |
| CPFP | `src/cpfp/` | `dependency.rs`, `cascade.rs` |
| Selection | `src/selection/` | `strategies.rs`, `ordering.rs` |
| Pools | `src/pools/` | `active.rs`, `pending.rs`, `conflict.rs`, `seen.rs` |
| Fee | `src/fee/` | `estimation.rs`, `tracker.rs` |
| Lifecycle | `src/lifecycle/` | `block.rs`, `hooks.rs`, `persistence.rs` |

## Implementation Checklist

Before moving to validation, verify:

- [ ] The failing test from dt-wf-test now PASSES
- [ ] Code matches the spec's acceptance criteria
- [ ] Uses chia crate functions where available (Rule 1)
- [ ] No custom CLVM execution (Rule 2)
- [ ] No opcode redefinition (Rule 4)
- [ ] No block building (Rule 5)
- [ ] Re-exports upstream types (Rule 6)
- [ ] New public API is re-exported in `src/lib.rs`

---

Navigation: Prev < [dt-wf-test.md](dt-wf-test.md) | Next > [dt-wf-validate.md](dt-wf-validate.md)
