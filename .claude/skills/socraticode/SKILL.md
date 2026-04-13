# SocratiCode — Semantic Codebase Intelligence Skill

## When to Use

Use SocratiCode **before reading any file**. Search semantically first, then read only the 1-3 files that matter.

## HARD RULE

**MUST search before reading files.** `codebase_search` finds the right files; you read targeted sections. Never `cat` or `head` entire directories.

## Setup

SocratiCode is configured as an MCP server in `.claude/settings.json`:

```json
{
  "mcpServers": {
    "socraticode": {
      "command": "npx",
      "args": ["-y", "socraticode"]
    }
  }
}
```

Prerequisites: Docker running, Node.js 18+.

## Commands

### Search (Primary)

```
codebase_search { query: "conflict detection RBF replace by fee" }
```

Hybrid semantic + keyword search across all indexed files.

### Dependency Graph

```
codebase_graph_query { filePath: "src/mempool.rs" }
```

Shows imports and dependents for a specific file.

### Circular Dependency Detection

```
codebase_graph_circular {}
```

Run after implementing to verify no circular deps were introduced.

### Context Search

```
codebase_context_search { query: "MempoolConfig capacity eviction" }
```

Searches schemas, APIs, and configuration patterns.

### Index Management

```
codebase_status {}    # Check if index is current
codebase_index {}     # Full reindex
codebase_update {}    # Incremental update
```

## Workflow Integration

| Step | SocratiCode Command |
|------|---------------------|
| Select requirement | `codebase_search { query: "existing implementation of X" }` |
| Gather context | `codebase_search` + `codebase_graph_query` |
| Before reading files | ALWAYS `codebase_search` first |
| Find test patterns | `codebase_search { query: "test submit mempool" }` |
| After implementing | `codebase_graph_circular {}` |

## Example Queries for dig-mempool

```
codebase_search { query: "submit spend bundle to mempool" }
codebase_search { query: "conflict detection coin index" }
codebase_search { query: "CPFP dependency chain package fee" }
codebase_search { query: "block candidate selection greedy" }
codebase_search { query: "eviction descendant score" }
codebase_graph_query { filePath: "src/conflict/rbf.rs" }
```

## Full Documentation

See `docs/prompt/tools/socraticode.md` for complete reference.
