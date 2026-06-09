# CodeLore

<!-- Primary topic discovery row (mirrors code-maat's own GitHub topics). -->
[![Topic: behavioral-code-analysis](https://img.shields.io/badge/topic-behavioral--code--analysis-7048e8?style=flat-square&logo=github)](https://github.com/topics/behavioral-code-analysis)
[![Topic: code-analysis-tool](https://img.shields.io/badge/topic-code--analysis--tool-7048e8?style=flat-square&logo=github)](https://github.com/topics/code-analysis-tool)
[![Topic: repository-mining](https://img.shields.io/badge/topic-repository--mining-7048e8?style=flat-square&logo=github)](https://github.com/topics/repository-mining)
[![Topic: technical-debt](https://img.shields.io/badge/topic-technical--debt-7048e8?style=flat-square&logo=github)](https://github.com/topics/technical-debt)
[![Topic: code-maat](https://img.shields.io/badge/topic-code--maat-7048e8?style=flat-square&logo=github)](https://github.com/topics/code-maat)

<!-- Tech-stack + secondary discovery row -->
[![Topic: rust](https://img.shields.io/badge/topic-rust-dea584?style=flat-square&logo=rust)](https://github.com/topics/rust)
[![Topic: sarif](https://img.shields.io/badge/topic-sarif-7048e8?style=flat-square&logo=github)](https://github.com/topics/sarif)
[![Topic: clone-detection](https://img.shields.io/badge/topic-clone--detection-7048e8?style=flat-square&logo=github)](https://github.com/topics/clone-detection)
[![Topic: hotspot-analysis](https://img.shields.io/badge/topic-hotspot--analysis-7048e8?style=flat-square&logo=github)](https://github.com/topics/hotspot-analysis)
[![Topic: developer-tools](https://img.shields.io/badge/topic-developer--tools-7048e8?style=flat-square&logo=github)](https://github.com/topics/developer-tools)

<!-- Release + license meta -->
[![Version](https://img.shields.io/badge/version-0.1.0-2ea44f?style=flat-square)](CHANGELOG.md)
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue?style=flat-square)](LICENSE)
[![Rust 1.96+](https://img.shields.io/badge/rust-1.96%2B-dea584?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Tests: 396 passing](https://img.shields.io/badge/tests-396%20passing-2ea44f?style=flat-square)](#status)
[![Clippy: clean](https://img.shields.io/badge/clippy-clean-2ea44f?style=flat-square&logo=rust)](#status)

> **Read the lore of your codebase.**

Behind every codebase is a human narrative your linter cannot see: who wrote this, who still understands it, which corners hide tribal knowledge nobody's written down, and where the historical scars are buried. Every commit is a piece of this **lore**.

**CodeLore** mines your repository's git history and projects it into 21 behavioral analyses — hotspots, change-coupling, ownership maps, knowledge fragmentation, code health scores, copy-paste clones, live clones (clones × Fisher-significant co-change), commit-message regex matching, sum-of-coupling, main-developer rankings, and more — surfaced as SARIF for your existing CI dashboard. The socio-technical signal your linter cannot see, with the methodological honesty your team can audit.

A Rust **drop-in successor** to Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat) — every published code-maat analysis is supported under the same `--analysis NAME` flag, with modern improvements: deterministic tiebreaks, Fisher exact significance gates, SARIF output, persistent cache, PR-mode diffing, and a SQL-queryable fact store. Built on [gix](https://github.com/GitoxideLabs/gitoxide) (pure-Rust git), [DuckDB](https://duckdb.org) (embedded analytics), [fancy-regex](https://github.com/fancy-regex/fancy-regex) (lookaround support for architectural grouping), and a vendored fork of Mozilla's [rust-code-analysis](https://github.com/mozilla/rust-code-analysis) (tree-sitter complexity).

---

## Why you need this

Static analyzers (SonarQube, ESLint, Clippy) read code at a single point in time. CodeLore reads its **history**, and that history answers questions static tools can't:

- **Bus-factor risk** — *"Which complex hotspots are owned by a single contributor — what happens when they go on leave?"*
- **Hidden architectural debt** — *"Which files are implicitly coupled — always modified together — but live in different subsystems?"*
- **Refactoring ROI** — *"Which highly complex files are actively changing (refactor!) vs stable (leave alone)?"*
- **Live clones** — *"Which copy-pasted blocks keep being edited in lockstep (real debt) vs which are dead patterns nobody touches (noise)?"*
- **Modernization scope** — *"Which files do my AI-coding-assistant commits touch most heavily?"* (`ai_attribution` column on every commit; auto-detects Claude / Copilot / Cursor / Aider / Cody / Continue / Codeium / Windsurf / Devin / Tabnine / Amazon Q)

CodeLore focuses on the **socio-technical dimension** — the legends your codebase tells about itself — so you can focus refactor effort where it actually pays off.

---

## What makes CodeLore different

What separates CodeLore from code-maat, CodeScene, and jscpd:

- **🎯 Live-clone × co-change intersection.** Every clone detector finds copy-pasted blocks. CodeLore intersects clones with Fisher-significant change-coupling — flagging only the clones whose copies actually evolve together. Dead clones (look-alike code nobody touches) are filtered out as noise; live clones (real debt) are surfaced with a `combined_score` ranking. We're not aware of another OSS tool that ships this intersection.
- **📋 Behavioral SARIF.** Findings land natively in **SARIF 2.1.0** with four rules — `CODELORE-HOTSPOT`, `CODELORE-CLONE`, `CODELORE-LIVE-CLONE`, and `CODELORE-MISSING-COCHANGE`. Drop them straight into GitHub Code Scanning, GitLab security dashboards, or Defectdojo and alerts appear inline on pull requests.
- **🔍 Transparency over opaque ML.** CodeScene's hotspot ranking is a closed ML model. CodeLore ranks with a **published deterministic formula**: `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (100 − code_health) / 10`. Every input is emitted alongside the score; anyone can reproduce it.
- **🧾 Provenance manifest.** Every run emits a `.provenance.json` sidecar recording every config knob (auto-derived via canonical Options serialization — adding a new field auto-propagates), version pin, and timestamp. Reproducibility receipt for the run; eliminates the "we got different numbers because we silently used different thresholds" failure mode.
- **💾 SQL-queryable fact store.** No proprietary format lock-in. Export the full DuckDB store as Parquet or SQLite and query your git history as a database from the command line.
- **⚡ Persistent cache.** Second invocation on the same `(repo, HEAD, options)` opens read-only in ~10 ms instead of re-walking history — typically a 10-100× speedup on the dev inner loop depending on repo size, and the foundation of the `codelore diff` PR-mode subcommand.
- **🔗 Drop-in code-maat compatibility.** Every published code-maat analysis is supported under the same `--analysis NAME`. The `--code-maat-compat` flag (planned) flips internal defaults back to legacy semantics for users with dashboards that parse code-maat CSV verbatim.

---

## The 21 analyses

Use `codelore analyze --analysis NAME` for any of these. Code-maat parity is **complete**; modern additions are marked **★**.

### Core behavioral signals (code-maat parity)

| Analysis | Output | Use case |
|---|---|---|
| `revisions` | per-file commit count | First-look hotspot proxy |
| `coupling` | file pairs with Fisher-significant co-change | Hidden architectural debt |
| `soc` | sum of coupling per file (centrality measure) | Network-level coupling outliers |
| `code-age` | months since last modification per file | Find dead code + recently-volatile areas |
| `abs-churn` | LOC added/deleted per date | Trend dashboards |
| `author-churn` | LOC added/deleted per author | Effort distribution |
| `entity-churn` | LOC added/deleted per file | Refactor-target ranking |
| `communication` | author pairs by shared-file work | Conway's law signals |
| `authors` | commits per canonical author | Onboarding / recognition |
| `summary` | one-page repo overview | First slide of any review |
| `ownership` | Fractal Value (1-HHI) per file + main-author | Bus-factor / knowledge-loss risk |
| `entity-effort` | per-(file, author) revision counts | "Who's doing the work on this file?" |
| `entity-ownership` | per-(file, author) added/deleted breakdown | Fine-grained ownership view |
| `main-dev` | top author per file by lines added | Onboarding / handoff |
| `main-dev-by-revs` | top author per file by revision count | Stewardship view |
| `main-dev-by-deletions` (alias: `refactoring-main-dev`) | top author per file by lines removed | Refactoring authorship |
| `messages` | per-file count of commits matching `--expression-to-match` regex | Bug/refactor archaeology |

### Modern additions ★

| Analysis | Output | What it adds beyond code-maat |
|---|---|---|
| `hotspots` ★ | files ranked by `percentile_rank(revs) × percentile_rank(cognitive) × (100 − code_health) / 10` | Published formula transparency; CodeScene-equivalent signal |
| `code-health` ★ | composite score 0..100 per file (cognitive + churn + Fractal Value + Fisher-filtered coupling centrality) | Multi-dimensional file-quality score |
| `clones` ★ | Type 1 + Type 2 clone families via AST structural hashing | Function-level copy-paste detection across Rust/Python/Java/JS/TS |
| `clone-coupling` ★ | clones intersected with Fisher-significant co-change | **The strategic differentiator** — separates live debt from dead noise |

---

## Quick start

Pick whichever fits your machine:

```bash
# Homebrew (macOS or Linuxbrew, arm64 or x86_64):
brew install emrecdr/codelore/codelore

# Prebuilt binary via cargo-binstall (any Rust dev environment):
cargo binstall codelore

# From source (Rust 1.96+ toolchain required):
cargo install --git https://github.com/emrecdr/codelore codelore-cli
```

Or grab a prebuilt archive straight from a [GitHub Release](https://github.com/emrecdr/codelore/releases/latest) — five targets ship per tag (macOS arm64/x86_64, Linux arm64/x86_64-gnu, Windows x86_64-msvc), each with SLSA L3 build provenance attached.

The rest of this README assumes `codelore` is on your PATH — substitute `./target/release/codelore` if you skipped the install step.

```bash
# Your first analysis: the top 10 hotspots in any git repo
codelore analyze --analysis hotspots --repo . --min-revs 5 --rows 10
```

Before the analysis runs, codelore prints a pre-flight banner to stderr (auto-suppressed when piped; suppress explicitly with `--no-banner`):

```
────────────────────────────────────────────────────────────────────────
 codelore 0.1.2                           gix 0.84.0 · duckdb 1.10503.1
────────────────────────────────────────────────────────────────────────
 Repo:     /Users/you/code/your-project
 Branch:   main @ a891295
 Analysis: hotspots  (min-revs=5, rows=10)
 Status:   ✓ ready
────────────────────────────────────────────────────────────────────────
```

The banner doubles as a fail-fast gate: if the path isn't a git repo, the repo has no commits, or `--output` points at a directory that doesn't exist, the banner renders `Status: ✗ <reason>` with a one-line `Hint:` and codelore exits non-zero — before spending 5–30 seconds on ingest you'd have to abort anyway.

Output (CSV, the default, on stdout — pipeable into other tools):

```
entity,revisions,cognitive,code-health,hotspot-score
src/auth/session.rs,87,42.00,60.00,9.1837
src/db/migrate.rs,54,28.00,71.20,4.6125
src/api/handlers.rs,38,18.00,80.36,2.4310
```

- `code-health ∈ [60, 100]` — higher = healthier (60 is the floor because the cognitive-complexity term contributes at most 40 points of deduction).
- `hotspot-score ∈ [0, 10]` — higher = more pressing refactor candidate. `9.18` means "near the top of the curve on revisions × complexity × poor health" — the canonical "on fire" file.

The top row is the file to look at first: high churn × high complexity × low code health = highest score.

When the analysis completes, codelore prints a footer summary to stderr (same TTY suppression rules):

```
────────────────────────────────────────────────────────────────────────
 ✓ hotspots completed in 4.3s
────────────────────────────────────────────────────────────────────────
```

---

## Your first 5 minutes with CodeLore

Four commands that build intuition:

**1. What does the repo look like?**

```bash
codelore analyze --analysis summary --repo .
```

One-page snapshot: commits, files, authors. Confirms you're pointed at the right git history.

**2. Where's the technical debt?**

```bash
codelore analyze --analysis hotspots --repo . --min-revs 5 --rows 10
```

Top 10 files ranked by hotspot score. Usually 2-3 names jump out as "I've been meaning to refactor that".

**3. Who owns the risky code?**

```bash
codelore analyze --analysis ownership --repo . --rows 10
```

Files sorted by ownership fragmentation (Fractal Value). High FV = many contributors share the file; low FV = bus-factor risk.

**4. Which copy-pasted code is actually hurting you?**

```bash
codelore analyze --analysis clone-coupling --repo . --format markdown
```

Live clones — function-level copy-paste families whose copies co-change at Fisher-significant rates. Real code-duplication debt: every change has to be made in N places, every bug has N variants. Dead clones (filtered out) are noise.

Once you've run those four, you have enough signal to triage. From here, [the advanced guide](docs/advanced-usage.md) covers all 21 analyses, every flag, configuration, CI integration, and tool-stack rationale.

---

## In CI: PR-mode delta analysis

```bash
codelore diff origin/main...HEAD \
  --analysis all \
  --format markdown \
  --output - >> "$GITHUB_STEP_SUMMARY"
```

Four signals per PR, surfaced via SARIF or human-readable Markdown:

- **Hotspot deltas** — files newly entering the top-N or worsening their score (`CODELORE-HOTSPOT` SARIF rule)
- **Missing co-changes** — "you changed `auth/login.rs` but historically `auth/session.rs` always changes with it — did you forget?" (`CODELORE-MISSING-COCHANGE` SARIF rule, the CodeScene-signature signal)
- **New clone families** — copy-paste debt introduced by the PR (`CODELORE-CLONE` SARIF rule)
- **Live clones** — clones whose copies co-change at Fisher-significant rates (`CODELORE-LIVE-CLONE` SARIF rule)

Quality-gate options:

```bash
# Block PRs that promote any file into the top-N hotspots:
codelore diff origin/main...HEAD --fail-on rank-entrant

# Block PRs that worsen an existing hotspot:
codelore diff origin/main...HEAD --fail-on score-increase

# Block on any of the four signals:
codelore diff origin/main...HEAD --fail-on any
```

See [`examples/.github/workflows/codelore-pr.yml`](examples/.github/workflows/codelore-pr.yml) for the full template with the critical configuration gotchas (`fetch-depth: 0`, three-dot merge-base, SARIF upload permissions).

Cache the base-rev analysis with `--base-cache PATH` to halve dual-analysis cost across PRs that share the same base SHA.

---

## Architectural grouping

For monorepos, treat groups of files as logical components. Drop a `groups.txt` at the repo root:

```text
# CodeLore architectural grouping. One rule per line; `<path-or-regex> => <group-name>`.
src/auth                   => Auth
src/db                     => DB
src/api                    => API
^src\/.*\/tests\/.*\.rs$   => Tests
^src\/((?!.*test.*).).*$   => Production
```

Run with `--group-file groups.txt`:

```bash
codelore analyze --group-file groups.txt --analysis revisions
```

Analyses then operate at the **group** level — `Auth`, `DB`, etc. — instead of raw paths. Plain-text LHS is matched as a prefix (anchored + slash-bound); regex LHS (starting with `^`) supports **full lookaround** via `fancy-regex` (code-maat's own test fixtures use this).

Default: **non-strict** (unmapped paths keep their raw names; safer than silent drop). Pass `--strict-grouping` to drop unmapped paths instead (code-maat's behavior).

---

## How it works (the 30-second version)

```
   Your git repo
        │  [gix walks history]
        ▼
   ┌─────────────────────┐
   │  codelore-lib        │
   │  ┌──────────────┐    │   tree-sitter parses each Tier-1 source
   │  │ codelore-rca │    │   file → cyclomatic, cognitive, Halstead,
   │  └──────────────┘    │   MI metrics, AST structural hash
   └────────┬────────────┘
            │ Stream<CommitEvent>
            ▼  + Kamei 14-feature enrichment
   ┌─────────────────────┐
   │   DuckDB Fact Store  │  commits · changes · hunks · entities ·
   │                     │  complexity_metrics · clones ·
   │                     │  author_aliases · provenance
   └────────┬────────────┘
            │ SQL queries (bind-parameterized) + Rust orchestrators
            ▼
   ┌─────────────────────┐
   │  21 Analyses         │  → 6 output formats (CSV/JSON/SARIF/
   │                     │     Markdown/Parquet/SQLite)
   │                     │  → persistent cache (10-100× speedup)
   │                     │  → provenance.json sidecar
   └─────────────────────┘
```

Every commit becomes a `CommitEvent` projected onto a DuckDB fact store. The 21 analyses are SQL queries over that store plus a thin Rust orchestrator each. Outputs flow through six format emitters. Every run is cached and audit-trail-stamped with a provenance sidecar.

For deeper architecture, see the [design specification](docs/superpowers/specs/2026-06-06-codelore-design.md) (~1100 lines, covers every threshold and identity rule).

---

## Why these tools?

| Why this | Why not the alternative |
|---|---|
| **gix** (gitoxide, pure-Rust git) | libgit2 has LGPL friction and a C build dep; gix is pure-Rust and natively `Send + Sync` |
| **DuckDB** (embedded columnar analytics) | SQLite isn't columnar; rolling-your-own gives up the SQL surface that's a power-user feature. Polars works for in-memory but doesn't expose embedded SQL the way DuckDB does |
| **tree-sitter** via vendored `rust-code-analysis` | Hand-rolled per-language parsers don't scale; tree-sitter gives us Rust + Python + Java + JS/TS for free and AST hashing for clones falls out naturally |
| **`fancy-regex`** for architectural grouping | The standard `regex` crate doesn't support lookaround; code-maat's own test fixtures use it. fancy-regex wraps regex with a backtracking engine |
| **Rayon** + crossbeam-channel | Workload is CPU-bound batch; an async runtime would add binary bloat for no measurable gain |
| **`fishers_exact`** for change-coupling | Approximate chi-square fails at small N; exact test is methodologically defensible and the crate has zero transitive dependencies |

What we deliberately don't ship: no async runtime, no libgit2 binding, no LLM-based scoring, no web UI, **no non-git VCS support** (git-only by design — see [`docs/github-topics.md`](docs/github-topics.md) and the project memory for the rationale). See the [advanced guide](docs/advanced-usage.md#7-tool-stack-why-these-choices) for the long version.

---

## Status

Release-ready alpha. **21 analyses × 6 output formats × `codelore diff` PR-mode × 4 SARIF rules.** 202 lib tests + 200+ integration tests pass on Rust 1.96.0; clippy / rustfmt / cargo-deny all green. `v0.1.1` ships prebuilt binaries (5 targets) with SLSA L3 build provenance, a distroless OCI container at `ghcr.io/emrecdr/codelore`, the `emrecdr/codelore` Homebrew tap, and a `cargo binstall` manifest — all regenerated by `.github/workflows/release.yml` on every `v*` tag push.

**This session's deliverables** (3 sprints + GitHub tags + versioning):

| Sprint | Tasks | Commits | Net analyses / flags added |
|---|---|---|---|
| **Bugfix** | 7 / 7 | 7 atomic | Fixed clone-coupling p-value=0, dropped empty `name` column, SARIF CODELORE-MISSING-COCHANGE rule, canonical Options serialization (cache + provenance), AI-attribution for 2024-2026 coders, worktree prune on diff startup, deterministic tertiary sorts |
| **Modernization** | 10 / 11 (E.3 deferred) | 9 atomic | Hot-path indexes, severity-band SARIF level, `main-dev` → `main-author` header, diff CLI typed enums, absence-threshold knobs, `.codelorebots` extension hook, 11 analyses migrated to bind parameters, change_type CHECK constraint, code_health Fisher-filtered centrality |
| **Code-maat parity** | **11 / 11 — feature complete** | 12 atomic | 7 new analyses + 9 wired CLI flags + architectural grouping with lookaround + `--time-bucket DAY/WEEK/MONTH` + `--code-maat-compat` migration helper + `--strict-grouping` |
| **Docs + tags + versioning** | misc | 4 atomic | Topic badges (18 tags), SemVer policy + release procedure, RELEASING.md, github-topics.md |

**31 atomic commits total this session.** Tests went from 349 → 395 (+46 regression tests added).

Known limitations (the honest list, validated against the current codebase):

- **Complexity metrics aren't re-aggregated after grouping** — `hotspots` + `code-health` analyses report 0 cognitive for `--group-file`-collapsed entities (group-level cognitive aggregation is a post-`0.1.0` follow-up)
- **Code-maat sliding-window `--temporal-period N`** is intentionally **not** emulated under `--code-maat-compat` — the modern `--time-bucket DAY|WEEK|MONTH` (non-overlapping buckets, no commit-duplication artifact) is the recommended surface and what ships; the legacy sliding-window-with-duplication is an opt-in future-work item if migration users hit it

Full backlog: [`docs/roadmap-v1.x-and-beyond.md`](docs/roadmap-v1.x-and-beyond.md).

---

## Documentation

| If you want… | Read |
|---|---|
| All 21 analyses + every flag + CI patterns + troubleshooting | [`docs/advanced-usage.md`](docs/advanced-usage.md) |
| The architecture overview (workspace shape, pipeline data flow, threading model) | [`docs/codebase_analysis.md`](docs/codebase_analysis.md) |
| The full design specification (~1100 lines) | [`docs/superpowers/specs/2026-06-06-codelore-design.md`](docs/superpowers/specs/2026-06-06-codelore-design.md) |
| The prioritized roadmap (near-term and long-term backlog) | [`docs/roadmap-v1.x-and-beyond.md`](docs/roadmap-v1.x-and-beyond.md) |
| Release-blocker performance numbers | [`docs/perf-evidence-v1.md`](docs/perf-evidence-v1.md) |
| The release procedure + SemVer policy | [`docs/RELEASING.md`](docs/RELEASING.md) |
| GitHub topic tags (canonical set + `gh repo edit` command) | [`docs/github-topics.md`](docs/github-topics.md) |
| Drop-in CI integration templates | [`examples/`](examples/) |
| The version-by-version release log | [`CHANGELOG.md`](CHANGELOG.md) |
| Every implementation plan, executed task-by-task | [`docs/superpowers/plans/`](docs/superpowers/plans/) |

---

## Migrating from code-maat

CodeLore is a **drop-in successor**. Every published code-maat analysis works under the same `--analysis NAME`:

```bash
# code-maat (Clojure, JVM, log-file based)
java -jar code-maat.jar -l logfile.log -c git -a coupling

# CodeLore (Rust, native, direct git read — no log preprocessing)
codelore analyze --analysis coupling --repo /path/to/repo
```

The output schemas mostly match exactly. Two deliberate divergences:

1. **Honest column headers.** Code-maat's `main-dev-by-revs` emits `added` / `total-added` columns containing revision counts (lying labels). CodeLore emits `revisions` / `total-revisions`.
2. **Deterministic tiebreaks.** Where code-maat's tie-breaking is arbitrary (`first (reverse (sort-by ...))`), CodeLore adds a secondary sort on canonical author name. Reproducible output across runs.

The planned `--code-maat-compat` flag will restore both for users with dashboards parsing code-maat CSV verbatim — queued for the next sprint.

---

## Why "CodeLore"?

The technical category is "behavioral code analysis." The metaphor is reading the **legends** a codebase tells about itself.

Every commit is tribal lore: who knew this code, who burned themselves on it, where the workarounds calcified, which functions are quietly cloned across a dozen files because everyone fixed the same bug in their corner. The word *lore* captures that human narrative more honestly than "metrics" or "telemetry" — it acknowledges that the most important signal about your codebase isn't in the code, it's in the people who wrote it.

CodeLore surfaces that lore as data you can act on, without pretending the methodology is more scientific than it is. Every formula is published, every threshold is documented, and every run leaves a provenance receipt. Read the lore. Act on what it tells you.

---

## License

**GPL-3.0-only.** Bundles a vendored fork of Mozilla's `rust-code-analysis` under **MPL-2.0** — see [`crates/codelore-rca/LICENSE-MPL`](crates/codelore-rca/LICENSE-MPL) and [`crates/codelore-rca/UPSTREAM.md`](crates/codelore-rca/UPSTREAM.md) for vendoring history.

---

## Acknowledgments

CodeLore stands on the shoulders of:
- **Adam Tornhill** for [code-maat](https://github.com/adamtornhill/code-maat) and the books *Your Code as a Crime Scene* and *Software Design X-Rays* — every behavioral analysis we ship was named, validated, or hinted at in his work.
- The **gitoxide** team for proving pure-Rust git reads can outperform libgit2.
- **DuckDB Labs** for shipping an embeddable columnar SQL engine that just works.
- The **tree-sitter** project + Mozilla's `rust-code-analysis` for cross-language AST parsing.
