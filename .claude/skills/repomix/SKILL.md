# Repomix — Context Packing Skill

## When to Use

Use Repomix **before implementing any requirement**. Pack the relevant scope so the LLM has full awareness of the code being modified.

## HARD RULE

**MUST pack context before writing implementation code.** Fresh context prevents redundant work and missed patterns.

## Commands

### Pack Implementation

```bash
npx repomix@latest src -o .repomix/pack-src.xml
```

### Pack Tests (CRITICAL for TDD)

```bash
npx repomix@latest tests -o .repomix/pack-tests.xml
```

### Pack Requirements by Domain

```bash
# Admission domain
npx repomix@latest docs/requirements/domains/admission -o .repomix/pack-adm-reqs.xml

# Conflict resolution
npx repomix@latest docs/requirements/domains/conflict_resolution -o .repomix/pack-cfr-reqs.xml

# CPFP
npx repomix@latest docs/requirements/domains/cpfp -o .repomix/pack-cpf-reqs.xml

# Selection
npx repomix@latest docs/requirements/domains/selection -o .repomix/pack-sel-reqs.xml

# Pools
npx repomix@latest docs/requirements/domains/pools -o .repomix/pack-pol-reqs.xml

# Fee estimation
npx repomix@latest docs/requirements/domains/fee_estimation -o .repomix/pack-fee-reqs.xml

# Lifecycle
npx repomix@latest docs/requirements/domains/lifecycle -o .repomix/pack-lcy-reqs.xml

# Crate API
npx repomix@latest docs/requirements/domains/crate_api -o .repomix/pack-api-reqs.xml

# All requirements at once
npx repomix@latest docs/requirements -o .repomix/pack-requirements.xml
```

### Pack the Full Spec

```bash
npx repomix@latest docs/resources -o .repomix/pack-spec.xml
```

### Pack with Compression

```bash
npx repomix@latest src --compress -o .repomix/pack-src-compressed.xml
```

### Pack Multiple Scopes

```bash
npx repomix@latest src tests -o .repomix/pack-impl-and-tests.xml
```

## Workflow Integration

| Step | Pack Command |
|------|-------------|
| Before writing tests | `npx repomix@latest tests -o .repomix/pack-tests.xml` |
| Before implementing | `npx repomix@latest src -o .repomix/pack-src.xml` |
| Cross-domain work | Pack both domains' requirements |

## Notes

- `.repomix/` is gitignored — pack files are never committed
- Regenerate packs when switching requirements
- Use `--compress` for large scopes to manage token count
- Pack requirements alongside code for spec compliance checks

## Full Documentation

See `docs/prompt/tools/repomix.md` for complete reference.
