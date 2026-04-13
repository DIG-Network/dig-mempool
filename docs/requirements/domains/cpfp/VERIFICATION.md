# Child-Pays-For-Parent (CPFP) — Verification

| ID | Status | Summary | Verification Approach |
|----|--------|---------|----------------------|
| [CPF-001](NORMATIVE.md#CPF-001) | ⚠️ | mempool_coins index | Additions registered on insert; entries removed on eviction. |
| [CPF-002](NORMATIVE.md#CPF-002) | ⚠️ | Dependency resolution | mempool_coins lookup for unresolved coins; dependency edge recorded. |
| [CPF-003](NORMATIVE.md#CPF-003) | ⚠️ | Maximum dependency depth | Depth computed from ancestor chain; DependencyTooDeep on overflow. |
| [CPF-004](NORMATIVE.md#CPF-004) | ⚠️ | Defensive cycle detection | Graph cycle check after edge insertion; DependencyCycle error. |
| [CPF-005](NORMATIVE.md#CPF-005) | ⚠️ | Package fee rate computation | Aggregate fee/cost across ancestor chain; scaled integer arithmetic. |
| [CPF-006](NORMATIVE.md#CPF-006) | ⚠️ | Descendant score tracking | Max of own FPC and descendant package FPC; updated on add/remove. |
| [CPF-007](NORMATIVE.md#CPF-007) | ⚠️ | Cascade eviction | Recursive dependent removal; full index cleanup. |
| [CPF-008](NORMATIVE.md#CPF-008) | ⚠️ | Cross-bundle announcement validation | Assertion conditions checked against ancestor conditions. |

**Status legend:** ✅ verified · ⚠️ partial · ❌ gap
