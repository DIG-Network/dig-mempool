# dig-mempool Requirements

This directory contains the formal requirements for the dig-mempool crate,
following the same two-tier requirements structure as dig-clvm
with full traceability.

## Quick Links

- [SCHEMA.md](SCHEMA.md) — Data model and conventions
- [REQUIREMENTS_REGISTRY.yaml](REQUIREMENTS_REGISTRY.yaml) — Central domain registry
- [domains/](domains/) — All requirement domains

## Structure

```
requirements/
├── README.md                    # This file
├── SCHEMA.md                    # Data model and conventions
├── REQUIREMENTS_REGISTRY.yaml   # Central registry
├── IMPLEMENTATION_ORDER.md      # Phased implementation checklist
└── domains/
    ├── admission/               # ADM-* Submission validation pipeline
    ├── conflict_resolution/     # CFR-* Conflict detection and RBF
    ├── cpfp/                    # CPF-* CPFP dependency chains
    ├── selection/               # SEL-* Block candidate selection
    ├── pools/                   # POL-* Pool management, eviction, capacity
    ├── fee_estimation/          # FEE-* Fee tracker and estimation
    ├── lifecycle/               # LCY-* Block events, reorg, hooks, persistence
    └── crate_api/               # API-* Public types, config, errors
```

## Three-Document Pattern

Each domain contains:

| File | Purpose |
|------|---------|
| `NORMATIVE.md` | Authoritative requirement statements (MUST/SHOULD/MAY) |
| `VERIFICATION.md` | QA approach and status per requirement |
| `TRACKING.yaml` | Machine-readable status, tests, and notes |

## Specification Files

Individual requirement specifications are in each domain's `specs/` subdirectory:

```
domains/
├── admission/specs/               # ADM-001.md through ADM-008.md
├── conflict_resolution/specs/     # CFR-001.md through CFR-NNN.md
├── cpfp/specs/                    # CPF-001.md through CPF-NNN.md
├── selection/specs/               # SEL-001.md through SEL-NNN.md
├── pools/specs/                   # POL-001.md through POL-NNN.md
├── fee_estimation/specs/          # FEE-001.md through FEE-NNN.md
├── lifecycle/specs/               # LCY-001.md through LCY-NNN.md
└── crate_api/specs/               # API-001.md through API-NNN.md
```

## Reference Document

All requirements are derived from:
- [SPEC.md](../resources/SPEC.md) — dig-mempool specification

## Requirement Count

| Domain | Prefix | Count |
|--------|--------|-------|
| Admission Pipeline | ADM | 8 |
| Conflict Resolution | CFR | TBD |
| CPFP Dependencies | CPF | TBD |
| Block Selection | SEL | TBD |
| Pool Management | POL | TBD |
| Fee Estimation | FEE | TBD |
| Lifecycle | LCY | TBD |
| Crate API | API | TBD |
| **Total** | | **8+** |
