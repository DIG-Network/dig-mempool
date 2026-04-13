# Tool Documentation Index

Three tools are integrated into the dig-mempool development workflow. Each serves a distinct purpose and has mandatory usage points.

## Tool Summary

| Tool | Purpose | Install | Primary Commands |
|------|---------|---------|-----------------|
| [SocratiCode](socraticode.md) | Semantic search, dependency graphs | MCP plugin or `npx` | `codebase_search`, `codebase_graph_query` |
| [GitNexus](gitnexus.md) | Knowledge graph, impact analysis | `npx gitnexus` | `analyze`, `status`, `impact` |
| [Repomix](repomix.md) | Context packing for LLMs | `npx repomix@latest` | `<scope> -o .repomix/pack.xml` |

## When to Use Each Tool

### Before reading files --> SocratiCode

Search first, read targeted. SocratiCode's hybrid semantic + keyword search finds the right files. You then read only the 1-3 files that matter, not entire directories.

```
codebase_search { query: "conflict detection coin_index" }
```

### Before refactoring --> GitNexus

Check dependency impact before renaming symbols or restructuring modules. GitNexus precomputes every dependency chain so you know what will break.

```bash
npx gitnexus analyze
# then via MCP: gitnexus_impact { symbol: "Mempool" }
```

### Before implementing --> Repomix

Pack the codebase scope you are about to modify. Feed the packed context to the LLM for complete awareness before writing code.

```bash
npx repomix@latest src -o .repomix/pack-src.xml
```

## Integration with Workflow

See [../tree/dt-tools.md](../tree/dt-tools.md) for the full integration matrix showing which tool to use at each workflow step.
