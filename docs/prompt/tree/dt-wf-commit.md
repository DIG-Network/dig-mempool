# dt-wf-commit — Workflow: Commit, Push, Loop

One requirement per commit. Include code, tests, and tracking updates together.

## Procedure

### Step 1: Update GitNexus

```bash
npx gitnexus analyze
```

### Step 2: Stage Files

Stage exactly the files for this requirement:

```bash
git add src/admission/pipeline.rs \
        tests/vv_req_adm_001.rs \
        docs/requirements/domains/admission/TRACKING.yaml \
        docs/requirements/domains/admission/VERIFICATION.md \
        docs/requirements/IMPLEMENTATION_ORDER.md
```

**Include:** Implementation + tests + tracking.
**Exclude:** Unrelated changes, `.repomix/` files.

### Step 3: Commit

```bash
git commit -m "feat(admission): implement ADM-001 submit() entry point"
```

### Step 4: Push

```bash
git push origin main
```

### Step 5: Update GitNexus Index

```bash
npx gitnexus analyze
```

## What to Avoid

- **Mixing requirement IDs** — one commit = one requirement
- **Incomplete TDD cycle** — test MUST exist and pass before commit
- **Missing tracking updates** — code + tests + tracking = one atomic unit
- **Committing `.repomix/` files** — gitignored

## Loop — RETURN TO THE BEGINNING

**The decision tree cycle is complete for this requirement. Start the next one.**

**Next requirement --> [dt-wf-select.md](dt-wf-select.md)**

Follow the full cycle again:
1. Select requirement from IMPLEMENTATION_ORDER
2. Gather context with all three tools
3. Write failing test (TDD)
4. Implement to make test pass
5. Validate (cargo test + clippy + fmt)
6. Update tracking artifacts
7. Commit and push

**Do not skip any step. Do not batch multiple requirements. Complete the full decision tree for every single requirement.**

---

Navigation: Prev < [dt-wf-update-tracking.md](dt-wf-update-tracking.md) | Loop > [dt-wf-select.md](dt-wf-select.md)
