# dt-git — Git Workflow

## Sync Before Work

```bash
git fetch origin && git pull origin main
```

Always sync before selecting work. Treat `[x]` items as done after pull.

## Commit Format

```
type(scope): imperative subject
```

### Types

| Type | When |
|------|------|
| `feat` | New functionality (implementing a requirement) |
| `fix` | Bug fix |
| `docs` | Documentation only (tracking updates, spec corrections) |
| `chore` | Build, deps, tooling |
| `refactor` | Code restructuring without behavior change |
| `test` | Test-only changes |

### Scopes

| Scope | Maps to |
|-------|---------|
| `admission` | Admission pipeline (ADM-*) |
| `conflict` | Conflict detection + RBF (CFR-*) |
| `cpfp` | CPFP dependencies (CPF-*) |
| `selection` | Block candidate selection (SEL-*) |
| `pools` | Pool management (POL-*) |
| `fee` | Fee estimation (FEE-*) |
| `lifecycle` | Lifecycle events (LCY-*) |
| `api` | Crate API types (API-*) |
| `deps` | Cargo.toml dependency changes |

### Examples

```
feat(api): implement API-001 Mempool constructor
feat(admission): implement ADM-002 CLVM validation via dig-clvm
test(conflict): add CFR-001 conflict detection tests
docs(admission): update TRACKING for ADM-001 through ADM-004
fix(cpfp): correct package fee calculation in CPF-005
```

## Push

```bash
git push origin main
```

Always push after commit.

---

Navigation: Prev < [dt-tools.md](dt-tools.md) | Next > [dt-wf-select.md](dt-wf-select.md)
