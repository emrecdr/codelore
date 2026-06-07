# CodeLore — Behavioral Code Analyzer

> **Read the lore of your codebase.**

Every codebase tells a story that static linters cannot see. Behind the syntax and structure lies a human narrative: who wrote the code, who *understands* it now, which corners hide secrets nobody's read in years, and where the historical scars are buried. Every commit is a piece of this tribal lore.

**CodeLore** mines your repository's git history and projects it into behavioral insight — hotspots, change-coupling, knowledge fragmentation, code-health scores, live code clones — the socio-technical signal your linter cannot see.

A Rust-based modernization of Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat). Powered by [gix](https://github.com/GitoxideLabs/gitoxide), [DuckDB](https://duckdb.org), [tree-sitter](https://tree-sitter.github.io/), and a vendored fork of Mozilla's [rust-code-analysis](https://github.com/mozilla/rust-code-analysis).

**Status: alpha (Plans 1–7 complete; Plan 8 in flight, §1–§4 + §6 core shipped).** 13 analyses × 6 output formats (CSV, JSON, **SARIF 2.1.0**, Markdown, Parquet, SQLite). Provenance manifest sidecars on every file output. Persistent fact-store cache with LRU eviction. Live-clone detection (clones × Fisher-significant co-change) — the strategic differentiator. 349 tests pass, clippy/fmt/deny clean.

---

## 1. Why CodeLore?

Standard static analyzers (SonarQube, ESLint, Clippy) look at code at a single point in time. They can tell you if code is poorly formatted or has too many branches — but they cannot tell you:

- **Knowledge Loss Risk** — which complex hotspot was written by an author who left the company two months ago?
- **Hidden Architectural Debt** — which files are implicitly coupled, *always* modified together, but live in different subsystems?
- **Refactoring ROI** — which highly complex files are actively changing (worth refactoring) vs. stable (leave alone)?
- **Live Clones** — which copy-pasted blocks keep being edited in lockstep (real debt) vs. which are dead patterns nobody touches (noise)?

CodeLore focuses on the **socio-technical dimension** of software engineering. It reads the legends a codebase tells about itself to help you focus refactoring effort where it pays off most.

---

## 2. Key Differentiators

What separates CodeLore from code-maat, CodeScene, jscpd, and the rest of the field:

### 2.1 Behavioral SARIF (the CI differentiator)
CodeLore emits its organizational findings — hotspots, live clones, ownership risks — as **SARIF 2.1.0**. Drop them straight into GitHub Code Scanning, GitLab security dashboards, or Defectdojo and the alerts appear inline on pull requests right next to the diff. No other behavioral-analysis tool ships SARIF natively. Plan 5 published rule `CODELORE-HOTSPOT`; Plan 8 §2 added `CODELORE-CLONE`; Plan 8 §6 will add `CODELORE-LIVE-CLONE` (the high-severity intersection variant).

### 2.2 Live-clone × co-change intersection
Every clone detector (jscpd, PMD CPD, SourcererCC) finds copy-pasted blocks. CodeLore is the only OSS tool that intersects clones with **Fisher-significant change-coupling** — flagging only the clones whose copies actually evolve together. Dead clones (look-alike code nobody touches) are filtered out as noise; live clones (real debt) are surfaced with `combined_score = similarity × coupling_degree × (1 − p_value)`. This is CodeScene's "X-Ray" pattern shipped with CodeLore's transparency wedge below.

### 2.3 Transparency vs. opaque ML scoring
CodeScene's hotspot ranking is a closed ML model you cannot inspect. CodeLore ranks with a **published deterministic formula** (spec §1.1): `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (10 − code_health) / 10`. Every input column is emitted alongside the score so anyone can reproduce it. Same applies to Code Health (4-input weighted composite, spec §4.6), Fractal Value ownership (1 − Herfindahl-Hirschman Index), and the Fisher exact significance filter on coupling.

### 2.4 Provenance manifest — solving inter-tool disagreement
Spadoni et al. 2025 found ≥ 500% disagreement between behavioral-code-analysis tools, almost entirely due to silently differing thresholds and version drift. Every CodeLore run emits a **`.provenance.json` sidecar** recording every config knob, version pin, mailmap state, and UTC run timestamp. Your analyses are exactly reproducible and mathematically verifiable months later.

### 2.5 SQL-queryable fact store (DuckDB)
CodeLore does not lock data in a proprietary format. The full git-history fact store maps into **DuckDB** — a columnar analytics engine with disk-spill — and can be exported as standard Parquet or SQLite (`facts.db`). Run ad-hoc SQL queries from the command line; turn CodeLore into a database tool for your git metadata.

### 2.6 Persistent cache for CI-speed reruns
The `FactsDb` is content-addressed and cached at `$XDG_CACHE_HOME/codelore/<repo>/<sha>.duckdb`. A second `codelore analyze` invocation on the same `(repo, HEAD, options)` opens read-only in ≈ 10 ms — **100×+ speedup** on the dev inner loop and the foundation of the upcoming `codelore diff` PR-mode subcommand.

---

## 3. Architecture

CodeLore is a lightweight, single-binary CLI. The pipeline:

```
┌─────────────────────────────────────────────────────────┐
│                    User Repository                      │
└────────────────────────────┬────────────────────────────┘
                             │  [gix (Gitoxide) walk]
                             ▼
┌─────────────────────────────────────────────────────────┐
│                    codelore-lib                         │
│   ┌─────────────────────────────────────────────────┐   │
│   │           tree-sitter + codelore-rca            │   │
│   │     (function-level entity extraction at HEAD)  │   │
│   └────────────────────────┬────────────────────────┘   │
└────────────────────────────┼────────────────────────────┘
                             │  Stream<CommitEvent>
                             ▼  +  Kamei 14-feature enrichment
┌─────────────────────────────────────────────────────────┐
│                   DuckDB Fact Store                     │
│ commits · changes · hunks · entities · complexity ·     │
│ author_aliases · provenance · clones                    │
└────────────────────────────┬────────────────────────────┘
                             │  [SQL views + Rust orchestrators]
                             ▼
┌─────────────────────────────────────────────────────────┐
│              13 Analyses → 6 Output Formats             │
│    hotspots · coupling · ownership · code-health ·      │
│    code-age · churn(×3) · communication · summary ·     │
│    revisions · authors · clones · clone-coupling        │
│                                                         │
│         CSV · JSON · SARIF 2.1.0 · Markdown ·           │
│              Parquet · SQLite                           │
└─────────────────────────────────────────────────────────┘
                             │  [Persistent cache layer]
                             ▼
                $XDG_CACHE_HOME/codelore/...
```

### Why this stack

- **`gix` (Gitoxide)** — pure-Rust git over `libgit2`. No LGPL question, `+Send`/`+Sync` work correctly, memory-safe, native multithreading.
- **DuckDB** — embedded columnar analytics engine. Spill-to-disk lets us analyze the Linux kernel (~1.4M commits) in **under 10 minutes / under 4 GB RAM**. Power-user surface: the fact store IS a queryable SQL database.
- **tree-sitter via `codelore-rca`** (vendored Mozilla `rust-code-analysis` fork) — AST-based Cyclomatic, Cognitive, Halstead, and MI complexity for Rust, TypeScript, JavaScript, Python, Java. Real logical weight, not surface SLOC. Function-level extraction at HEAD.
- **`fishers_exact`** — statistical significance test (`p < 0.05` default) that filters spurious change-coupling pairs. The Fisher gate is what makes coupling and live-clone analyses methodologically defensible.

### Workspace

3-crate Cargo workspace:
- `codelore-lib` — types, `Repo` trait, gix-backed `GixRepo`, shell-out `GitCliRepo` differential oracle, DuckDB-backed `FactsDb`, 13 analyses, 6 output emitters, persistent cache layer
- `codelore-cli` — clap CLI (`analyze` + soon `diff`)
- `codelore-rca` — Mozilla `rust-code-analysis` vendor (MPL-2.0)

See the full design at [`docs/superpowers/specs/2026-06-06-codelore-design.md`](docs/superpowers/specs/2026-06-06-codelore-design.md).

---

## 4. Quick start

```bash
# Build from source
cargo build --release -p codelore-cli

# Run an analysis against any git repo (no installation needed)
./target/release/codelore analyze --analysis hotspots --repo . --min-revs 5
```

Output (CSV, the default):
```
entity,name,revisions,cognitive,code-health,hotspot-score
src/auth/session.rs,validate,87,42.0,4.2,0.8731
src/db/migrate.rs,run_migration,54,28.0,5.1,0.6204
...
```

### All 13 analyses

| Analysis | What it tells you |
|---|---|
| `revisions` | File → distinct commit count |
| `hotspots` | Published formula: rank × complexity × (10 − health). The ranking signal. |
| `code-health` | 4-input composite (cognitive 0.40 + churn 0.25 + fragmentation 0.15 + coupling 0.20) per spec §4.6 |
| `code-age` | Months since last modification per file |
| `abs-churn`, `author-churn`, `entity-churn` | Lines added/deleted grouped by date / author / file |
| `communication` | Conway's law shared-work author pairs |
| `code-ownership` | Fractal Value (1 − HHI) + main-developer per file |
| `change-coupling` | Fisher exact-filtered logical (temporal) coupling |
| `summary` | Repo-level overview (commits / changes / entities / authors) |
| `authors` | One row per canonical author, sorted by commit count |
| `clones` | Type 1 + Type 2 clone families via AST structural hashing on tree-sitter |
| `clone-coupling` | **Live clones**: clone families that also co-change at Fisher-significant rates — the differentiator |

### Every output format

```bash
# CSV (code-maat header parity)
codelore analyze --analysis hotspots --repo . --min-revs 5

# JSON (structured, machine-readable)
codelore analyze --analysis hotspots --repo . --format json

# SARIF 2.1.0 → GitHub Code Scanning / GitLab / Defectdojo
codelore analyze --analysis hotspots --repo . --format sarif --output hotspots.sarif

# Markdown → $GITHUB_STEP_SUMMARY
codelore analyze --analysis hotspots --repo . --format markdown >> "$GITHUB_STEP_SUMMARY"

# Parquet → DuckDB / Polars / pandas / Spark for downstream analytics
codelore analyze --analysis hotspots --repo . --format parquet --output hotspots.parquet

# SQLite → the full 8-table fact store for ad-hoc SQL exploration
codelore analyze --analysis revisions --repo . --format sqlite --output facts.db
```

Every file output gets a `{output}.provenance.json` sidecar (except `--format sqlite`, where the `provenance` table lives inside the DB). The sidecar records `codelore` / `gix` / `duckdb` / `tree-sitter` versions, every threshold knob, the mailmap state, and the UTC run timestamp.

### Excluding vendored or generated code

```bash
# Repeatable --exclude glob (Plan 8 §2)
codelore analyze --analysis clones --exclude 'vendor/**' --exclude '**/*_generated.rs'

# Or commit a .codeloreignore at your repo root (gitignore-style)
echo 'vendor/**' >> .codeloreignore
codelore analyze --analysis clones
```

### Cache control

```bash
# Skip the persistent cache (always fresh in-memory ingest)
codelore analyze --analysis hotspots --no-cache

# Override the XDG cache root (useful in CI with per-job caches)
codelore analyze --analysis hotspots --cache-dir /tmp/codelore-cache
```

---

## 5. Quality bar

349 tests pass across the workspace (199 RCA upstream + 6 Tier-1 smoke + 144 codelore-lib / codelore-cli suite). 3 ignored: 1 unrelated RCA upstream, 2 code-maat parity tests gated on `CODE_MAAT_PATH`.

`cargo bench -p codelore-lib --all-features` runs the criterion harness:
- `ingest_tiny` — 5-commit fixture ≈ 22 ms (sanity)
- `ingest/medium_500_commits` — 500-commit fixture (CI baseline)
- `ingest_kernel/linux_kernel_snapshot` — gated on `CODELORE_BENCH_LINUX_KERNEL_PATH`; weekly CI job

Gates: `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all --check`, `cargo deny check`. All green.

---

## 6. Roadmap

Plans 1–7 done. Plan 8 (v1.x release readiness) in flight:

- **Plan 1** ✅ — Phase 0 walking skeleton (workspace, gix walker, DuckDB fact store, revisions analysis)
- **Plan 2** ✅ — vendored Mozilla `rust-code-analysis` as `codelore-rca/`; Tier-1 metric smoke tests
- **Plan 3** ✅ — complexity integration; hotspots; Code Health (cognitive-only first pass)
- **Plan 4** ✅ — 8 new analyses + Kamei 14-feature vector + identity resolution + full §4.6 Code Health composite
- **Plan 5** ✅ — SARIF 2.1.0 + JSON + Markdown + Parquet + SQLite outputs + provenance manifest sidecar
- **Plan 6** ✅ — `GitCliRepo` differential oracle + 50-commit property tests; `criterion` benches; `cargo-dist` config; SLSA L3 provenance; distroless container; PGO scaffolding
- **Plan 7** ✅ — Clone detection (Type 1 + Type 2) via AST structural hashing on tree-sitter
- **Plan 8** 🚧 — v1.x release readiness:
  - **§1 pre-tag hardening** ✅ — README/spec/test/CLI-error fixes
  - **§2 spec-gap closures** ✅ — `--analysis authors` + `--group-file` + `--exclude` + `.codeloreignore` + clones JSON/Markdown/SARIF
  - **§3 persistent fact-store cache** ✅ — XDG-keyed `(repo, HEAD, options)` cache + LRU eviction + `--no-cache` / `--cache-dir`
  - **§4 FactsDb clones integration** ✅ — `clones` table populated during ingest
  - **§6 clone-coupling intersection** ✅ — core analysis shipped; CLI dispatch + `CODELORE-LIVE-CLONE` SARIF rule pending
  - **§5 parallel complexity extraction** ⏳ — Rayon `map_init` over working-tree walk
  - **§7 `codelore diff <base>..<head>`** ⏳ — PR-mode delta analysis (hotspots rank-entrant + coupling absent-change-pattern + new clone families)
  - **§8 docs + version bump + `v1.0.0` tag** ⏳

### Releasing v1.0

```bash
# 1. Generate code-maat goldens (one-time; needs lein installed)
bash scripts/capture-code-maat-goldens.sh

# 2. Kernel perf evidence (one-time; ~2 GB clone)
git clone --depth=10000 --filter=blob:none https://github.com/torvalds/linux /tmp/linux-snapshot
CODELORE_BENCH_LINUX_KERNEL_PATH=/tmp/linux-snapshot \
  cargo bench -p codelore-lib --all-features --bench end_to_end -- ingest_kernel

# 3. Bump workspace version + push tag
sed -i '' 's/0.1.0-alpha.1/1.0.0/' Cargo.toml
git commit -am "release: v1.0.0" && git tag -s v1.0.0 && git push --follow-tags
```

The tag push triggers `.github/workflows/release.yml` (cargo-dist multi-platform binaries + SLSA L3 provenance) and `.github/workflows/container.yml` (distroless image to `ghcr.io/<owner>/codelore`).

---

## 7. Advanced usage

- [`docs/superpowers/specs/2026-06-06-codelore-design.md`](docs/superpowers/specs/2026-06-06-codelore-design.md) — full design spec (~1100 lines) covering every analysis, threshold, identity rule, and Kamei feature
- [`docs/roadmap-v1.x-and-beyond.md`](docs/roadmap-v1.x-and-beyond.md) — prioritized backlog of v1.x and v2 work
- [`docs/perf-evidence-v1.md`](docs/perf-evidence-v1.md) — release-blocker performance evidence
- [`examples/`](examples/) — drop-in integration templates (GitHub Actions PR mode, more coming)
- [`docs/superpowers/plans/`](docs/superpowers/plans/) — every implementation plan, executed task-by-task

For the algorithmic detail on each analysis (formulas, default thresholds, edge cases) see spec §1.1 + §4.6 + §3.2.1.

---

## 8. Why "CodeLore"?

The technical category is "behavioral code analysis." The metaphor is reading the legends a codebase tells about itself. Every commit is tribal lore: who knew this code, who burned themselves on it, where the workarounds calcified, which functions are quietly cloned across a dozen files because everyone fixed the same bug in their corner. CodeLore surfaces that lore as data you can act on.

---

## 9. License

**GPL-3.0-only.** Bundles a vendored fork of Mozilla's `rust-code-analysis` under **MPL-2.0** — see [`crates/codelore-rca/LICENSE-MPL`](crates/codelore-rca/LICENSE-MPL) and [`crates/codelore-rca/UPSTREAM.md`](crates/codelore-rca/UPSTREAM.md) for vendoring history and modification notes.
