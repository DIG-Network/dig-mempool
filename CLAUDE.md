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

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **dig-mempool** (2834 symbols, 6868 relationships, 239 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## When Debugging

1. `gitnexus_query({query: "<error or symptom>"})` — find execution flows related to the issue
2. `gitnexus_context({name: "<suspect function>"})` — see all callers, callees, and process participation
3. `READ gitnexus://repo/dig-mempool/process/{processName}` — trace the full execution flow step by step
4. For regressions: `gitnexus_detect_changes({scope: "compare", base_ref: "main"})` — see what your branch changed

## When Refactoring

- **Renaming**: MUST use `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` first. Review the preview — graph edits are safe, text_search edits need manual review. Then run with `dry_run: false`.
- **Extracting/Splitting**: MUST run `gitnexus_context({name: "target"})` to see all incoming/outgoing refs, then `gitnexus_impact({target: "target", direction: "upstream"})` to find all external callers before moving code.
- After any refactor: run `gitnexus_detect_changes({scope: "all"})` to verify only expected files changed.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Tools Quick Reference

| Tool | When to use | Command |
|------|-------------|---------|
| `query` | Find code by concept | `gitnexus_query({query: "auth validation"})` |
| `context` | 360-degree view of one symbol | `gitnexus_context({name: "validateUser"})` |
| `impact` | Blast radius before editing | `gitnexus_impact({target: "X", direction: "upstream"})` |
| `detect_changes` | Pre-commit scope check | `gitnexus_detect_changes({scope: "staged"})` |
| `rename` | Safe multi-file rename | `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` |
| `cypher` | Custom graph queries | `gitnexus_cypher({query: "MATCH ..."})` |

## Impact Risk Levels

| Depth | Meaning | Action |
|-------|---------|--------|
| d=1 | WILL BREAK — direct callers/importers | MUST update these |
| d=2 | LIKELY AFFECTED — indirect deps | Should test |
| d=3 | MAY NEED TESTING — transitive | Test if critical path |

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/dig-mempool/context` | Codebase overview, check index freshness |
| `gitnexus://repo/dig-mempool/clusters` | All functional areas |
| `gitnexus://repo/dig-mempool/processes` | All execution flows |
| `gitnexus://repo/dig-mempool/process/{name}` | Step-by-step execution trace |

## Self-Check Before Finishing

Before completing any code modification task, verify:
1. `gitnexus_impact` was run for all modified symbols
2. No HIGH/CRITICAL risk warnings were ignored
3. `gitnexus_detect_changes()` confirms changes match expected scope
4. All d=1 (WILL BREAK) dependents were updated

## Keeping the Index Fresh

After committing code changes, the GitNexus index becomes stale. Re-run analyze to update it:

```bash
npx gitnexus analyze
```

If the index previously included embeddings, preserve them by adding `--embeddings`:

```bash
npx gitnexus analyze --embeddings
```

To check whether embeddings exist, inspect `.gitnexus/meta.json` — the `stats.embeddings` field shows the count (0 means no embeddings). **Running analyze without `--embeddings` will delete any previously generated embeddings.**

> Claude Code users: A PostToolUse hook handles this automatically after `git commit` and `git merge`.

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
