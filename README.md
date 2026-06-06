# bca — Behavioral Code Analyzer

> Rust-based modernization of Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat).
> Mines git history to produce hotspots, change coupling, ownership topology, and code-health metrics.

**Status: alpha (Plan 1 walking skeleton).** Architecture validated end-to-end; feature parity with code-maat lands across Plans 2–6.

## Quick start

```bash
# Build from source
cargo build --release -p bca-cli

# Run the walking skeleton against any git repo
./target/release/bca analyze --analysis revisions --repo . --rows 10 --min-revs 1
```

Output:
```
entity,n-revs
src/main.rs,42
src/lib.rs,38
...
```

## What works today

`bca analyze --analysis revisions --format csv` end-to-end:
- Walks git history via `gix 0.84` (libgit2-free, pure Rust)
- Stores commits + file-changes in an in-memory DuckDB fact store
- Runs the `revisions` SQL view (file → distinct commit count)
- Emits code-maat-compatible CSV (`entity,n-revs` header)

20 tests pass across the workspace (17 library + 3 CLI integration).

## Architecture

3-crate workspace:
- `bca-lib` — types, Repo trait, gix-backed GixRepo, DuckDB-backed FactsDb, revisions analysis, CSV emitter
- `bca-cli` — clap CLI, single `analyze` subcommand
- `bca-rca` — Mozilla rust-code-analysis vendor (Plan 2)

Key design choices (see [`docs/superpowers/specs/2026-06-06-bca-design.md`](docs/superpowers/specs/2026-06-06-bca-design.md)):
- **gix** (gitoxide) over libgit2 — pure Rust, no LGPL question, `+Send` works correctly
- **DuckDB** as embedded fact store — SQL surface as power-user feature, spill-to-disk for scale
- **Event-sourced pipeline** — `Stream<CommitEvent>` + projections, Salsa-retrofit-able
- **Behavioral SARIF** as the differentiator (Plan 5) — no other tool emits SARIF for organizational signals
- **Provenance manifest** with every output (Plan 5) — addresses Spadoni 2025's 500% inter-tool disagreement problem

## Roadmap

This walking skeleton (Plan 1 of 6) proves the spine. Subsequent plans:
- **Plan 2** — vendor Mozilla's `rust-code-analysis` as `bca-rca/`, add Go support
- **Plan 3** — complexity metrics integration; hotspot ranking; Code Health composite
- **Plan 4** — 9 other code-maat analyses + Fisher exact significance + identity resolution
- **Plan 5** — SARIF + Markdown + Parquet + SQLite outputs + provenance manifest
- **Plan 6** — differential test harness + performance benchmarks + release infrastructure

Phase 0 deliverable target: full v1.0 in ~10 weeks of focused work.

## License

GPL-3.0-only. Will include a vendored fork of Mozilla's `rust-code-analysis` under MPL-2.0 starting Plan 2 — see `crates/bca-rca/LICENSE-MPL` (when it lands).
