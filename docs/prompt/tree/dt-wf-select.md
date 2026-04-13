# dt-wf-select — Workflow: Select Next Requirement

## Procedure

### Step 1: Sync

```bash
git pull origin main
```

### Step 2: Open IMPLEMENTATION_ORDER.md

```
docs/requirements/IMPLEMENTATION_ORDER.md
```

### Step 3: Choose the first unchecked item

- Scan from top to bottom within the current phase
- Choose the first `- [ ]` item
- **Skip every `[x]`** — those are done
- **Work phases in order** — complete Phase 0 before Phase 1, etc.

### Step 4: Read the requirement

1. Open `domains/{domain}/NORMATIVE.md` — find the `<a id="PREFIX-NNN">` anchor
2. Read the authoritative MUST/SHOULD statement
3. Open `domains/{domain}/specs/PREFIX-NNN.md` — read the FULL specification:
   - Summary, Specification, Acceptance Criteria
   - **Test Plan** — this tells you exactly what tests to write in the TDD step
   - Source Citations, References
4. Open VERIFICATION.md — understand the expected verification approach

### Step 5: Confirm selection

Before proceeding, verify:
- [ ] The requirement is `[ ]` (unchecked) in IMPLEMENTATION_ORDER
- [ ] You have read NORMATIVE.md for the requirement ID
- [ ] You have read the dedicated spec including the **Test Plan**
- [ ] You understand what MUST be implemented
- [ ] You know which tests to write (from the Test Plan section)

**Do NOT proceed to gather context until you have read the full spec.**

---

Navigation: Prev < [dt-git.md](dt-git.md) | Next > [dt-wf-gather-context.md](dt-wf-gather-context.md)
