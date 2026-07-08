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
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue?style=flat-square)](LICENSE)
[![Rust 1.96+](https://img.shields.io/badge/rust-1.96%2B-dea584?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Clippy: clean](https://img.shields.io/badge/clippy-clean-2ea44f?style=flat-square&logo=rust)](#status)

> **Read the lore of your codebase.**

Behind every codebase is a human narrative your linter cannot see: who wrote this, who still understands it, which corners hide tribal knowledge nobody's written down, and where the historical scars are buried. Every commit is a piece of this **lore**.

**CodeLore** mines your repository's git history and projects it into **52 behavioral analyses** — hotspots, change-coupling, ownership maps, knowledge fragmentation, code health scores, copy-paste clones, live clones (clones × Fisher-significant co-change), Leiden community detection on the coupling graph, per-file centrality, knowledge-island bus-factor risk, code familiarity, team composition, coordination needs, marginal-owner risk, god-class detection, layered-architecture rule validation, modularity-violation detection (co-change without imports), unstable-interface detection, per-module bus factor, pair-programming detection, stale-code surfacing, refactoring-targets ROI ranking, and more — surfaced as SARIF for your existing CI dashboard. The socio-technical signal your linter cannot see, with the methodological honesty your team can audit.

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
- **📋 Behavioral SARIF.** Findings land natively in **SARIF 2.1.0** with three rules — `CODELORE-HOTSPOT`, `CODELORE-CLONE`, and `CODELORE-LIVE-CLONE`. Drop them straight into GitHub Code Scanning, GitLab security dashboards, or Defectdojo and alerts appear inline on pull requests.
- **🔍 Transparency over opaque ML.** CodeScene's hotspot ranking is a closed ML model. CodeLore ranks with a **published deterministic formula**: `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (100 − code_health) / 4`. Every input is emitted alongside the score; anyone can reproduce it.
- **🧾 Provenance manifest.** Every run emits a `.provenance.json` sidecar recording every config knob (auto-derived via canonical Options serialization — adding a new field auto-propagates), version pin, and timestamp. Reproducibility receipt for the run; eliminates the "we got different numbers because we silently used different thresholds" failure mode.
- **💾 SQL-queryable fact store.** No proprietary format lock-in. Export the full DuckDB store as Parquet or SQLite and query your git history as a database from the command line.
- **⚡ Persistent cache.** Second invocation on the same `(repo, HEAD, options)` opens read-only in ~10 ms instead of re-walking history — typically a 10-100× speedup on the dev inner loop depending on repo size, and the foundation of the `codelore diff` PR-mode subcommand.
- **🔗 Drop-in code-maat compatibility.** Every published code-maat analysis is supported under the same `--analysis NAME`. The `--code-maat-compat` flag flips internal defaults (`min-revs` pivot, CSV column headers for `summary` / `code-age` / `communication` / `ownership` / `authors`, `--min-soc` overload) back to legacy semantics for users with dashboards that parse code-maat CSV verbatim — see the [migration table](#migrating-from-code-maat) below.

---

## The analyses

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
| `authors` | per-file count of distinct authors (humans / bots / AI broken out) | Bird et al. 2011 defect-risk indicator |
| `top-committers` | per-author leaderboard (commits, LoC, first/last commit, bot flag) | Release notes / contributor recognition |
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
| `hotspots` ★ | files ranked by `percentile_rank(revs) × percentile_rank(cognitive) × (100 − code_health) / 4` | Published formula transparency; CodeScene-equivalent signal |
| `hotspot-velocity` ★ | files *accelerating* in churn — recent vs baseline change rate | Early warning: a file becoming a hotspot before its all-time count shows it |
| `code-health` ★ | biomarker composite score 0..100 per file: `100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv)`; `structural_risk` is a weighted sum of five biomarkers (Complex Method 0.30, God Class 0.25, Large Method 0.15, DRY 0.15, Shotgun Surgery 0.15); each row carries a `band` (red ≥ 0.55 / yellow ≥ 0.28 / green) and per-language `percentile` of structural risk | Multi-dimensional file-quality score with explicit biomarker breakdown |
| `clones` ★ | Type 1 + Type 2 clone families via AST structural hashing | Function-level copy-paste detection across Rust/Python/Java/JS/TS |
| `clone-coupling` ★ | clones intersected with Fisher-significant co-change | **The strategic differentiator** — separates live debt from dead noise |
| `knowledge-islands` ★ | per-file bus-factor risk from departed primary authors | Auto-detects knowledge loss vs CodeScene's required manual Ex-Developer marking |
| `code-familiarity` ★ | SLOC-weighted mean team familiarity + knowledge-islands ratio | One-number coverage question: "What fraction of this codebase is actively understood?" |
| `team-composition` ★ | active-author count and commit share per tenure bucket (onboarded / experienced / veteran) | Onboarding throughput + veteran over-concentration at a glance |
| `coordination-needs` ★ | per-file authorship fragmentation × interleave × co-change entropy, tiered single→high | Strongest predictors of merge friction and review delays |
| `marginal-owner-risk` ★ | ownership concentration × code-health: files where the most knowledgeable active author holds a low share | Predicts where the on-call person has the least context when a red-band file breaks |
| `centrality` ★ | per-file degree / PageRank on the Fisher-significant coupling graph | Network-centrality lens on behavioural coupling (Newman 2010 §7) |
| `communities` ★ | Leiden algorithm partitions on the coupling graph | Conway's-law cluster auto-detection (Traag, Waltman, van Eck 2019) |
| `god-classes` ★ | files combining high cognitive × fan-in × fan-out | Brown et al. 1998 AntiPatterns §3.1 — surfaces files where every dimension pulls up |
| `architecture-violations` ★ | imports crossing forbidden layer boundaries per `.codelore-arch-rules.toml` | Layered-architecture enforcement at CI time |
| `dependency-cycles` ★ | strongly-connected components (tangles) of the import graph | Tarjan SCC — the cyclic-dependency smell a DSM shows as a red diagonal block (Fontana et al. 2017) |
| `architecture-roles` ★ | per-file Core / Shared / Control / Periphery from import-graph reachability | Baldwin & MacCormack "hidden structure" — maps the architecture + per-file blast radius (propagation cost) |
| `instability` ★ | per-file afferent/efferent coupling + Instability `I = Ce/(Ca+Ce)` | Robert C. Martin's package metrics — find unstable files that too much depends on |
| `architecture-metrics` ★ | repo-level propagation cost, Lakos ACD/NCCD, cycle count, architecture type | One trendable structural-health summary (Lakos 1996; MacCormack/Baldwin) |
| `architecture-trend` ★ | propagation cost / cycles / largest tangle recomputed at sampled historical revs | Is the architecture decaying, and when did it start? Structure × history over *time* |
| `cycle-origins` ★ | bisects history to find the commit each HEAD dependency cycle first formed | Commit-level archaeology: "this tangle started here" — actionable blame for cycles |
| `modularity-violations` ★ | co-change pairs (Fisher-significant) with **no** import edge between them | The structure×history fusion — implicit cross-module dependencies (Mo, Cai & Kazman 2015 *Hotspot Patterns*) no import-only graph can see |
| `unstable-interface` ★ | widely-imported files that change often AND drag their dependents | DV8 "Unstable Interface" — instability that propagates through the dependency graph |
| `crossing` ★ | structural "X" files (high fan-in AND fan-out) that co-change in both directions | DV8 "Crossing" — couples upstream and downstream through itself; hardest shape to change safely |
| `stale-code` ★ | files alive at HEAD untouched ≥12 months AND low cognitive | The intersection minimises false-positive deletion candidates |
| `pair-programming` ★ | per-pair commit count from `Co-Authored-By:` trailers | Surfaces who pair-programs with whom across the project |
| `lead-time` ★ | per-commit author-date → committer-date delta (DORA metric) | In-flight review time without GitHub PR metadata |
| `bus-factor` ★ | per-module Filatov 2010 bus factor | Lifts CodeScene's file-level "Key Personnel" to actionable module-level granularity |
| `delivery-friction` ★ | composite of `percent_rank(revs) × percent_rank(median lead-time) × percent_rank(cognitive)` per file | Counters CodeScene v7.4's Delivery Analysis surface; lights up only files elevated on all three axes (churn × review-time × complexity), with p95 lead-time + WIP-age side columns |
| `delivery-metrics` ★ | p50/p75/p90 distributions of five flow-metric proxies (batch size, branch duration, rework %, lead-time proxy); requires `--include-merges`; each row carries a plain-text caveat | Git-only flow-metric sampling; drives the Delivery factor tile on the SPA dashboard. Not a DORA replacement |
| `release-cadence` ★ | per-release-tag inter-release gap (days) plus a `__summary__` row with median, IQR, OLS trend (`accelerating` / `stable` / `slowing`); tags filtered by `--release-tag-glob` (default `v*`) | Release-velocity monitoring without a deployment system; drives the cadence number on the Delivery factor tile |
| `effort-exposure` ★ | per-band (red/yellow/green) breakdown of commit share and LOC share over the trailing window | Answers whether engineering effort is concentrated in healthy or unhealthy code; drives the effort-exposure share bars on the SPA dashboard |
| `health-trend` ★ | code-health score series per file at sampled historical revisions; feeds the health-trend sparklines and improvements feed on the SPA dashboard | Distinguishes files that are genuinely improving from those that briefly recovered before deteriorating again |
| `function-xray` ★ | per-function hunk-overlap attribution: counts revisions where at least one diff hunk overlaps the function's line span at `--target <path>`; requires `--target` | Gall et al. ICSM 2003 HistoryFinder — per-function change-frequency leaderboard with LOC, cyclomatic, and cognitive complexity; more precise than file-level churn |
| `refactoring-targets` ★ | files ranked by refactoring ROI: `priority = (structural_risk × hotspot_score) / max(loc, 25)`; each row annotated with `dominant_type` (highest-intensity biomarker) and `manual_up_rank` (ascending-size ManualUp baseline) | Effort-aware Popt/PofB20-style ranking — a small, dense, churning, unhealthy file outranks a large one with the same raw risk |

### CLI subcommands

In addition to `codelore analyze` and `codelore diff`, the CLI exposes:

```bash
codelore explain <metric>           # formula + citation + SQL source for any metric
codelore check                      # quality-gate validation against .codelore-thresholds.toml
codelore check --diff base..head    # PR-mode quality gate
codelore mcp --repo <path>          # MCP server over stdio for AI agent integration
codelore profile                    # operational telemetry (version, schema, deps, cache root)
codelore docs                       # markdown analysis catalogue
codelore notes <base>..<head>       # release-notes markdown summary
codelore completions <shell>        # bash | zsh | fish | powershell | elvish
codelore schema <row-type>          # JSON Schema 2020-12 emit
```

`codelore check` writes `result=pass|fail` + `violations=N` to `$GITHUB_OUTPUT` when the env var is set — direct GitHub Actions step-output integration. It also works as a git hook: `codelore check --repo . --quiet` exits 0/1 and suppresses per-violation noise for hook scripts. See [§11.8 of the advanced-usage guide](docs/advanced-usage.md) for a ready-to-paste `.git/hooks/pre-push` script, exit-code contract, warm-cache performance notes, and `--ratchet` + `--history` in hook workflows.

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

# From source WITH the optional interactive dashboard emitter
# (`--format spa` — Apache ECharts + d3-hierarchy fetched once at
# build time, SHA-pinned). Requires internet on first build:
cargo install --git https://github.com/emrecdr/codelore codelore-cli --features spa
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
 codelore                                 gix · duckdb
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

Once you've run those four, you have enough signal to triage. From here, [the advanced guide](docs/advanced-usage.md) covers all analyses, every flag, configuration, CI integration, and tool-stack rationale.

---

## Interactive dashboard (`--format spa`)

Emit a single self-contained HTML file you can open in any browser,
share via Slack, attach to a CI run, or install as a PWA. Runs
fully offline — no server, no CDN, no JavaScript bundles to host.

```bash
codelore analyze --format spa --output codelore.html --repo .
```

### What's inside

Fifteen interactive widgets driven by a single embedded JSON blob:

- **Codebase at a glance** — KPIs: files, commits, contributors, median code-health, complexity peaks, MI band breakdown
- **Knowledge islands** — files whose primary author has departed and where no other substantial owner exists (no manual ex-developer marking required)
- **Hotspots circle-pack** — files sized by churn, coloured by default as a **bivariate health×activity map** (the unhealthy-and-churning danger quadrant reads at a glance), with seven single-signal lenses one tab away: complexity, code health, tech-debt friction, knowledge map, AI attribution, clones, and knowledge loss under your off-boarding scenario
- **Hotspots table + treemap** — same data, sortable / filterable / strict-area comparison
- **Trends** — monthly revision counts for the top-N hotspots
- **Multi-metric comparison** — parallel coordinates across five behavioural axes, drag-filter on any axis
- **Delivery risk** — per-commit risk (Kamei JIT-SDP) for the last N commits, with the dominant risk dimension annotated
- **Module coupling + Architecture graph** — change-coupling chord (modules coloured by top-level cluster) vs. resolved-import force-graph (agreement = healthy modularity; disagreement = signal worth investigating)
- **Change coupling sankey** — Fisher-significant file pairs
- **Cognitive distribution** — boxplot across every file with measured complexity
- **Function X-Ray** — function-level cognitive complexity sunburst
- **Commit activity** — GitHub-style calendar heatmap
- **File detail drawer** — click any file anywhere to slide in a tabbed side panel (Overview / Coupling / People / Health / X-Ray) with its full profile; the X-Ray tab shows a per-function change-frequency leaderboard for the top-10 hotspot paths
- **Factor header + guided tour** — four composite KPI tiles (Code / Architecture / Knowledge / Delivery) with XmR attention badges that fire only on statistically unlikely trends; a four-step guided tour walks each lens in sequence before handing off to free-form exploration

### What you can do with it

- **Tune every chart in place.** Depth selectors on the coupling chord / arch graph / sankey; Top-N tabs on Trends / Multi-metric / Delivery risk. Selections survive reload and sync to the URL hash so a pasted link reproduces the exact view your teammate is looking at.
- **Simulate off-boarding.** Tick departing authors → the hotspot circle-pack recolours, the hotspot table flags affected rows, and the file drawer flags every coupling partner and contributor owned by a departing author. See the bus-factor exposure before it becomes an incident.
- **Click any file.** The right-side drawer slides in with its full profile organised into **Overview / Coupling / People** tabs — a 6-axis behavioural radar, hotspot metrics, knowledge-island data, top contributors with `+added/-deleted` LoC, coupling partners + their authors, top complex functions with line numbers, and clone-group membership. Non-modal — click another row and the content swaps in place.
- **Linked brushing everywhere.** Select a file in *any* view — the hotspot map, table, coupling sankey, architecture DSM, trends, or multi-metric plot — and it lights up across all of them at once (and is announced to screen readers). Click a cell in the health×activity legend to brush every file in that quadrant. One shared focus; highlight, not hide.
- **See what's coupled to what.** Selecting a file on the hotspot map outlines it and its change-coupling partners in blue and lists those partners (with co-change %) in its tooltip — so implicit "always changed together" relationships across subsystems are legible at a glance, not just implied by arcs.
- **Self-explanatory.** Every widget has a *Learn more* disclosure: what it measures, how to read it, what signal to watch for, recommended action, and citation. Hover the `?` icons on tabs for one-line explanations.
- **Fullscreen any panel.** Top-right corner toggle. The hotspots and architecture graphs also have wheel-zoom + drag-pan and a reset button.
- **Share the exact view.** URL hash carries depth, Top-N, and off-boarding picks. Works on air-gapped CI artefacts too — the link is just a fragment in a file URL.

### Differentiation

Signals CodeScene doesn't expose: **auto-detected knowledge
islands** (no manual ex-developer marking), **AI-attribution
filtering** as a hotspot colour mode, **clone-detection overlay**
on the same hotspot view, and **auditable per-metric formulas**
(every number links back to the SQL query that produced it via
the provenance sidecar).

The `spa` Cargo feature gates the visualisation deps. Pre-built
binaries (Homebrew, ghcr, GitHub Releases) enable it, so the
dashboard works out of the box.

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

## Agent integration

`codelore mcp --repo <path>` starts a Model Context Protocol server over stdio, giving AI agents (Claude, Cursor, and any MCP-compatible client) direct access to behavioral code analysis — no account, no network, fully local.

**Client config** (add to your `claude_desktop_config.json` or Cursor `mcp.json`):

```json
{
  "mcpServers": {
    "codelore": {
      "command": "codelore",
      "args": ["mcp", "--repo", "/path/to/your/repo"]
    }
  }
}
```

**Available tools:**

| Tool | Purpose |
|---|---|
| `repo_overview` | Repository summary (commit count, authors, files, date range) plus active options snapshot |
| `hotspots` | Top hotspot files ranked by revision count; optional `limit` parameter |
| `code_health` | Per-file composite health scores (red / yellow / green band); optional `path` filter |
| `delta_health` | Function-level health delta between two revisions (`base`, `head` — any `git rev-parse` string) |
| `refactoring_targets` | Highest-priority refactoring candidates ranked by risk÷LOC; optional `limit` |
| `function_xray` | Per-function change-frequency and complexity for a given file `path` |
| `check_gates` | Evaluates `.codelore-thresholds.toml` quality gates at HEAD; returns verdict + violations |

**Fully local — no account, no network, no telemetry.** The server reads the repository at the path you configure and answers tool calls using the same fact store that powers the CLI. The first call on a cold cache pays the one-time ingest cost (a few seconds to a couple of minutes depending on repo size); subsequent calls in the same session use the warm cache and return in milliseconds.

See [§11.9 of the advanced-usage guide](docs/advanced-usage.md) for the full tool reference, parameter details, return shape descriptions, and troubleshooting.

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
   │                     │  complexity_metrics · clones · imports ·
   │                     │  author_aliases · provenance
   └────────┬────────────┘
            │ SQL queries (bind-parameterized) + Rust orchestrators
            ▼
   ┌─────────────────────┐
   │  Analyses            │  → 10 output formats (CSV/JSON/NDJSON/
   │                     │     SARIF/Markdown/GHA/HTML/Parquet/
   │                     │     SQLite/SPA)
   │                     │  → persistent cache (10-100× speedup)
   │                     │  → provenance.json sidecar
   │                     │
   │                     │  * SPA = single-HTML interactive
   │                     │    dashboard (opt-in `spa` feature)
   └─────────────────────┘
```

Every commit becomes a `CommitEvent` projected onto a DuckDB fact store. Each analysis is a SQL query over that store plus a thin Rust orchestrator (the historical `architecture-trend` additionally re-reads source at sampled past revisions). Outputs flow through ten format emitters. Every run is cached and audit-trail-stamped with a provenance sidecar.

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

Release-ready alpha. **Multiple analyses × 10 output formats × `codelore diff` PR-mode × `codelore check` quality gate × 4 SARIF rules.** Full test suite (`codelore-lib` unit + integration, `codelore-cli` integration, differential `GixRepo` vs `GitCliRepo` cross-walker parity, headless-browser SPA smoke) passes on Rust 1.96.0 across Linux, macOS, and Windows; `clippy -D warnings`, `rustfmt --check`, and `cargo deny check` all gate every push. Each tagged release ships prebuilt binaries for five targets (macOS arm64/x86_64, Linux arm64/x86_64-gnu, Windows x86_64-msvc), each with SLSA L3 build provenance attached, a distroless OCI container at `ghcr.io/emrecdr/codelore`, an auto-regenerated formula in the `emrecdr/codelore` Homebrew tap, and a `cargo binstall`-compatible asset layout — all produced by `.github/workflows/release.yml` on every `v*` tag push, gated by the `protect-release-tags` ruleset that requires green CI on the target commit before the tag is accepted.

**This session's deliverables** (3 sprints + GitHub tags + versioning):

| Sprint | Tasks | Commits | Net analyses / flags added |
|---|---|---|---|
| **Bugfix** | 7 / 7 | 7 atomic | Fixed clone-coupling p-value=0, dropped empty `name` column, SARIF CODELORE-MISSING-COCHANGE rule, canonical Options serialization (cache + provenance), AI-attribution for 2024-2026 coders, worktree prune on diff startup, deterministic tertiary sorts |
| **Modernization** | 10 / 11 (E.3 deferred) | 9 atomic | Hot-path indexes, severity-band SARIF level, `main-dev` → `main-author` header, diff CLI typed enums, absence-threshold knobs, `.codelorebots` extension hook, 11 analyses migrated to bind parameters, change_type CHECK constraint, code_health Fisher-filtered centrality |
| **Code-maat parity** | **11 / 11 — feature complete** | 12 atomic | 7 new analyses + 9 wired CLI flags + architectural grouping with lookaround + `--time-bucket DAY/WEEK/MONTH` + `--code-maat-compat` migration helper + `--strict-grouping` |
| **Docs + tags + versioning** | misc | 4 atomic | Topic badges (18 tags), SemVer policy + release procedure, RELEASING.md, github-topics.md |

Known limitations (the honest list, validated against the current codebase):

- **Code-maat sliding-window `--temporal-period N`** is intentionally **not** emulated under `--code-maat-compat` — the modern `--time-bucket DAY|WEEK|MONTH` (non-overlapping buckets, no commit-duplication artifact) is the recommended surface and what ships; the legacy sliding-window-with-duplication is an opt-in future-work item if migration users hit it

Full backlog: [`docs/roadmap-v1.x-and-beyond.md`](docs/roadmap-v1.x-and-beyond.md).

---

## Building & testing

CodeLore is a Cargo workspace; the toolchain is pinned in `rust-toolchain.toml`. The task runner is [`just`](https://github.com/casey/just) (`cargo install just`) — every recipe below is a thin wrapper over the exact command shown, so you can run the raw `cargo` line instead if you prefer.

```bash
just build          # cargo build --workspace
just release        # cargo build --workspace --release
just test           # cargo test --workspace --features test-support,spa   (CI's non-browser scope)
just test-browser   # headless-Chrome SPA smoke test — needs Chrome/Chromium on PATH; skips gracefully without one
just lint           # cargo clippy --workspace --all-targets --all-features -- -D warnings
just fmt-check      # cargo fmt --all --check
just deny           # cargo deny check   (licenses + advisories)
just ci             # fmt-check + lint + deny + test — the full local gate CI runs
```

Run the CLI against any repository:

```bash
just codelore -- analyze --analysis hotspots --repo /path/to/repo
```

The interactive dashboard (`--format spa`) needs the optional `spa` feature, which inlines Apache ECharts + d3-hierarchy at build time. `just codelore` does **not** enable it, so build/run the CLI with the feature explicitly:

```bash
cargo run --release -p codelore-cli --features spa -- \
    analyze --format spa --output codelore.html --repo .
# then open codelore.html in a browser
```

> **Recent macOS:** if the `spa`-feature link step fails with a deployment-target mismatch, prefix cargo commands with `MACOSX_DEPLOYMENT_TARGET=15.0`.

---

## Documentation

| If you want… | Read |
|---|---|
| All analyses + every flag + CI patterns + troubleshooting | [`docs/advanced-usage.md`](docs/advanced-usage.md) |
| The full 27-feature v0.6.x implementation plan + validation | [`docs/maximum-feature-plan.md`](docs/maximum-feature-plan.md) |
| CodeScene visual parity strategy + design decisions | [`docs/codescene-parity-plan.md`](docs/codescene-parity-plan.md) |
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

### Modern defaults vs code-maat compatibility

CodeLore's default surface reflects modern stack capabilities; code-maat compatibility is opt-in via `--code-maat-compat`. The divergences below are intentional — see the *Modernise, don't migrate* framing in `docs/reports/deep_analysis_report.md`.

| Surface | Code-maat | CodeLore default | Reachable via `--code-maat-compat` |
|---|---|---|---|
| Default `-a` | `authors` | `revisions` | Pass `-a authors` explicitly under compat to get the per-entity Bird et al. risk indicator. |
| `-a authors` columns | `[entity, n-authors, n-revs]` | `[entity, n_authors, n_humans, n_bots, n_revs, last_author, last_modified]` — exploits CodeLore's identity layers (humans / bots / AI-author classification) | ✓ — CSV writer emits the legacy three columns under compat. |
| Per-author leaderboard | Approximated via `-a author-churn` + sort | First-class `-a top-committers` with `commits / loc_added / loc_deleted / first_commit / last_commit / is_bot` | (n/a — distinct analysis; no code-maat equivalent.) |
| `code-age` columns | `[entity, age-months]` | `[entity, age_months, age_days, last_modified]` — second-precision back-test, recency triage | ✓ — CSV writer emits `entity,age-months` under compat. |
| Column casing | `n-authors`, `age-months`, `loc-added` (hyphens) | `n_authors`, `age_months`, `added` (snake_case — Rust idiom, also matches JSON/SARIF/parquet) | ✓ — compat-mode CSV writers (`summary`, `code-age`, `communication`, `ownership`, `authors`) emit code-maat's hyphenated names. |
| Tie-break | Arbitrary | Secondary sort on canonical author name; cross-run reproducibility | (n/a — modernisation, no code-maat equivalent worth restoring.) |
| Short flags | 13 single-letter flags (`-n -m -i -x -s -t -d -l -c -r`) | Long flags only (4 surviving shorts: `-a -o -g -p -e`) | (n/a — modern CLI convention; migration map below.) |

### Short-flag migration map

Modern CLI design favours long flags; CodeLore does not restore code-maat's 2013-era cryptic shorts. The one-time script rewrite:

| code-maat | CodeLore |
|---|---|
| `-l <log>` | (no analog — CodeLore reads git directly) |
| `-c <vcs>` | (no analog — CodeLore is git-only by design) |
| `-r N` | `--rows N` |
| `-n N` | `--min-revs N` |
| `-m N` | `--min-shared-revs N` |
| `-i N` | `--min-coupling N` |
| `-x N` | `--max-coupling N` |
| `-s N` | `--max-changeset-size N` |
| `-d <date>` | `--age-time-now <date>` |
| `-t N` | `--time-bucket DAY|WEEK|MONTH` (cleaner non-overlapping buckets; code-maat's sliding window is not propagated — see deep-analysis report PAR-3) |

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
