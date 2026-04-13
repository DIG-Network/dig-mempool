# dt-authoritative-sources — Source Authority Hierarchy

## Authoritative Sources Table

| Source | Location | Purpose |
|--------|----------|---------|
| SPEC | `docs/resources/SPEC.md` | Master specification — all requirements derive from this |
| NORMATIVE | `domains/{domain}/NORMATIVE.md` | Authoritative requirement statements (MUST/SHOULD/MAY) |
| Dedicated Spec | `domains/{domain}/specs/PREFIX-NNN.md` | Detailed specification per requirement + test plan |
| VERIFICATION | `domains/{domain}/VERIFICATION.md` | QA approach and status |
| TRACKING | `domains/{domain}/TRACKING.yaml` | Machine-readable status |
| IMPLEMENTATION_ORDER | `requirements/IMPLEMENTATION_ORDER.md` | Phased checklist (61 requirements) |
| SCHEMA | `requirements/SCHEMA.md` | Data model and conventions |
| REGISTRY | `requirements/REQUIREMENTS_REGISTRY.yaml` | Domain registry |
| Chia L1 Reference | `github.com/Chia-Network/chia-blockchain` | L1 behavior reference (links in SPEC.md) |

All paths above are relative to `docs/`.

## Traceability Chain

```
IMPLEMENTATION_ORDER  (pick next [ ] item)
        |
        v
   NORMATIVE.md       (read the authoritative requirement statement)
        |
        v
   specs/PREFIX-NNN   (read the detailed specification + TEST PLAN)
        |
        v
   TOOLS              (SocratiCode search, Repomix pack, GitNexus impact)
        |
        v
   WRITE FAILING TEST (TDD — test defines the contract)
        |
        v
   implement          (make the test pass — chia crates first)
        |
        v
   VERIFICATION.md    (update QA status)
   TRACKING.yaml      (update machine-readable status)
   IMPLEMENTATION_ORDER (check off [x])
```

## Authority Order

When sources conflict, the higher-ranked source wins:

1. **NORMATIVE.md** — the requirement statement is authoritative
2. **Dedicated spec** (`specs/PREFIX-NNN.md`) — elaborates the requirement
3. **SPEC.md** — the master specification provides original context
4. **Chia L1 source** — reference implementation for behavior questions
5. **Existing code** — lowest authority; may need to be corrected

If NORMATIVE says MUST and a dedicated spec says SHOULD, NORMATIVE wins.
If SPEC.md and NORMATIVE disagree, flag the conflict and ask before proceeding.

## Source Citations

Every dedicated spec contains a `Source Citations` section linking back to SPEC.md sections and/or Chia L1 source files. Follow these links to verify understanding. The `Test Plan` section tells you exactly what tests to write — use it.

---

Navigation: Prev < [dt-hard-rules.md](dt-hard-rules.md) | Next > [dt-tools.md](dt-tools.md)
