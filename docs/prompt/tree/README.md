# Decision Tree Index

This directory contains the decision tree files for the dig-mempool prompt system.
Each file is a single decision node. **Traverse them in order during any work session. Do not skip steps.**

## Traversal Order

1. **[dt-paths.md](dt-paths.md)** — Path conventions and project layout
2. **[dt-role.md](dt-role.md)** — Role definition and key competencies
3. **[dt-hard-rules.md](dt-hard-rules.md)** — 16 non-negotiable rules
4. **[dt-authoritative-sources.md](dt-authoritative-sources.md)** — Source authority hierarchy
5. **[dt-tools.md](dt-tools.md)** — SocratiCode, GitNexus, Repomix integration
6. **[dt-git.md](dt-git.md)** — Git workflow and commit conventions
7. **[dt-wf-select.md](dt-wf-select.md)** — Workflow: select next requirement
8. **[dt-wf-gather-context.md](dt-wf-gather-context.md)** — Workflow: gather context with all three tools
9. **[dt-wf-test.md](dt-wf-test.md)** — Workflow: TDD — write failing tests FIRST
10. **[dt-wf-implement.md](dt-wf-implement.md)** — Workflow: implement against spec (make tests pass)
11. **[dt-wf-validate.md](dt-wf-validate.md)** — Workflow: validate with cargo and tools
12. **[dt-wf-update-tracking.md](dt-wf-update-tracking.md)** — Workflow: update tracking artifacts
13. **[dt-wf-commit.md](dt-wf-commit.md)** — Workflow: commit, push, loop

## How to Use

- **First session:** Read dt-paths through dt-git to internalize the environment.
- **Each requirement:** Follow dt-wf-select through dt-wf-commit **in strict order**. Do not skip any step.
- **Loop:** After dt-wf-commit, return to dt-wf-select for the next requirement.
- **The test step (dt-wf-test) comes BEFORE the implement step (dt-wf-implement).** This is TDD. Write the failing test first. Then make it pass.
