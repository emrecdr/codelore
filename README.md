# CodeLore

> **Read the lore of your codebase.**

Behind every codebase is a human narrative your linter cannot see: who wrote this, who still understands it, which corners hide tribal knowledge nobody's written down, and where the historical scars are buried. Every commit is a piece of this **lore**.

**CodeLore** mines your repository's git history and projects it into behavioral insight — hotspots, change-coupling, ownership maps, knowledge fragmentation, code health scores, and live code clones — surfaced as SARIF for your existing CI dashboard. The socio-technical signal your linter cannot see, with the methodological honesty your team can audit.

A Rust modernization of Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat). Built on [gix](https://github.com/GitoxideLabs/gitoxide) (pure-Rust git), [DuckDB](https://duckdb.org) (embedded analytics), and a vendored fork of Mozilla's [rust-code-analysis](https://github.com/mozilla/rust-code-analysis) (tree-sitter complexity).

---

## Why you need this

Static analyzers (SonarQube, ESLint, Clippy) read code at a single point in time. CodeLore reads its **history**, and that history answers questions static tools can't:

- **Bus-factor risk** — *"Which complex hotspots are owned by a single contributor — what happens when they go on leave?"*
- **Hidden Architectural Debt** — *"Which files are implicitly coupled — always modified together — but live in different subsystems?"*
- **Refactoring ROI** — *"Which highly complex files are actively changing (refactor!) vs stable (leave alone)?"*
- **Live Clones** — *"Which copy-pasted blocks keep being edited in lockstep (real debt) vs which are dead patterns nobody touches (noise)?"*

CodeLore focuses on the **socio-technical dimension** — the legends your codebase tells about itself — so you can focus refactor effort where it actually pays off.

---

## What makes CodeLore different

What separates CodeLore from code-maat, CodeScene, and jscpd:

- **🎯 Live-clone × co-change intersection.** Every clone detector finds copy-pasted blocks. CodeLore intersects clones with Fisher-significant change-coupling — flagging only the clones whose copies actually evolve together. Dead clones (look-alike code nobody touches) are filtered out as noise; live clones (real debt) are surfaced with a `combined_score` ranking. We're not aware of another OSS tool that ships this intersection.
- **📋 Behavioral SARIF.** Findings land natively in **SARIF 2.1.0** with three rules — `CODELORE-HOTSPOT`, `CODELORE-CLONE`, `CODELORE-LIVE-CLONE`. Drop them straight into GitHub Code Scanning, GitLab security dashboards, or Defectdojo and alerts appear inline on pull requests.
- **🔍 Transparency over opaque ML.** CodeScene's hotspot ranking is a closed ML model. CodeLore ranks with a **published deterministic formula**: `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (10 − code_health) / 10`. Every input is emitted alongside the score; anyone can reproduce it.
- **🧾 Provenance manifest.** Every run emits a `.provenance.json` sidecar recording every config knob, version pin, and timestamp. Reproducibility receipt for the run; eliminates the "we got different numbers because we silently used different thresholds" failure mode that plagues comparisons between behavioral analyzers.
- **💾 SQL-queryable fact store.** No proprietary format lock-in. Export the full DuckDB store as Parquet or SQLite and query your git history as a database from the command line.
- **⚡ Persistent cache.** Second invocation on the same `(repo, HEAD, options)` opens read-only in ~10 ms instead of re-walking history — typically a 10-100× speedup on the dev inner loop depending on repo size, and the foundation of the `codelore diff` PR-mode subcommand.

---

## Quick start

```bash
# Build from source (Rust toolchain required)
cargo build --release -p codelore-cli

# Or once a published release is available:
cargo binstall codelore
```

Either symlink the binary or invoke it with the full path. The remaining snippets in this README assume `codelore` is on your PATH — adjust to `./target/release/codelore` if you skipped the install step.

```bash
# Your first analysis: the top 10 hotspots in any git repo
codelore analyze --analysis hotspots --repo . --min-revs 5
```

Output (CSV, the default):

```
entity,name,revisions,cognitive,code-health,hotspot-score
src/auth/session.rs,,87,42.0,4.2,0.8731
src/db/migrate.rs,,54,28.0,5.1,0.6204
...
```

The top row is your most pressing refactor candidate: high churn × high complexity × low code health = highest score. (The `name` column is reserved for future function-level rollups; file-level rows leave it empty.)

---

## Your first 5 minutes with CodeLore

Three commands that build intuition:

**1. What does the repo look like?**

```bash
codelore analyze --analysis summary --repo .
```

A one-page snapshot: commits, files, authors. Confirms you're pointed at the right git history.

**2. Where's the technical debt?**

```bash
codelore analyze --analysis hotspots --repo . --min-revs 5 --rows 10
```

The top 10 files ranked by hotspot score. Scan the list — usually 2-3 names jump out as "I've been meaning to refactor that". This is your `--top-n 3` for the next CI quality gate.

**3. Which copy-pasted code is actually hurting you?**

```bash
codelore analyze --analysis clone-coupling --repo . --format markdown
```

Live clones — function-level copy-paste families whose copies co-change at Fisher-significant rates. These are the *real* code-duplication debts: every change has to be made in N places, every bug has N variants. Dead clones (filtered out) are noise.

Once you've run those three, you have enough signal to triage. From here, [the advanced guide](docs/advanced-usage.md) covers all 14 analyses, every flag, configuration, CI integration, and tool-stack rationale.

---

## In CI: PR-mode delta analysis

```bash
codelore diff origin/main...HEAD \
  --analysis all \
  --output markdown >> "$GITHUB_STEP_SUMMARY"
```

Three signals per PR:

- **Hotspot deltas** — files newly entering the top-N or worsening their score
- **Missing co-changes** — "you changed `auth/login.rs` but historically `auth/session.rs` always changes with it — did you forget?" (the CodeScene-signature signal)
- **New clone families** — copy-paste debt introduced by the PR

Optional quality gate: `--fail-on rank-entrant` exits non-zero if the PR promotes any file into the top-N hotspots. See [`examples/.github/workflows/codelore-pr.yml`](examples/.github/workflows/codelore-pr.yml) for the full template with the critical configuration gotchas (`fetch-depth: 0`, three-dot merge-base, SARIF upload).

---

## How it works (the 30-second version)

```
   Your git repo
        │  [gix walks history]
        ▼
   ┌────────────────────┐
   │  codelore-lib       │
   │  ┌──────────────┐   │   tree-sitter parses each Tier-1
   │  │ codelore-rca  │   │   source file → cyclomatic, cognitive,
   │  └──────────────┘   │   Halstead, MI, AST structural hash
   └────────┬───────────┘
            │ Stream<CommitEvent>
            ▼
   ┌────────────────────┐
   │   DuckDB Fact Store │  commits · changes · hunks · entities ·
   │                    │  complexity_metrics · clones · author_aliases · provenance
   └────────┬───────────┘
            │ SQL views + Rust orchestrators
            ▼
   ┌────────────────────┐
   │  14 Analyses        │  → 6 Output formats
   │                    │  → optional persistent cache
   └────────────────────┘
```

Every commit becomes a `CommitEvent` projected onto a DuckDB fact store. The 14 analyses are SQL views over that store plus a thin Rust orchestrator each. Outputs flow through six format emitters. Every run is cached and audit-trail-stamped with a provenance sidecar.

For deeper architecture, see the [design specification](docs/superpowers/specs/2026-06-06-codelore-design.md) (~1100 lines, covers every threshold and identity rule).

---

## Why these tools?

| Why this | Why not the alternative |
|---|---|
| **gix** (gitoxide, pure-Rust git) | libgit2 has LGPL friction and a C build dep; gix is pure-Rust and natively `Send + Sync` |
| **DuckDB** (embedded columnar analytics) | SQLite isn't columnar; rolling-your-own gives up the SQL surface that's a power-user feature. Polars works for in-memory but doesn't expose embedded SQL the way DuckDB does |
| **tree-sitter** via vendored `rust-code-analysis` | Hand-rolled per-language parsers don't scale; tree-sitter gives us Rust + Python + Java + JS/TS for free and AST hashing for clones falls out naturally |
| **Rayon** + crossbeam-channel | Workload is CPU-bound batch; an async runtime would add binary bloat for no measurable gain |
| **`fishers_exact`** for change-coupling | Approximate chi-square fails at small N; exact test is methodologically defensible and the crate has zero transitive dependencies |

What we deliberately don't ship: no async runtime, no libgit2 binding, no LLM-based scoring, no web UI. See the [advanced guide](docs/advanced-usage.md#7-tool-stack-why-these-choices) for the long version.

---

## Status

Release-ready alpha. 14 analyses × 6 output formats × `codelore diff` PR-mode. 349 tests pass, clippy / fmt / deny all green. The first stable tag is the only remaining gate.

Known limitations (the honest list, validated against the current codebase):

- **Rename tracking** — `ChangeType::Renamed` is captured at ingest but analyses don't follow renames yet (a renamed file's history splits across old + new paths)
- **`Options` cross-field validation** — pathological combinations like `min_revs > max_changeset_size` silently return empty results
- **Hand-rolled CSV emitter** — `quote_if_needed` covers the worst case but a `csv`-crate migration is on the open list
- **Clone extraction is still single-threaded** — same Rayon pattern as the parallel complexity extraction is queued next
- **Parallel complexity extraction is workload-dependent** — the parallel pass beats serial measurably only on codebases with many Tier-1 source files; on small repos (< 30 files) the speedup is within bench noise because the bottleneck is elsewhere (commit walk + change-feature enrichment)

Full backlog with priority ranks: [`docs/codebase_analysis_report.md`](docs/codebase_analysis_report.md).

---

## Documentation

| If you want… | Read |
|---|---|
| All 14 analyses + every flag + CI patterns + troubleshooting | [`docs/advanced-usage.md`](docs/advanced-usage.md) |
| The deep-dive architecture review + open-item backlog | [`docs/codebase_analysis_report.md`](docs/codebase_analysis_report.md) |
| The full design specification (~1100 lines) | [`docs/superpowers/specs/2026-06-06-codelore-design.md`](docs/superpowers/specs/2026-06-06-codelore-design.md) |
| The prioritized roadmap (near-term and long-term backlog) | [`docs/roadmap-v1.x-and-beyond.md`](docs/roadmap-v1.x-and-beyond.md) |
| Release-blocker performance numbers | [`docs/perf-evidence-v1.md`](docs/perf-evidence-v1.md) |
| Drop-in CI integration templates | [`examples/`](examples/) |
| The version-by-version release log | [`CHANGELOG.md`](CHANGELOG.md) |

---

## Why "CodeLore"?

The technical category is "behavioral code analysis." The metaphor is reading the **legends** a codebase tells about itself.

Every commit is tribal lore: who knew this code, who burned themselves on it, where the workarounds calcified, which functions are quietly cloned across a dozen files because everyone fixed the same bug in their corner. The word *lore* captures that human narrative more honestly than "metrics" or "telemetry" — it acknowledges that the most important signal about your codebase isn't in the code, it's in the people who wrote it.

CodeLore surfaces that lore as data you can act on, without pretending the methodology is more scientific than it is. Every formula is published, every threshold is documented, and every run leaves a provenance receipt. Read the lore. Act on what it tells you.

---

## License

**GPL-3.0-only.** Bundles a vendored fork of Mozilla's `rust-code-analysis` under **MPL-2.0** — see [`crates/codelore-rca/LICENSE-MPL`](crates/codelore-rca/LICENSE-MPL) and [`crates/codelore-rca/UPSTREAM.md`](crates/codelore-rca/UPSTREAM.md) for vendoring history.
