# CodeLore

> **Read the lore of your codebase.**
> Every commit is a fragment of tribal knowledge: who wrote the code, who *understands* it, where the scars are, and which corners hide secrets nobody's read in years. CodeLore reads those fragments from git history and turns them into hotspots, change-coupling, ownership maps, and code-health scores — the socio-technical signal your linter can't see.

A Rust-based modernization of Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat). Powered by [gix](https://github.com/GitoxideLabs/gitoxide), [DuckDB](https://duckdb.org), and a vendored fork of Mozilla's [rust-code-analysis](https://github.com/mozilla/rust-code-analysis).

**Status: alpha (Plans 1–5 complete, Plan 6 in progress).** 11 analyses × 6 output formats (CSV, JSON, **SARIF 2.1.0**, Markdown, Parquet, SQLite). Provenance manifest sidecars ship with every file output. Plan 6 (differential testing, perf benchmarks, release infra) is the final gate before v1.0 — currently shipping `GitCliRepo` differential oracle + 50-commit fixture.

## Quick start

```bash
# Build from source
cargo build --release -p codelore-cli

# Run the walking skeleton against any git repo
./target/release/codelore analyze --analysis revisions --repo . --rows 10 --min-revs 1
```

Output:
```
entity,n-revs
src/main.rs,42
src/lib.rs,38
...
```

## What works today

End-to-end pipeline: gix walk → DuckDB fact store → analysis → 6 output formats.

```bash
# CSV (code-maat parity, the default)
codelore analyze --analysis hotspots --repo . --min-revs 5

# JSON (structured, machine-readable)
codelore analyze --analysis hotspots --repo . --format json

# SARIF 2.1.0 — drop into GitHub Code Scanning, GitLab, Defectdojo
codelore analyze --analysis hotspots --repo . --format sarif --output hotspots.sarif

# Markdown — pipe directly into $GITHUB_STEP_SUMMARY
codelore analyze --analysis hotspots --repo . --format markdown >> "$GITHUB_STEP_SUMMARY"

# Parquet — for downstream analytics in DuckDB, Polars, pandas, Spark
codelore analyze --analysis hotspots --repo . --format parquet --output hotspots.parquet

# SQLite — full 7-table fact-store dump (commits, changes, hunks, entities,
# complexity_metrics, author_aliases, provenance) for ad-hoc SQL exploration
codelore analyze --analysis revisions --repo . --format sqlite --output facts.db
```

Every file output gets a `{output}.provenance.json` sidecar (except SQLite, where the `provenance` table lives inside the DB). The sidecar records the codelore/gix/duckdb versions, every threshold knob, and the UTC run timestamp — addresses Spadoni 2025's 500% inter-tool disagreement problem.

310 tests pass across the workspace (199 RCA unit + 6 Tier-1 smoke + 105 codelore-lib/codelore-cli Plan 1–6 suite), 1 ignored (P6.T03 — documented `GitCliRepo` parser bug).
- Per-language complexity metrics (Cyclomatic, Cognitive, Halstead, MI) for Rust, TypeScript/JavaScript, Python, Java via vendored `codelore-rca/` (Mozilla rust-code-analysis fork)
- `codelore analyze --analysis NAME --format csv` for 11 analyses:
  - `revisions` — file → commit count
  - `hotspots` — published §1.1 formula (percentile_rank(revs) × percentile_rank(cognitive) × (10 − code_health) / 10)
  - `code-health` — full §4.6 composite (cognitive, churn, fragmentation, coupling)
  - `code-age` — months since last modification
  - `abs-churn`, `author-churn`, `entity-churn` — three churn views
  - `communication` — Conway's law shared-work author pairs
  - `code-ownership` — Fractal Value (1−HHI) + main developer
  - `change-coupling` — Fisher exact-filtered logical coupling
  - `summary` — 4-row repo overview
- `.mailmap` resolution + bot filtering + AI attribution stub
- Kamei 14-feature change vector populated per commit
- Function-level entity extraction at HEAD for Tier-1 languages

## Architecture

3-crate workspace:
- `codelore-lib` — types, Repo trait, gix-backed GixRepo, DuckDB-backed FactsDb, revisions analysis, CSV emitter
- `codelore-cli` — clap CLI, single `analyze` subcommand
- `codelore-rca` — Mozilla rust-code-analysis vendor (Plan 2)

Key design choices (see [`docs/superpowers/specs/2026-06-06-codelore-design.md`](docs/superpowers/specs/2026-06-06-codelore-design.md)):
- **gix** (gitoxide) over libgit2 — pure Rust, no LGPL question, `+Send` works correctly
- **DuckDB** as embedded fact store — SQL surface as power-user feature, spill-to-disk for scale
- **Event-sourced pipeline** — `Stream<CommitEvent>` + projections, Salsa-retrofit-able
- **Behavioral SARIF** as the differentiator (Plan 5) — no other tool emits SARIF for organizational signals
- **Provenance manifest** with every output (Plan 5) — addresses Spadoni 2025's 500% inter-tool disagreement problem

## Roadmap

Plans 1–5 complete. Plan 6 in progress (final plan before v1.0):
- **Plan 1** ✅ — Phase 0 walking skeleton (workspace, gix walker, DuckDB fact store, revisions analysis)
- **Plan 2** ✅ — vendored Mozilla's `rust-code-analysis` as `codelore-rca/`, Tier-1 metric smoke tests
- **Plan 3** ✅ — complexity integration via codelore-rca, hotspot ranking (published §1.1 formula), Code Health composite
- **Plan 4** ✅ — 8 new analyses + Kamei vector + identity resolution + full Code Health composite
- **Plan 5** ✅ — SARIF 2.1.0 + JSON + Markdown + Parquet + SQLite outputs + provenance manifest sidecar
- **Plan 6** 🚧 — differential test harness (`GitCliRepo` ≡ `GixRepo`), code-maat golden parity tests, criterion perf benchmarks (Linux kernel scale), release infrastructure (`cargo-dist`, SLSA L3, distroless container, PGO)

Phase 0 deliverable target: full v1.0 in ~10 weeks of focused work.

## Why "CodeLore"?

Behavioral code analysis isn't about lines of code — it's about the *lore*: who wrote the code, who understands it now, what secrets are hidden in the commits, and which corners of the codebase carry tribal knowledge nobody's documented. CodeLore surfaces that lore as hotspots, ownership maps, change-coupling, and code-health composites. The technical category is "behavioral code analysis"; the metaphor is reading the legends a codebase tells about itself.

## License

GPL-3.0-only. Bundles a vendored fork of Mozilla's `rust-code-analysis` under MPL-2.0 — see `crates/codelore-rca/LICENSE-MPL`.
