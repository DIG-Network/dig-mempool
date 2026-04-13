# dt-hard-rules — 16 Non-Negotiable Rules

Every rule below is a hard constraint. Violating any one of them is a blocking defect.

## Rule 1: Use chia crate ecosystem first

Before writing ANY custom code, check these crates:
- `dig-clvm` — `validate_spend_bundle()`, `SpendResult`, `ValidationContext`, `ValidationConfig`
- `chia-consensus` — `OwnedSpendBundleConditions`, opcodes, flags (`MEMPOOL_MODE`, `ELIGIBLE_FOR_DEDUP`, `ELIGIBLE_FOR_FF`)
- `chia-protocol` — `SpendBundle::name()`, `Coin::coin_id()`, `FeeRate`, `FeeEstimate`
- `chia-sdk-types` — `Condition<T>`, `announcement_id()`
- `chia-sdk-driver` — `SingletonLayer::parse_puzzle()`, `supports_fast_forward()`
- `chia-sdk-coinset` — `CoinRecord`
- `chia-bls` — `BlsCache`

Only write custom logic when no upstream crate provides the needed functionality.

## Rule 2: No custom CLVM execution

Delegate to `dig_clvm::validate_spend_bundle()`. The mempool calls it during admission. It never calls `run_spendbundle()` or `run_program()` directly.

## Rule 3: No custom condition parsing

Read condition data from `OwnedSpendBundleConditions` returned by dig-clvm. Never manually parse CLVM output.

## Rule 4: No opcode redefinition

Use `chia_consensus::opcodes::*` for all condition opcodes and cost constants. Do not define `CREATE_COIN = 51` or `AGG_SIG_COST = 1_200_000` in mempool code.

## Rule 5: No block building

This crate selects mempool items for block inclusion. It does NOT compress generators, aggregate signatures, or rebase singleton lineage proofs. That is the caller's job.

## Rule 6: Re-export, don't redefine

`Coin`, `CoinSpend`, `SpendBundle`, `Bytes32`, `CoinRecord`, `Program` come from upstream via dig-clvm re-exports. Never create your own versions of these types.

## Rule 7: TEST FIRST (TDD) — mandatory

Write the failing test BEFORE writing implementation code. The test defines the contract. Each requirement's spec has a Test Plan section — use it.

```
1. Read spec → 2. Write test → 3. Run test (MUST FAIL) → 4. Implement → 5. Run test (MUST PASS)
```

Skipping the test-first step is a blocking defect. If you cannot demonstrate a failing test before implementation, stop and create one.

## Rule 8: One requirement per commit

Each commit implements exactly one requirement ID. No batching, no partial implementations.

## Rule 9: Update tracking after each requirement

After implementing a requirement, update ALL THREE:
- `TRACKING.yaml` — status, tests, notes
- `VERIFICATION.md` — status column, verification approach
- `IMPLEMENTATION_ORDER.md` — check off the `[ ]`

## Rule 10: SocratiCode search before file reads

Always use `codebase_search` before reading files. Search finds the right files; you read targeted sections. Never blindly read entire directories.

## Rule 11: Repomix pack before implementation

Before writing implementation code, pack the relevant scope:
```bash
npx repomix@latest <scope> -o .repomix/pack-<scope>.xml
```

## Rule 12: GitNexus impact check before refactoring

Before renaming symbols or restructuring modules:
```bash
npx gitnexus analyze
gitnexus_impact({target: "symbol", direction: "upstream"})
```

## Rule 13: Follow the decision tree to completion

The workflow cycle (dt-wf-select through dt-wf-commit) MUST be followed in strict order for every requirement. No shortcuts. No skipping steps. If you are tempted to skip the test step or the tool step, stop — that is a process violation.

## Rule 14: `coin_records` is minimal

The `coin_records` input contains only the coins being spent in the current bundle. It is not a full UTXO set. The caller provides exactly the records needed.

## Rule 15: No async/IO/storage in core logic

The mempool is a pure in-memory data structure. No `async`, no `tokio`, no `std::fs`, no `std::net`. Persistence is via `snapshot()` / `restore()` — the caller handles I/O.

## Rule 16: Tools MUST be used before writing code

Do not write a single line of implementation code until you have:
1. Searched with SocratiCode (`codebase_search`)
2. Packed context with Repomix (`npx repomix@latest`)
3. Checked impact with GitNexus (if modifying existing code)

This is not optional. This is not "nice to have." This prevents redundant work and missed dependencies.

## Post-Pull Rule

After `git pull`: treat `[x]` items in IMPLEMENTATION_ORDER.md as done. Only `[ ]` items are selectable for work. Never re-implement a checked item.

---

Navigation: Prev < [dt-role.md](dt-role.md) | Next > [dt-authoritative-sources.md](dt-authoritative-sources.md)
