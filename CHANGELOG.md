# Changelog

All notable changes to this project are documented here.
This project adheres to [Semantic Versioning](https://semver.org) and
[Conventional Commits](https://www.conventionalcommits.org).

## [0.1.2] - 2026-07-19

### Documentation
- Correct dig-mempool README and reserved error-variant docs (#2)

## [0.1.1] - 2026-07-12

### Testing
- **stats:** Cover MempoolStats::empty baseline snapshot

### CI
- Gate test job at >=80% line coverage via cargo-llvm-cov- Run the >=80% coverage gate on push to main + PRs- Enforce version increment in PRs (package.json / Cargo.toml)- Enforce Conventional Commits with commitlint on PRs- Enforce Conventional Commits with commitlint on PRs- Release automation (git-cliff changelog + tag on merge); publish is manual workflow_dispatch (#230)- Re-arm crates.io auto-publish on version tag (token in org secrets; auto-publish-everything #230)- Add flaky-test management (#489) (#1)

### Chores
- **changelog:** Add git-cliff config for Conventional-Commit changelog

## [0.1.0] - 2026-04-14

### Features
- **api:** Implement API-001 Mempool constructors- **api:** Implement API-002 MempoolItem struct with all fields- **api:** Implement API-003 MempoolConfig with builder pattern and defaults- **api:** Implement API-004 MempoolError enum verification- **api:** Implement API-005 SubmitResult enum- **api:** Implement API-006 MempoolStats struct verification- **api:** Implement API-007 extension traits- **api:** Implement API-008 query methods- **admission:** Implement ADM-001 submit() entry point signature- **admission:** Implement ADM-002 internal CLVM validation- **admission:** Implement ADM-003 seen-cache dedup before CLVM- **admission:** Implement ADM-004 fee extraction + RESERVE_FEE check- **admission:** Implement ADM-005 virtual cost computation- **admission:** Implement ADM-006 timelock resolution- **admission:** Implement ADM-007 dedup/FF flag extraction- **admission:** Implement ADM-008 submit_batch()- **pools:** Implement POL-001 active pool storage- **pools:** Implement POL-002 active pool capacity management- **pools:** Implement POL-003 expiry protection during eviction- **pools:** Implement POL-004 pending pool- **pools:** Implement POL-005 pending pool deduplication- **pools:** Implement POL-006 conflict cache- **pools:** Implement POL-007 seen cache multi-pool dedup and clear()- **pools:** Implement POL-008 identical spend dedup index- **pools:** Implement POL-010 concurrency verification- **conflict:** Implement CFR-001 through CFR-006 conflict detection and RBF- **cpfp:** Implement CPF-001 through CPF-008- **selection:** Implement SEL-001 through SEL-008 with per-requirement tests- **lifecycle:** Implement LCY-001 on_new_block() + LCY-002 RetryBundles- **lifecycle:** Implement LCY-003 caller workflow + fix seen-cache on_new_block- **lifecycle:** Implement LCY-004/005/006 clear(), hooks, RemovalReason- **lifecycle:** Implement LCY-008 evict_lowest_percent()- **fee:** Implement FEE-001 estimate_min_fee() 3-tier system- **fee:** Implement FEE-002/003/004 FeeTracker, estimate_fee_rate, record_confirmed_block- **fee:** Implement FEE-005 FeeEstimatorState serialization- **lifecycle:** Implement LCY-007 snapshot()/restore() persistence- **pools:** Implement POL-009 singleton tracking

### Refactor
- **tests:** Use hex_literal for puzzle hash constants- **tests:** Split combined CFR and CPF test files into per-requirement files- **arch:** Extract pool types and selection helpers into proper module structure

### Documentation
- Add prompt system, Claude skills, and tool integration- Pin all Chia L1 citations to commit 6e7a4954- Add comprehensive comments to all existing source files


