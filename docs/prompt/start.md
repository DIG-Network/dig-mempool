# Start

## Immediate Actions

1. **Sync**
   ```bash
   git fetch origin && git pull origin main
   ```

2. **Check tools — ALL THREE MUST BE FRESH**
   ```bash
   npx gitnexus status          # GitNexus index fresh?
   npx gitnexus analyze         # Update if stale
   # SocratiCode: verify Docker running, index current
   codebase_status {}            # SocratiCode MCP status
   ```
   **Do not proceed until tools are confirmed operational.** Coding without tools leads to redundant work and missed dependencies.

3. **Pick work** — open `docs/requirements/IMPLEMENTATION_ORDER.md`
   - Choose the first `- [ ]` item
   - Every `- [x]` is done on main — skip it
   - Work phases in order: Phase 0 before Phase 1, etc.

4. **Pack context — BEFORE reading any code**
   ```bash
   npx repomix@latest src -o .repomix/pack-src.xml
   npx repomix@latest tests -o .repomix/pack-tests.xml
   ```

5. **Search with SocratiCode — BEFORE reading files**
   ```
   codebase_search { query: "submit spend bundle mempool" }
   codebase_graph_query { filePath: "src/mempool.rs" }
   ```

6. **Read spec** — follow the full trace:
   - `NORMATIVE.md#PREFIX-NNN` → authoritative requirement
   - `specs/PREFIX-NNN.md` → detailed specification + **test plan**
   - `VERIFICATION.md` → how to verify
   - `TRACKING.yaml` → current status

7. **Continue** → [dt-wf-select.md](tree/dt-wf-select.md)

---

## Hard Requirements

1. **Use chia crate ecosystem first** — never reimplement what `chia-consensus`, `chia-sdk-types`, `chia-sdk-driver`, `chia-bls`, `chia-protocol`, `chia-puzzles` provide.
2. **No custom CLVM execution** — delegate to `dig_clvm::validate_spend_bundle()`.
3. **No custom condition parsing** — read `OwnedSpendBundleConditions` from dig-clvm output.
4. **No opcode redefinition** — use `chia_consensus::opcodes::*` directly.
5. **No block building** — this crate selects items; the caller builds blocks.
6. **Re-export, don't redefine** — `Coin`, `CoinSpend`, `SpendBundle`, `Bytes32` from upstream via dig-clvm.
7. **TEST FIRST (TDD)** — write the failing test before writing implementation code. The test defines the contract. The spec's Test Plan section tells you exactly what tests to write.
8. **One requirement per commit** — don't batch unrelated work.
9. **Update tracking after each requirement** — VERIFICATION.md, TRACKING.yaml, IMPLEMENTATION_ORDER.md.
10. **SocratiCode before file reads** — search semantically first, read targeted files second.
11. **Repomix before implementation** — pack relevant scope for full context.
12. **GitNexus before refactoring** — check dependency impact before renaming or moving symbols.
13. **Follow the decision tree to completion** — dt-wf-select through dt-wf-commit, no shortcuts.
14. **`coin_records` is minimal** — only coins being spent, never the full UTXO set.

---

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
| Puzzle bytecodes | `chia-puzzles` | 0.20 |
| Error handling | `thiserror` | 2 |
| Serialization | `serde` | latest |
| Testing | `chia-sdk-test` | 0.30 |
