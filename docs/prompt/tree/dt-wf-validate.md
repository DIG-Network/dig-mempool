# dt-wf-validate — Workflow: Validate

Run the full validation suite before committing. All checks must pass.

## Required Checks

```bash
# All tests pass (including the new VV test)
cargo test

# No clippy warnings (treated as errors)
cargo clippy -- -D warnings

# Formatting is clean
cargo fmt --check
```

## Targeted Checks

```bash
# Run the specific VV test
cargo test vv_req_adm_001

# Run with output visible
cargo test vv_req_adm_001 -- --nocapture
```

## Tool Checks

### No circular dependencies

```
codebase_graph_circular {}
```

### Change scope verification

```
gitnexus_detect_changes({scope: "staged"})
```

Verify changes only affect expected files and symbols.

## Critical Audit Checks

### No custom CLVM execution

```bash
grep -r "run_program\|run_spendbundle\|run_block_generator" src/
# Only dig_clvm::validate_spend_bundle() should appear
```

### No IO imports

```bash
grep -rE "std::fs|std::net|tokio|async fn|reqwest|sqlx" src/
# Must find nothing
```

### No opcode redefinition

```bash
grep -rE "CREATE_COIN\s*=|AGG_SIG.*=.*1_?200" src/
# Must find nothing — use chia_consensus::opcodes::*
```

## Failure Handling

- **Test failure:** Fix the implementation to match the spec, not the test. The spec is authoritative. If the spec is wrong, flag it.
- **Clippy warning:** Fix it. No `#[allow(...)]` without justification.
- **Format failure:** Run `cargo fmt` and include formatting in the commit.
- **Circular dependency:** Restructure to break the cycle.
- **Unexpected change scope:** Investigate — did you accidentally modify unrelated code?

## All Checks Passed

When all checks are green, proceed to tracking updates.

---

Navigation: Prev < [dt-wf-implement.md](dt-wf-implement.md) | Next > [dt-wf-update-tracking.md](dt-wf-update-tracking.md)
