# dt-wf-update-tracking — Workflow: Update Tracking Artifacts

After validation passes, update ALL THREE tracking artifacts for the completed requirement.

## 1. TRACKING.yaml

File: `docs/requirements/domains/{domain}/TRACKING.yaml`

```yaml
- id: PREFIX-NNN
  section: "Section Name"
  summary: "Brief title"
  status: verified          # was: gap
  spec_ref: "docs/requirements/domains/{domain}/specs/PREFIX-NNN.md"
  tests:
    - vv_req_prefix_nnn     # test file name
  notes: "Brief description of implementation"
```

### Status Values

| Status | Meaning |
|--------|---------|
| `gap` | Not started |
| `partial` | Some work done |
| `implemented` | Code written, tests pass |
| `verified` | Tests pass AND clippy/fmt clean |

## 2. VERIFICATION.md

File: `docs/requirements/domains/{domain}/VERIFICATION.md`

Update the row:

```markdown
| PREFIX-NNN | ✅ | Brief summary | Tests: vv_req_prefix_nnn. Verified via TDD. |
```

## 3. IMPLEMENTATION_ORDER.md

File: `docs/requirements/IMPLEMENTATION_ORDER.md`

```markdown
# Before
- [ ] PREFIX-NNN — Description

# After
- [x] PREFIX-NNN — Description
```

## Checklist

- [ ] TRACKING.yaml updated (status, tests, notes)
- [ ] VERIFICATION.md row updated (status, approach)
- [ ] IMPLEMENTATION_ORDER.md checkbox changed `[ ]` to `[x]`
- [ ] No other requirement accidentally modified

---

Navigation: Prev < [dt-wf-validate.md](dt-wf-validate.md) | Next > [dt-wf-commit.md](dt-wf-commit.md)
