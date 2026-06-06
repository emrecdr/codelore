# Changelog

Conventional Commits format. All notable changes documented here.

## [Unreleased]

### Added (Plan 1: Phase 0 + Walking Skeleton)
- 3-crate Cargo workspace (`bca-lib`, `bca-cli`, future `bca-rca`)
- Core types: `CommitEvent`, `FileChange`, `Hunk`, `ChangeType`, `KameiFeatures`
- `AnalysisName` enum and `Options` struct with code-maat parity defaults
- `arrow_facade` module — single re-export point for `arrow-rs`
- `Repo` trait + `GixRepo` impl (read .git via gix 0.84)
- Fixture builder (`test_support::tiny_repo`) for reproducible 5-commit test repos
- `FactsDb` — DuckDB-backed fact store with v1 schema (7 tables)
- Commit ingestion pipeline (gix → crossbeam channel → DuckDB Appender)
- `revisions` analysis (SQL view + Rust orchestrator)
- CSV output emitter (code-maat header parity)
- `bca analyze --analysis revisions --format csv` CLI
- GitHub Actions CI (fmt, clippy, test on 3 OSes, cargo-deny)
- Justfile, deny.toml, renovate.json, rust-toolchain.toml

### Pending (subsequent plans)
- Plan 2: RCA vendor (Mozilla rust-code-analysis fork) + Go support
- Plan 3: complexity integration + hotspots + Code Health composite
- Plan 4: 9 other analyses + Fisher significance + identity resolution
- Plan 5: SARIF + Markdown + Parquet + SQLite + provenance manifest
- Plan 6: differential testing harness + perf benchmarks + release infra
