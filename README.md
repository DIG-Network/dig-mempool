# dig-mempool

A fee-prioritized, conflict-aware transaction mempool for the DIG Network L2
blockchain.

`dig-mempool` accepts raw `SpendBundle` submissions, validates them internally
via [`dig-clvm`](../dig-clvm), and manages their lifecycle — fee ordering,
conflict detection and replace-by-fee, timelock resolution, child-pays-for-parent
(CPFP) packaging, and capacity eviction — then selects `MempoolItem`s for block
candidate production. It does **not** build or validate blocks; that is the
caller's responsibility.

**Boundary:** inputs are `SpendBundle` + `CoinRecord`s; output is
`Vec<Arc<MempoolItem>>`.

## Documentation

- [`SPEC.md`](docs/resources/SPEC.md) — the normative specification (wire model,
  admission pipeline, conflict/RBF rules, CPFP semantics, fee tiers, invariants).
- [`docs/requirements/README.md`](docs/requirements/README.md) — the requirement
  catalog (61 requirements across 8 domains).

## License

See the repository root for license terms.
