# dt-paths — Path Conventions

## Project Layout

```
dig-mempool/
├── docs/
│   ├── resources/
│   │   └── SPEC.md                          # Master specification (2200+ lines)
│   ├── requirements/
│   │   ├── SCHEMA.md                        # Data model and conventions
│   │   ├── README.md                        # Requirements system overview
│   │   ├── REQUIREMENTS_REGISTRY.yaml       # Domain registry
│   │   ├── IMPLEMENTATION_ORDER.md          # Phased checklist (61 requirements)
│   │   └── domains/
│   │       ├── admission/                   # ADM-* Admission pipeline
│   │       ├── conflict_resolution/         # CFR-* Conflict detection + RBF
│   │       ├── cpfp/                        # CPF-* CPFP dependency chains
│   │       ├── selection/                   # SEL-* Block candidate selection
│   │       ├── pools/                       # POL-* Pool management + eviction
│   │       ├── fee_estimation/              # FEE-* Fee tracker + estimation
│   │       ├── lifecycle/                   # LCY-* Block events, hooks, persistence
│   │       └── crate_api/                   # API-* Public types, config, errors
│   └── prompt/                              # This workflow system
│       ├── prompt.md
│       ├── start.md
│       ├── tree/                            # Decision tree files (you are here)
│       └── tools/                           # Tool documentation
├── src/
│   ├── lib.rs                               # Public API re-exports
│   ├── mempool.rs                           # Mempool struct, submit, select, on_new_block
│   ├── item.rs                              # MempoolItem, MempoolConfig, constants
│   ├── error.rs                             # MempoolError enum
│   ├── pools/                               # Active, pending, conflict, seen pools
│   ├── admission/                           # Two-phase admission pipeline
│   ├── conflict/                            # Conflict detection, RBF
│   ├── cpfp/                                # Dependency graph, cascade eviction
│   ├── selection/                           # Multi-strategy greedy selection
│   ├── fee/                                 # Fee estimation + tracker
│   └── lifecycle/                           # Block events, hooks, persistence
├── tests/
│   └── vv_req_{prefix}_{nnn}.rs             # Per-requirement TDD tests
├── Cargo.toml
└── .repomix/                                # Ephemeral context packs (gitignored)
```

## Sibling Crates

```
../dig-constants/                            # Network parameters (separate crate)
../dig-clvm/                                 # CLVM validation engine (runtime dependency)
```

## Key Paths to Remember

| Artifact | Path |
|----------|------|
| Master spec | `docs/resources/SPEC.md` |
| Implementation order | `docs/requirements/IMPLEMENTATION_ORDER.md` |
| Domain requirements | `docs/requirements/domains/{domain}/NORMATIVE.md` |
| Requirement spec | `docs/requirements/domains/{domain}/specs/PREFIX-NNN.md` |
| Main entry | `src/lib.rs` |
| Core logic | `src/*.rs` + `src/*/` |
| Tests | `tests/vv_req_*.rs` |

---

Navigation: Next > [dt-role.md](dt-role.md)
