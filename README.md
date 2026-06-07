# CodeLore

> **Read the lore of your codebase.**

Behind every codebase is a human narrative your linter cannot see: who wrote this, who still understands it, which corners hide tribal knowledge nobody's written down, and where the historical scars are buried. Every commit is a piece of this **lore**.

**CodeLore** mines your repository's git history and projects it into behavioral insight — hotspots, change-coupling, ownership maps, knowledge fragmentation, code health scores, and live code clones — surfaced as SARIF for your existing CI dashboard. The socio-technical signal your linter cannot see, with the methodological honesty your team can audit.

A Rust modernization of Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat). Built on [gix](https://github.com/GitoxideLabs/gitoxide) (pure-Rust git), [DuckDB](https://duckdb.org) (embedded analytics), and a vendored fork of Mozilla's [rust-code-analysis](https://github.com/mozilla/rust-code-analysis) (tree-sitter complexity).

---

## Why you need this

Static analyzers (SonarQube, ESLint, Clippy) read code at a single point in time. CodeLore reads its **history**, and that history answers questions static tools can't:

- **Knowledge Loss Risk** — *"Which complex hotspot was written by an author who left 2 months ago?"*
- **Hidden Architectural Debt** — *"Which files are implicitly coupled — always modified together — but live in different subsystems?"*
- **Refactoring ROI** — *"Which highly complex files are actively changing (refactor!) vs stable (leave alone)?"*
- **Live Clones** — *"Which copy-pasted blocks keep being edited in lockstep (real debt) vs which are dead patterns nobody touches (noise)?"*

CodeLore focuses on the **socio-technical dimension** — the legends your codebase tells about itself — so you can focus refactor effort where it actually pays off.

---

## What makes CodeLore different

What separates CodeLore from code-maat, CodeScene, and jscpd:

- **🎯 Live-clone × co-change intersection.** Every clone detector finds copy-pasted blocks. CodeLore is the only OSS tool that intersects clones with Fisher-significant change-coupling — flagging only the clones whose copies actually evolve together. Dead clones (look-alike code nobody touches) are filtered out as noise; live clones (real debt) are surfaced with a `combined_score` ranking.
- **📋 Behavioral SARIF.** Findings land natively in **SARIF 2.1.0** with three rules — `CODELORE-HOTSPOT`, `CODELORE-CLONE`, `CODELORE-LIVE-CLONE`. Drop them straight into GitHub Code Scanning, GitLab security dashboards, or Defectdojo and alerts appear inline on pull requests. No other behavioral-analysis tool ships SARIF natively.
- **🔍 Transparency over opaque ML.** CodeScene's hotspot ranking is a closed ML model. CodeLore ranks with a **published deterministic formula**: `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (10 − code_health) / 10`. Every input is emitted alongside the score; anyone can reproduce it.
- **🧾 Provenance manifest.** Every run emits a `.provenance.json` sidecar recording every config knob, version pin, and timestamp. Solves the inter-tool disagreement problem ([Spadoni 2025](https://arxiv.org/) found ≥ 500% disagreement between behavioral analyzers, almost entirely due to silently differing thresholds).
- **💾 SQL-queryable fact store.** No proprietary format lock-in. Export the full DuckDB store as Parquet or SQLite and query your git history as a database from the command line.
- **⚡ Persistent cache.** Second invocation on the same `(repo, HEAD, options)` opens read-only in ~10 ms — a 100× speedup on the dev inner loop, and the foundation of the `codelore diff` PR-mode subcommand.

---

## Quick start

```bash
# Build from source (Rust toolchain required)
cargo build --release -p codelore-cli

# Or once a published release is available:
cargo binstall codelore
```

```bash
# Your first analysis: the top 10 hotspots in any git repo
./target/release/codelore analyze --analysis hotspots --repo . --min-revs 5
```

Output (CSV, the default):

```
entity,name,revisions,cognitive,code-health,hotspot-score
src/auth/session.rs,validate,87,42.0,4.2,0.8731
src/db/migrate.rs,run_migration,54,28.0,5.1,0.6204
...
```

The top row is your most pressing refactor candidate: high churn × high complexity × low code health = highest score.

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

Once you've run those three, you have enough signal to triage. From here, [the advanced guide](docs/advanced-usage.md) covers all 13 analyses, every flag, configuration, CI integration, and tool-stack rationale.

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
   │   DuckDB Fact Store │  commits · changes · entities · clones ·
   │                    │  complexity_metrics · author_aliases · provenance
   └────────┬───────────┘
            │ SQL views + Rust orchestrators
            ▼
   ┌────────────────────┐
   │  13 Analyses        │  → 6 Output formats
   │                    │  → optional persistent cache
   └────────────────────┘
```

Every commit becomes a `CommitEvent` projected onto a DuckDB fact store. The 13 analyses are SQL views over that store plus a thin Rust orchestrator each. Outputs flow through six format emitters. Every run is cached and audit-trail-stamped with a provenance sidecar.

For deeper architecture, see the [design specification](docs/superpowers/specs/2026-06-06-codelore-design.md) (~1100 lines, covers every threshold and identity rule).

---

## Why these tools?

| Why this | Why not the alternative |
|---|---|
| **gix** (gitoxide, pure-Rust git) | libgit2 has LGPL friction and a C build dep; gix is faster on cold reads and has native `Send + Sync` |
| **DuckDB** (embedded columnar analytics) | Polars doesn't have spill-to-disk; SQLite isn't columnar; rolling-your-own gives up the SQL surface that's a power-user feature |
| **tree-sitter** via vendored `rust-code-analysis` | Hand-rolled per-language parsers don't scale; tree-sitter gives us Rust + Python + Java + JS/TS for free and AST hashing for clones falls out naturally |
| **Rayon** + crossbeam-channel | Workload is CPU-bound batch; tokio would add 200 KB of binary bloat for no gain. |
| **`fishers_exact`** for change-coupling | Approximate chi-square fails at small N; exact test is methodologically defensible and the crate is < 200 lines |

What we deliberately don't ship: no async runtime, no libgit2 binding, no LLM-based scoring, no web UI. See the [advanced guide](docs/advanced-usage.md#7-tool-stack-why-these-choices) for the long version.

---

## Status

Release-ready alpha. 13 analyses × 6 output formats × `codelore diff` PR-mode. 349 tests pass, clippy / fmt / deny all green. The first stable tag is the only remaining gate.

Known limitations (the honest list, validated against the current codebase):

- **Rename tracking** — `ChangeType::Renamed` is captured at ingest but analyses don't follow renames yet (a renamed file's history splits across old + new paths)
- **`Options` cross-field validation** — pathological combinations like `min_revs > max_changeset_size` silently return empty results
- **Hand-rolled CSV emitter** — `quote_if_needed` covers the worst case but a `csv`-crate migration is on the open list
- **Clone extraction is still single-threaded** — same Rayon pattern as the parallel complexity extraction (which already shipped) is queued next

Full backlog with priority ranks: [`docs/codebase_analysis_report.md`](docs/codebase_analysis_report.md).

---

## Documentation

| If you want… | Read |
|---|---|
| All 13 analyses + every flag + CI patterns + troubleshooting | [`docs/advanced-usage.md`](docs/advanced-usage.md) |
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
