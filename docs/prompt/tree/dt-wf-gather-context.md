# dt-wf-gather-context — Workflow: Gather Context

**MUST use all three tools during context gathering.** This step ensures you have complete understanding before writing any code or tests. Do NOT skip this step.

## Step 0: SocratiCode Search First

Before reading any files, search for related code:

```
codebase_search { query: "requirement topic or key concept" }
```

Understand the dependency structure of relevant files:

```
codebase_graph_query { filePath: "src/relevant_file.rs" }
```

Search for related patterns:

```
codebase_context_search { query: "related concept or type name" }
```

## Step 1: Repomix Pack

Pack the scope you are about to work on:

```bash
# Pack implementation scope
npx repomix@latest src -o .repomix/pack-src.xml

# Pack tests for pattern reference (CRITICAL for TDD step)
npx repomix@latest tests -o .repomix/pack-tests.xml

# Pack the domain requirements
npx repomix@latest docs/requirements/domains/<domain> -o .repomix/pack-<domain>-reqs.xml
```

**Packing tests is especially important** — you need to match existing test patterns when writing your failing test in the next step.

## Step 2: Requirements Trace

Read the full requirements chain for the selected requirement:

1. **NORMATIVE.md** — Read `#{id}` section for the authoritative statement
2. **specs/{id}.md** — Read the detailed specification
3. **Test Plan section** — This tells you exactly what tests to write. Copy the test table.
4. **Source citations** — Follow links to SPEC.md sections and Chia L1 code
5. **References section** — Check related requirements in other domains
6. **TRACKING.yaml** — Current status (should be `gap`)

## Step 3: Cross-References and Related Code

- Check the `References` section in the dedicated spec for related requirement IDs
- Search for code that implements those related requirements:
  ```
  codebase_search { query: "related requirement function or type" }
  ```
- If modifying existing code, check impact:
  ```
  gitnexus_impact({target: "symbol", direction: "upstream"})
  ```

## Step 4: Existing Test Patterns

- Search for existing tests to match their style:
  ```
  codebase_search { query: "test pattern for similar requirement" }
  ```
- Understand the test infrastructure being used (Simulator, mock helpers, etc.)

## Verification Checklist

Before proceeding to the test step, confirm:
- [ ] SocratiCode search completed
- [ ] Repomix context packed (src + tests + domain requirements)
- [ ] Full spec read including Test Plan
- [ ] Cross-references checked
- [ ] Existing test patterns reviewed
- [ ] GitNexus impact checked (if modifying existing code)

**Do NOT proceed to dt-wf-test until all tools have been used.**

---

Navigation: Prev < [dt-wf-select.md](dt-wf-select.md) | Next > [dt-wf-test.md](dt-wf-test.md)
