# dt-wf-test — Workflow: TDD — Write Failing Tests FIRST

**This is the most important step in the workflow.** Write the test before writing the implementation. The test defines the contract. If you cannot demonstrate a failing test, you do not understand the requirement well enough to implement it.

## HARD RULE: Test MUST fail before implementation exists

```
1. Write test based on spec's Test Plan section
2. Run test → MUST FAIL (compilation error or assertion failure)
3. Only then proceed to dt-wf-implement
4. Implementation makes the test pass
```

If the test passes without any implementation, either:
- The requirement is already implemented (check TRACKING.yaml)
- Your test is wrong (it's not actually testing the requirement)

## Test File Naming

```
tests/vv_req_{prefix}_{nnn}.rs
```

Examples:
- ADM-001 --> `tests/vv_req_adm_001.rs`
- CFR-003 --> `tests/vv_req_cfr_003.rs`
- CPF-005 --> `tests/vv_req_cpf_005.rs`
- SEL-001 --> `tests/vv_req_sel_001.rs`

## File Structure

```rust
//! REQUIREMENT: ADM-001 — submit() Entry Point
//!
//! Test-driven verification of the mempool submission entry point.
//! Tests written BEFORE implementation per TDD workflow.

use dig_mempool::{Mempool, MempoolConfig, SubmitResult, MempoolError};
use dig_clvm::{SpendBundle, Bytes32, CoinRecord};
use dig_constants::DIG_TESTNET;
use std::collections::HashMap;

#[test]
fn vv_req_adm_001_valid_bundle_returns_success() {
    // Arrange: create mempool, build valid spend bundle with coin records
    // Act: call mempool.submit(bundle, &coin_records, height, timestamp)
    // Assert: returns Ok(SubmitResult::Success)
}

#[test]
fn vv_req_adm_001_returns_error_on_invalid_signature() {
    // Arrange: create mempool, build bundle with bad signature
    // Act: call mempool.submit(...)
    // Assert: returns Err(MempoolError::ValidationError(...))
}
```

## Where to Find Test Cases

**Every requirement spec has a Test Plan section.** This is your test blueprint.

Open `docs/requirements/domains/{domain}/specs/PREFIX-NNN.md` and find:

```markdown
## Verification

### Test Plan

| Test | Type | Description | Expected Result |
|------|------|-------------|-----------------|
| test_name_1 | Unit | What it tests | Expected outcome |
| test_name_2 | Integration | What it tests | Expected outcome |
...
```

**Implement every row in the Test Plan table as a test function.** Each row = one `#[test]` function.

## Required Test Types

### Integration Tests (MUST for every requirement)

Full mempool lifecycle:
- Create `Mempool` with `DIG_TESTNET` constants
- Build `SpendBundle`s using `chia-sdk-test::Simulator` and `SpendContext`
- Submit via the public API
- Assert on results, errors, and state changes

### Unit Tests

Individual function behavior:
- Input/output correctness
- Error path coverage
- Boundary conditions

### Permutation Matrix

Cover all dimensions for each requirement:

| Dimension | Examples |
|-----------|----------|
| Valid inputs | Correct spend, proper signature, within cost/fee |
| Invalid inputs | Bad signature, missing coin, cost exceeded, double spend |
| Edge cases | Zero fee, max u64 amount, empty bundle, single coin |
| Concurrency | Concurrent submits, submit during select_for_block |
| State transitions | Submit → on_new_block → resubmit → select |

## Running Tests

```bash
# Run the specific VV test for the requirement
cargo test vv_req_adm_001

# Run with output visible
cargo test vv_req_adm_001 -- --nocapture

# Run all tests
cargo test
```

## When the Test Fails (Expected)

The test should fail because the function/type doesn't exist yet, or returns a default/wrong value. This is correct TDD behavior:

- **Compilation error** — the function signature doesn't exist → implement the signature stub
- **Assertion failure** — the function exists but returns wrong result → implement the logic
- **Panic** — unimplemented!() macro or todo!() → replace with real implementation

## When to Skip Test-First

Only skip TDD for:
- Documentation-only changes (tracking updates, spec corrections)
- Pure configuration changes (Cargo.toml, constants)
- Tracking file updates

For **everything else**: test first, then implement.

---

Navigation: Prev < [dt-wf-gather-context.md](dt-wf-gather-context.md) | Next > [dt-wf-implement.md](dt-wf-implement.md)
