# dig-mempool — Project Context

## What This Is

`dig-mempool` is a standalone Rust crate that implements a fee-prioritized, conflict-aware transaction mempool for the DIG Network L2 blockchain. It accepts raw `SpendBundle` submissions, validates them internally via `dig-clvm`, manages their lifecycle (ordering, conflicts, timelocks, CPFP, eviction), and outputs selected `MempoolItem`s for block candidate production. It does NOT perform block building or block validation.

**Hard boundary:** Inputs = `SpendBundle` + `CoinRecord`s. Outputs = `Vec<Arc<MempoolItem>>`.

## Key Documents

| Document | Path | Purpose |
|----------|------|---------|
| Master Spec | `docs/resources/SPEC.md` | Complete specification (2200+ lines) |
| Requirements | `docs/requirements/README.md` | 61 requirements across 8 domains |
| Implementation Order | `docs/requirements/IMPLEMENTATION_ORDER.md` | Phased checklist |
| Tool Docs | `docs/prompt/tools/README.md` | SocratiCode, GitNexus, Repomix |
| Network Constants | `../dig-constants/src/lib.rs` | DIG_MAINNET / DIG_TESTNET |
| CLVM Engine | `../dig-clvm/src/consensus/validate.rs` | `validate_spend_bundle()` |

## Hard Rules

1. **Use chia crate ecosystem first** — never reimplement what `chia-consensus`, `chia-sdk-types`, `chia-sdk-driver`, `chia-bls`, `chia-protocol`, `chia-puzzles` provide.
2. **No custom CLVM execution** — delegate to `dig_clvm::validate_spend_bundle()`.
3. **No custom condition parsing** — read `OwnedSpendBundleConditions` fields from dig-clvm's output.
4. **No opcode constant redefinition** — use `chia_consensus::opcodes::*` directly.
5. **No block building** — this crate selects items; the caller builds blocks via `dig-clvm`.
6. **Re-export, don't redefine** — `Coin`, `CoinSpend`, `SpendBundle`, `Bytes32`, `CoinRecord` from upstream via dig-clvm.
7. **Tests MUST be test-driven** — write failing test first, then implement.
8. **One requirement per commit** — don't batch unrelated work.
9. **Update tracking after each requirement** — VERIFICATION.md, TRACKING.yaml, IMPLEMENTATION_ORDER.md.

---

## Tool Usage — MANDATORY ON EVERY PROMPT

### SocratiCode — Search before reading

**ALWAYS use `codebase_search` before reading files.** Find the right files first, then read targeted sections.

```
codebase_search { query: "conflict detection RBF" }
codebase_graph_query { filePath: "src/mempool.rs" }
```

If the SocratiCode MCP server is not connected, fall back to Grep/Glob but note the degraded workflow.

### GitNexus — Impact analysis before editing

**ALWAYS run impact analysis before modifying any public symbol.**

```bash
npx gitnexus status          # Check index freshness
npx gitnexus analyze         # Update if stale
```

```
gitnexus_impact({target: "Mempool::submit", direction: "upstream"})
gitnexus_detect_changes({scope: "staged"})
```

**After every commit:** `npx gitnexus analyze` to keep the index current.

### Repomix — Pack context before implementing

**ALWAYS pack relevant scope before starting implementation work.**

```bash
npx repomix@latest src -o .repomix/pack-src.xml
npx repomix@latest tests -o .repomix/pack-tests.xml
npx repomix@latest docs/requirements/domains/admission -o .repomix/pack-adm-reqs.xml
```

---

## Workflow Cycle

| Step | Action | Tool |
|------|--------|------|
| 0 | Sync repo, check tool freshness | `git pull`, `npx gitnexus status`, `codebase_status {}` |
| 1 | Pick next `- [ ]` from `IMPLEMENTATION_ORDER.md` | — |
| 2 | Pack context | Repomix |
| 3 | Search for related code | SocratiCode |
| 4 | Read requirement spec | `docs/requirements/domains/{domain}/specs/{ID}.md` |
| 5 | Write failing test | TDD |
| 6 | Implement against spec | Use chia crates first |
| 7 | Run tests, clippy, fmt | `cargo test`, `cargo clippy`, `cargo fmt` |
| 8 | Check impact | `gitnexus_detect_changes` |
| 9 | Update tracking | TRACKING.yaml, VERIFICATION.md, IMPLEMENTATION_ORDER.md |
| 10 | Commit + update index | `git commit`, `npx gitnexus analyze` |

---

## Architecture

```
src/
  lib.rs              — Public API re-exports
  mempool.rs          — Mempool struct, submit(), select_for_block(), on_new_block()
  item.rs             — MempoolItem, MempoolConfig, constants
  error.rs            — MempoolError enum
  pools/
    active.rs         — Active pool (items, coin_index, mempool_coins)
    pending.rs        — Pending pool (timelocked items)
    conflict.rs       — Conflict cache (failed RBF bundles)
    seen.rs           — Seen-cache (LRU dedup)
  admission/
    pipeline.rs       — Two-phase admission pipeline
    timelock.rs       — Timelock resolution
    dedup.rs          — Dedup/FF flag extraction
  conflict/
    detection.rs      — Coin conflict detection
    rbf.rs            — Replace-by-fee rules
  cpfp/
    dependency.rs     — Dependency graph, package fees
    cascade.rs        — Cascade eviction
    announcements.rs  — Cross-bundle announcement validation
  selection/
    strategies.rs     — 4-way greedy selection
    ordering.rs       — Topological sort, FPC ordering
  fee/
    estimation.rs     — estimate_min_fee(), estimate_fee_rate()
    tracker.rs        — FeeTracker (bucket-based)
  lifecycle/
    block.rs          — on_new_block(), clear()
    hooks.rs          — MempoolEventHook, RemovalReason
    persistence.rs    — snapshot(), restore()
```

## Requirement Domains

| Domain | Prefix | Count | Directory |
|--------|--------|-------|-----------|
| Admission Pipeline | ADM | 8 | `domains/admission/` |
| Conflict Resolution | CFR | 6 | `domains/conflict_resolution/` |
| CPFP Dependencies | CPF | 8 | `domains/cpfp/` |
| Block Candidate Selection | SEL | 8 | `domains/selection/` |
| Pool Management | POL | 10 | `domains/pools/` |
| Fee Estimation | FEE | 5 | `domains/fee_estimation/` |
| Lifecycle Events | LCY | 8 | `domains/lifecycle/` |
| Crate API | API | 8 | `domains/crate_api/` |
| **Total** | | **61** | |

## Tech Stack

| Component | Crate | Version |
|-----------|-------|---------|
| CLVM validation | `dig-clvm` | 0.1.0 |
| Network constants | `dig-constants` | 0.1.0 |
| Consensus engine | `chia-consensus` | 0.26 |
| Protocol types | `chia-protocol` | 0.26 |
| BLS signatures | `chia-bls` | 0.26 |
| Condition types | `chia-sdk-types` | 0.30 |
| Singleton driver | `chia-sdk-driver` | 0.30 |
| Coin state | `chia-sdk-coinset` | 0.30 |
| Puzzle hashes | `chia-puzzles` | 0.20 |
| Error handling | `thiserror` | 2 |
| Serialization | `serde` | latest |
| Testing | `chia-sdk-test` | 0.30 |
