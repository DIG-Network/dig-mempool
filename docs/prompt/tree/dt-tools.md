# dt-tools — Tool Integration

Three tools are deeply integrated into the dig-mempool workflow. Each has a HARD RULE governing when it MUST be used. **All three tools MUST be used before writing any implementation code.**

## SocratiCode — Semantic Codebase Intelligence

**HARD RULE:** MUST use before reading files. Search first, read targeted.

| Command | Purpose |
|---------|---------|
| `codebase_status {}` | Check index status |
| `codebase_search { query: "..." }` | Hybrid semantic + keyword search |
| `codebase_graph_query { filePath: "..." }` | Show imports and dependents |
| `codebase_graph_circular {}` | Detect circular dependencies |

### When to Use
- **Before reading any file** — search finds the right code
- **Before implementing** — find related code and patterns
- **After implementing** — check for circular deps

Full docs: [../tools/socraticode.md](../tools/socraticode.md)

---

## GitNexus — Knowledge Graph Dependency Analysis

**HARD RULE:** MUST use before refactoring or renaming public symbols. MUST update after commits.

| Command | Purpose |
|---------|---------|
| `npx gitnexus status` | Check if index is fresh |
| `npx gitnexus analyze` | Incremental index update |
| `gitnexus_impact { symbol: "..." }` | What depends on this symbol? |
| `gitnexus_detect_changes` | What changed since last analyze? |

### When to Use
- **Start of session** — `npx gitnexus status`
- **Before modifying public symbols** — `gitnexus_impact`
- **Before committing** — `gitnexus_detect_changes`
- **After committing** — `npx gitnexus analyze`

Full docs: [../tools/gitnexus.md](../tools/gitnexus.md)

---

## Repomix — Context Packing for LLM Consumption

**HARD RULE:** MUST pack context before starting implementation.

```bash
npx repomix@latest src -o .repomix/pack-src.xml
npx repomix@latest tests -o .repomix/pack-tests.xml
npx repomix@latest docs/requirements/domains/<domain> -o .repomix/pack-<domain>-reqs.xml
```

### When to Use
- **Before implementing** — pack the scope you're about to modify
- **Before testing** — pack tests for pattern reference
- **Cross-requirement work** — pack multiple domains

Full docs: [../tools/repomix.md](../tools/repomix.md)

---

## Integration Matrix

| Workflow Step | SocratiCode | GitNexus | Repomix |
|--------------|-------------|----------|---------|
| Select requirement | `codebase_search` for existing impl | -- | -- |
| Gather context | `codebase_search` + `graph_query` | -- | Pack scope |
| Write test | Search for test patterns | -- | Pack tests |
| Implement | Search before coding | Impact check before refactoring | Pack impl scope |
| Validate | `graph_circular` | `detect_changes` | -- |
| Commit | -- | `npx gitnexus analyze` | -- |

**The gather context step is NOT optional.** If you skip tool usage, you will miss dependencies, write redundant code, or break existing functionality.

---

Navigation: Prev < [dt-authoritative-sources.md](dt-authoritative-sources.md) | Next > [dt-git.md](dt-git.md)
