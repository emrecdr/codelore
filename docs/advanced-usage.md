# CodeLore — Advanced Usage Guide

This guide is the developer-facing reference for CodeLore. The [README](../README.md) is the 5-minute pitch; this is the 30-minute manual.

## Table of contents

1. [The analyses (what they tell you)](#1-the-analyses-what-they-tell-you)
2. [Output formats deep-dive](#2-output-formats-deep-dive)
3. [Every CLI flag explained](#3-every-cli-flag-explained)
4. [PR-mode: `codelore diff`](#4-pr-mode-codelore-diff)
5. [Configuration: `.codeloreignore` + thresholds](#5-configuration-codeloreignore--thresholds)
6. [Identity resolution (mailmap, bot filtering, AI authorship)](#6-identity-resolution-mailmap-bot-filtering-ai-authorship)
7. [Kamei change-feature vector](#7-kamei-change-feature-vector)
8. [Persistent cache mechanics](#8-persistent-cache-mechanics)
9. [Tool stack: why these choices](#9-tool-stack-why-these-choices)
10. [Performance characteristics](#10-performance-characteristics)
11. [CI/CD integration patterns](#11-cicd-integration-patterns)
12. [Troubleshooting](#12-troubleshooting)
13. [Workspace layout](#13-workspace-layout)

---

## 1. The analyses (what they tell you)

CodeLore ships **55 behavioral analyses** across four tiers. The table below is split into the code-maat-parity analyses (drop-in successors to legacy code-maat), a modern signal (`top-committers` — a first-class per-author leaderboard that code-maat approximated via `-a author-churn` + sort), modern additions marked ★ (the SARIF-backed differentiators including `hotspots`, `code-health`, `clones`, `clone-coupling`, `hotspot-velocity`, `refactoring-targets`, and `finding-hotspot-overlap`), graph-analytics analyses marked ★ (knowledge-islands + code-familiarity + team-composition + coordination-needs + marginal-owner-risk + centrality + communities), and architecture-analytics analyses marked ★★ (god-classes + architecture-violations + dependency-cycles + cycle-health + architecture-roles + instability + architecture-metrics + architecture-trend + cycle-origins + modularity-violations + unstable-interface + crossing + stale-code + pair-programming + lead-time + bus-factor + delivery-friction — `dependency-cycles` (Tarjan SCC), `architecture-roles` (Core/Shared/Control/Periphery), `instability` (Martin Ca/Ce/I) and `architecture-metrics` (Lakos ACD/NCCD + propagation cost) all run on a shared import-graph kernel; `architecture-trend` reruns that kernel at sampled historical revisions to show structural decay over time; `modularity-violations`, `unstable-interface` and `crossing` fuse the structural import graph with the temporal co-change graph (the DV8 hotspot-pattern trilogy); see `docs/maximum-feature-plan.md`).

### Code-maat parity (17) + modern signal

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `revisions` | "Which files change most often?" | `COUNT(DISTINCT rev)` per file | First-look for any unfamiliar repo |
| `summary` | "Give me the one-page snapshot" | Commits + changes + entities + authors counts | First slide of any review |
| `authors` | "Which files are touched by many authors (defect-risk indicator)?" | Per-entity distinct author count, broken out by humans / bots / AI | Bird et al. 2011 risk signal — pair with `hotspots` for triage |
| `top-committers` | "Who are the biggest contributors repo-wide?" | Per-author totals: commits, LoC added/deleted, first/last commit, bot flag | Release notes; onboarding; contributor recognition |
| `code-age` | "Which files are stale vs. recently churned?" | Months since last commit per file | Find dead code + recently-volatile areas |
| `abs-churn` | "How fast does the team add/delete code?" | Lines added/deleted/commits grouped by date | Trend dashboards |
| `author-churn` | "Who contributes how much?" | Same as `abs-churn` grouped by canonical author (post-mailmap) | Effort distribution |
| `entity-churn` | "Which files churn the most?" | Same grouped by file | Pair with `hotspots` |
| `entity-effort` | "How much effort has each entity received per author?" | Per-(entity, author) revision counts | Pair with `code-ownership` for bus-factor narratives |
| `entity-ownership` | "Who has added/deleted what in each entity?" | Per-(entity, author) `added` + `deleted` lines | Fine-grained ownership beyond fractal value |
| `communication` | "Who works on the same code as whom?" (Conway's Law) | Author pairs by shared-work intensity | Team topology insight |
| `ownership` (aliases: `code-ownership`, `fragmentation`) | "Is each file mainly owned by one person, or fragmented?" | Fractal Value = 1 − Herfindahl-Hirschman Index + main-developer | Bus-factor; knowledge-loss risk. `fragmentation` is code-maat's name for the fractal-value-only subset |
| `main-dev` | "Who is the main developer of each entity by lines added?" | Per-entity author with max `added` (default metric) | Knowledge-owner discovery |
| `main-dev-by-revs` | "Who is the main developer by revision count?" | Per-entity author with max revision count | Use when added-lines is misleading (e.g., reformatters) |
| `main-dev-by-deletions` (alias: `refactoring-main-dev`) | "Who is the main refactorer of each entity?" | Per-entity author with max `deleted` lines | Spot quiet refactor leaders |
| `change-coupling` | "Which files always change together?" | Fisher exact-filtered logical (temporal) coupling at `p < 0.05` | Hidden architectural debt |
| `soc` | "Sum of Coupling — how central is each file in the change-coupling graph?" | Σ(N−1) over each commit of size N the file appears in | Find systemic-coupling hubs (high `SoC`) |
| `messages` | "Which entities co-occur with commits matching this message regex?" | Server-side `regexp_matches(message, --expression-to-match)` join with `changes` | Bug-fix density, label-driven hotspots |

### Modern additions (6 ★)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `hotspots` ★ | "Which files are both complex AND change a lot?" | `percentile_rank(revs) × percentile_rank(cognitive) × (100 − code_health) / 4` — `code_health` here is the inline cognitive-only proxy `100 × (1 − 0.40 · normalize(cognitive))` ∈ [60, 100], so the unscaled product caps at 40; dividing by 4 maps output to [0, 10] ([see design spec](superpowers/specs/2026-06-06-codelore-design.md)) | The headline ranking signal — refactor priorities |
| `code-health` ★ | "How healthy is each file's structure?" | Biomarker composite: `100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv)`; `structural_risk` = weighted sum of eight biomarkers — Complex Method (0.22), God Class (0.18), Large Method (0.12), DRY (0.12), Shotgun Surgery (0.12), Deep Nesting (0.10), Many Args (0.07), Complex Conditional (0.07); each intensity is a per-language `PERCENT_RANK`; score ∈ [0, 100] (higher = healthier); each row carries a `band` (red ≥ 0.55 / yellow ≥ 0.28 / green) and per-language `percentile` of `structural_risk` | Multi-dimensional file-quality score with explicit biomarker breakdown; used as the composite gate in `codelore check code_health_min` |
| `clones` ★ | "Where is code copy-pasted?" | Type 1 + Type 2 via AST structural hashing on tree-sitter | Refactoring candidates |
| `clone-coupling` ★ | "Which copy-pasted blocks ALSO change together?" (the strategic differentiator) | Clones JOIN coupling, Fisher-significant only | Live debt that hurts you on every change |
| `hotspot-velocity` ★ | "Which files are *accelerating* in churn?" | Recent vs baseline change rate | Early warning: a file becoming a hotspot before its all-time count shows it |
| `refactoring-targets` ★ | "Where should I refactor first for maximum ROI?" | `priority = (structural_risk × hotspot_score) / max(loc, 25)`; rows carry `dominant_type` (highest-intensity biomarker) and `manual_up_rank` (ascending-size ManualUp baseline) | Effort-aware Popt/PofB20-style ranking — a small, dense, churning, unhealthy file outranks a large one with the same raw risk |

### Graph-analytics tier (7 ★)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `knowledge-islands` ★ | "Which files are at risk because their primary author is gone?" | Bird et al. 2011 + departed-author detection | Bus-factor risk surfaced automatically (no manual ex-developer marking) |
| `code-familiarity` ★ | "What fraction of this codebase is actively understood?" | SLOC-weighted decayed knowledge share; islands = files with no active ≥80%-owner | One-number knowledge-coverage question for the whole repo |
| `team-composition` ★ | "How is commit activity distributed across tenure buckets?" | Contribution-span tenure classification (onboarded / experienced / veteran) with behavioral breadth gate | Onboarding throughput + veteran over-concentration at a glance |
| `coordination-needs` ★ | "Which files generate the most coordination overhead?" | Fragmentation (HHI complement) × interleave (LAG-window author switches) × co-change entropy (Shannon, window-scoped) | Strongest predictors of merge friction and review delays |
| `marginal-owner-risk` ★ | "Where does the most knowledgeable active author have the least context?" | Max decayed knowledge share among active authors, per yellow/red-band file | Predicts where the on-call person has the least context when a file breaks |
| `centrality` ★ | "Which files are most central in the coupling graph?" | Degree / weighted-degree / PageRank on the Fisher-significant coupling graph | Network-centrality lens (Newman 2010 §7) |
| `communities` ★ | "What are the actual Conway's-law clusters?" | Leiden algorithm (Traag, Waltman, van Eck 2019) on the coupling graph | Auto-detect socio-technical modules |

### Architecture-analytics tier (★★)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `god-classes` ★★ | "Which files are gnarly AND coupled AND depended-upon?" | `(cognitive / 100) × (fan_in + fan_out)` (Brown et al. 1998 AntiPatterns §3.1) | Pick refactor targets that hit every dimension |
| `architecture-violations` ★★ | "Are layer boundaries respected?" | Imports crossing forbidden boundaries per `.codelore-arch-rules.toml` | CI gate for layered architecture |
| `dependency-cycles` ★★ | "Where are my import tangles?" | Strongly-connected components (size ≥ 2) of the import graph via Tarjan SCC (Fontana et al. 2017) | Break cycles to restore testability/replaceability; rank by tangle size |
| `cycle-health` ★★ | "Which import cycles are actively hurting, and where do I cut?" | Per-cycle `heat_pct` (members' share of window LOC churn), `live`/`fossil` verdict, trial-removal extraction candidate, predicted propagation-cost drop | Rank tangles by behavioral urgency — untangle live cycles first, starting at the suggested cut point (see [Cycle-health analysis](#cycle-health-analysis) below) |
| `architecture-roles` ★★ | "What shape is my architecture, and what's each file's blast radius?" | Core/Shared/Control/Periphery from transitive visibility fan-in/out (Baldwin & MacCormack 2014); `reach_pct = vfo/n` | Find the Core knot + the widely-reaching files; trend propagation cost over time |
| `instability` ★★ | "Which widely-used files are themselves unstable?" | Martin Ca (in-degree) / Ce (out-degree) / `I = Ce/(Ca+Ce)` (Martin 1994) | Spot Stable-Dependencies-Principle violations — high Ca + high I is dangerous |
| `architecture-metrics` ★★ | "How tangled/layered is the architecture overall?" | Propagation cost, Lakos ACD/NCCD, cycle count, architecture type (Lakos 1996; MacCormack/Baldwin) | One trendable repo-level structural-health number for CI |
| `modularity-violations` ★★ | "Which files change together but don't import each other?" | Fisher-significant co-change pairs with no import edge in either direction (Mo, Cai & Kazman 2015 *Hotspot Patterns* / DV8) | Find implicit/hidden coupling — shared globals, leaky abstractions, contracts honoured through a third party |
| `unstable-interface` ★★ | "Which interfaces are unstable enough to drag their dependents?" | `revisions × coupled_dependents`, gated on `fan_in ≥ 3` and `revisions ≥ min_revs` (DV8) | Prioritise stabilising the hubs whose churn propagates |
| `crossing` ★★ | "Which files couple their upstream and downstream together?" | Structural "X" (`fan_in ≥ 3` AND `fan_out ≥ 3`) that co-changes with both importers and imports (DV8 Crossing) | The hardest files to change safely — edits ripple both ways at once |
| `stale-code` ★★ | "Which trivial files are likely abandoned?" | Alive at HEAD AND untouched ≥12 months AND `max(cognitive) ≤ 5` | Delete-candidate surfacing (intersection minimises false positives) |
| `pair-programming` ★★ | "Who pair-programs with whom?" | `Co-Authored-By:` trailer aggregation per author pair | Team-topology / mentoring signal |
| `lead-time` ★★ | "How long does code sit before shipping?" | Per-commit author-date → committer-date delta (DORA Accelerate) | Cycle-time monitoring |
| `bus-factor` ★★ | "What's our per-module bus factor?" | Filatov 2010 — minimum N authors covering ≥80% of a module's commits | Module-level Key Personnel (CodeScene shows file-level; this is the actionable view) |
| `delivery-metrics` ★★ | "What do our batch size, rework, branch lifetime, and lead-time proxies look like?" | Percentile distributions (p50/p75/p90) of five flow metrics derived from git topology and hunk overlap; requires `--include-merges` | Git-only proxy snapshot of flow-metric distributions — run before deciding whether full DORA tooling is warranted |
| `release-cadence` ★★ | "How often do we ship, and is the pace changing?" | Inter-release tag gaps (days), median, IQR, OLS trend; tags filtered by `--release-tag-glob` (default `v*`) | Release-velocity monitoring without a deployment system; trend direction (`accelerating` / `stable` / `slowing`) at a glance |
| `architecture-trend` ★★ | "Is the architecture getting better or worse over time?" | Propagation cost / cycle count / largest tangle recomputed at sampled historical revisions (the same metrics as `architecture-metrics`, time-sliced) | Structural decay detector — see when a tangle started growing or a refactor paid off |
| `cycle-origins` ★★ | "When and where did each dependency cycle start?" | Bisects history to find the commit each HEAD dependency cycle first appeared | Commit-level archaeology: pinpoints the change that introduced a cycle so the root cause (not just the symptom) can be fixed |
| `delivery-friction` ★★ | "Which files slow down delivery most?" | Composite of `percent_rank(revs) × percent_rank(median lead-time) × percent_rank(cognitive)` per file; p95 lead-time + WIP-age side columns | Only files elevated on all three axes (churn × review-time × complexity) rank high — eliminates single-axis false positives |
| `effort-exposure` ★★ | "Are we spending engineering effort in healthy or unhealthy code?" | Per-band (red/yellow/green) breakdown of commit share and LOC share over the trailing window; drives the effort-exposure share bars on the SPA dashboard | Answers whether refactoring investment or technical-debt paydown is needed — the fraction of effort in red-band code is the key leading indicator |
| `health-trend` ★★ | "How has file-level code health changed over sampled commits?" | Code-health score series per file at sampled historical revisions; feeds the health-trend sparklines and improvements feed on the SPA dashboard | Distinguishes files that are genuinely improving from those that briefly recovered before deteriorating again |
| `function-xray` ★★ | "Which functions in a file change most often?" | Per-function hunk-overlap attribution: counts revisions where at least one diff hunk overlaps the function's line span; requires `--target <path>` | Gall et al. ICSM 2003 HistoryFinder — per-function change-frequency leaderboard with LOC, cyclomatic, and cognitive complexity; more precise than file-level churn |
| `function-coupling` ★★ | "Which function pairs in a file always change together?" | Per-function-pair co-change frequency with two-tailed Fisher exact significance; requires `--target <path>`; emits pairs with co-change count ≥ 2, sorted by p-value ascending | Adams et al. ICSM 2006 — function-level logical coupling within a file; pairs with low p-value are candidates for extract-and-share refactoring |

All analyses are pure SQL views over the DuckDB fact store + thin Rust orchestrators. You can run any analysis at any output format.

### Cycle-health analysis

`dependency-cycles` lists the members of every import tangle; `cycle-health` ranks those tangles by how much they matter *right now* and says where to start dismantling them — the structure×history fusion applied to the cyclic groups of the import graph. One row per non-trivial SCC (size ≥ 2), sorted hottest-first (`heat_pct` desc, then size desc):

- **`heat_pct`** — the cycle members' share of repo LOC churn (`loc_added + loc_deleted`) over the trailing `--window-days` window (default 90, anchored to the repo's *last commit date* so archived repos reproduce). A tangle nobody touches costs little; a hot one taxes every change that passes through it.
- **`verdict`** — `live` when at least one member appears in a window commit (a zero-LOC touch still counts as touched), `fossil` otherwise. Fossils are candidates for leaving alone; live cycles are the untangling backlog.
- **`extract_candidate`** — the member whose removal best dismantles the tangle. Each member is trial-removed and Tarjan re-run on the remnant; the candidate minimises `(largest surviving SCC, total surviving cyclic nodes)`, ties resolving to the lexicographically smallest path. This finds articulation members that a degree ranking misses.
- **`predicted_pc_drop`** — the whole-graph MacCormack propagation-cost drop if the candidate were extracted (every edge touching it removed). Both the trial-removal search and this prediction run only for cycles of **≤ 64 members**; above that bound the drop is absent (honest absence, not an estimate) and the candidate falls back to the member with the highest in-cycle degree.
- **`members_preview`** — the first three members lexicographically (`+N more` for larger tangles); full membership stays `dependency-cycles`' job.

No cycles ⇒ zero rows. Accuracy follows the import resolver's language coverage, same caveat as `dependency-cycles`. Outputs: csv, json, markdown.

### CLI subcommands beyond `analyze` + `diff`

```bash
codelore explain <metric>           # formula + citation + SQL source for any metric
codelore check                      # quality-gate validation against .codelore-thresholds.toml
codelore diff <base>..<head>        # PR-mode quality gate
codelore profile                    # operational telemetry
codelore docs                       # markdown analysis catalogue
codelore completions <shell>        # bash | zsh | fish | powershell | elvish
codelore schema <row-type>          # JSON Schema 2020-12 emit
```

`codelore check` writes `result=pass|fail` + `violations=N` to `$GITHUB_OUTPUT` when the env var is set — direct GitHub Actions step-output integration.

## 2. Output formats deep-dive

```bash
codelore analyze --analysis <NAME> --format <FORMAT>
```

| Format | Use case | Notes |
|---|---|---|
| `csv` (default) | Code-maat compatibility; pipe into other tools | snake_case headers by default; code-maat-exact hyphenated headers only under `--code-maat-compat` |
| `json` | Programmatic consumption | Pretty-printed; serde-derived |
| `markdown` | `$GITHUB_STEP_SUMMARY` in CI | GFM tables; one analysis per `# CodeLore <name>` header |
| `sarif` | GitHub Code Scanning / GitLab security / Defectdojo | SARIF 2.1.0; supported for `hotspots`, `clones`, `clone-coupling`, and `codelore diff` (CODELORE-MISSING-COCHANGE) today |
| `parquet` | DuckDB / Polars / pandas / Spark | `--output PATH` required; binary format |
| `sqlite` | Ad-hoc SQL exploration of the full fact store | `--output PATH` required; dumps 8 tables: `commits`, `changes`, `hunks`, `entities`, `complexity_metrics`, `author_aliases`, `provenance`, `clones`. |
| `spa` | Single-HTML interactive dashboard (CodeScene-equivalent surface). Opens in any browser, runs offline, fits in a CI artefact. | `--output PATH` optional (defaults to `.codelore/spa.html`); ~1.5 MB self-contained HTML. Embeds Apache ECharts + d3-hierarchy SHA-pinned at build time. Composite (multi-analysis) emitter — bypasses `--analysis`. **Opt-in `spa` Cargo feature**: default `cargo install codelore` builds offline-clean without this. Released binaries / Homebrew / ghcr ship with `spa` enabled. |
| `step-summary` | GitHub Actions `$GITHUB_STEP_SUMMARY`. Single GFM Markdown summary with KPI table, top-10 hotspots (MI band emoji), MI band breakdown (unicode bars), behavioral coupling density, knowledge islands `<details>` collapsible. | Streams to stdout by default; redirect with `>> $GITHUB_STEP_SUMMARY` in CI. Typical 2–10 KB; well under GitHub's 1 MB step-summary cap. Same composite-dashboard inputs as `--format spa` so a single ingest run can emit BOTH (run `--format step-summary` first to stdout, then `--format spa` to file). Requires the same `spa` Cargo feature as `--format spa`. |

Every file output (except SQLite, where the provenance table lives inside the DB, and SPA, where it's embedded as a JSON block in the page) emits a `{output}.provenance.json` sidecar with the bca/gix/duckdb versions, every threshold knob, mailmap state, and UTC timestamp. This is your reproducibility receipt.

### `--format spa` widget surface

The dashboard composes fifteen widgets in one HTML file, plus a tabbed click-target file detail drawer:

1. **KPI tiles** — at-a-glance summary: files analyzed, commits, distinct authors, median code health, cognitive p95, knowledge-island count, coupling pair count, coupling-graph density. Each tile has a `?` provenance tooltip linking to the formula in `docs/research-foundations.md`.
2. **Knowledge islands** (CodeLore differentiator) — ranked table of departed-primary-author files with no substantial other owner. Auto-detected from commit history + co-change intensity. CodeScene paywalls this and requires manual ex-developer marking.
3. **Hotspot circle-pack map** — the signature CodeScene view. Files sized by churn, nested by filesystem hierarchy, `d3.pack()` layout fed into an ECharts `custom` series. Defaults to a **bivariate health×activity** colour mode (each glyph encodes code-health band × development activity, so the danger quadrant reads without swapping lenses); single-signal modes (Cognitive, Code Health, Friction, Author, AI attribution, Knowledge-loss, Clones) are one tab away. Selecting a file outlines its change-coupling partners in blue and names them in the tooltip. Clicking a legend cell brushes the whole quadrant.
4. **Hotspot table + treemap** — sortable, filterable drill-down (same data, strict-area comparison). Cross-widget filter state (Alpine `$store('filter')` with `$persist`) survives reload. Click row → file detail drawer.
5. **Change-coupling sankey** + **clustered module chord** — top-N file-pair coupling flows; the chord colours each module by its top-level group. Node click → drawer.
6. **Architecture graph + DSM** — resolved-import force/layered graph and dependency-structure matrix, with the modularity-violation / unstable-interface fusion overlay. The DSM has two cell-modes behind a persisted toggle: **Structure** (the default — plain import counts) and **Fusion**, which reclassifies each above-diagonal cell by structure×history agreement against the change-coupling data already in the payload, aggregated to the same module depth as the import edges: `agree` (import + co-change, opacity graded by co-change strength), `struct-only` (import that never co-changes, dimmed), and `temporal-only` (co-change with **no** import edge — a modularity violation, drawn in the same amber as the graph's dashed violation edges). Below-diagonal back-edges stay red in both modes; every cell's tooltip names its class and a legend row spells out all four renderings in text, so the encoding is never color-only. With no coupling data, Fusion mode falls back to the structural view plus a one-line hint.
7. **Monthly trends**, **multi-metric parallel-coordinates**, **delivery-risk** (Kamei JIT-SDP), **cognitive boxplot** — behavioural distributions across the top hotspots.
8. **Calendar heatmap** — per-day commit volume, GitHub-style 52-week strip.
9. **X-Ray function sunburst** — function-level cognitive complexity drill-down, leaf colour mapped to cognitive complexity (yellow → red ramp via the same `heatmapColor` helper the circle-pack uses).
10. **Architecture-trend** — propagation-cost and cycle-count decay over the sampled revisions.
11. **Factor header** — four headline tiles (Code, Architecture, Knowledge, Delivery) above the KPI grid. Each tile shows a 0–100 composite score; an XmR attention badge fires when the trailing signal is statistically unlikely to be noise (see [Factor header and XmR attention](#factor-header-and-xmr-attention) below).
12. **Effort-exposure share bars** — banded commit-share and LOC-share bars plus a 20-dot effort strip showing the fraction of engineering activity that fell in each code-health band (red / yellow / green) over the trailing window (see [Effort-exposure analysis](#effort-exposure-analysis) below).
13. **Health improvements & regressions feed** — two clickable lists of signal-bearing band transitions (entering red / leaving red / entering green) across the top hotspot files, newest-first; clicking a row brushes the file across all widgets.
14. **Guided tour** — a four-step walkthrough over the hero circle-pack map that sets the colour lens and optional brush for each step (see [Guided tour](#guided-tour) below).

**Linked brushing:** selecting a file in any of these views highlights it across all of them at once (and announces it to screen readers); the health×activity legend also supports a set-brush. The **file detail drawer** groups its sections into Overview / Coupling / People tabs (keyboard-navigable). One shared focus; highlight, not hide.

Stack: Tailwind v4 (utility-first layout) + DaisyUI 5 (themed components; OS `prefers-color-scheme` honoured on first paint via the plugin's `--prefersdark` config) + Alpine.js 3.15 (HTML-attribute reactivity for stores + drawer + filter + selection/brush buses) + Apache ECharts + d3-hierarchy. All four vendored at build time, SHA-pinned in `build.rs`; bundle stays fully self-contained (~1.9 MB rendered SPA, no CDN at runtime).

The emitter runs every analysis each widget needs (`hotspots`, `summary`, `code_health`, `coupling`, `knowledge_islands`, `entity_ownership`, `xray`, `daily_commits`, `trends`, `effort_exposure`, `code_familiarity`, plus a clone-summary helper and a health-trend scan that populates the file health series and improvements feed) so a single `codelore analyze --format spa` invocation produces a fully populated dashboard. Coupling and knowledge-islands degrade gracefully on tiny fixtures where Fisher significance can't be reached.

### Factor header and XmR attention

The four factor tiles aggregate their respective composites into a single 0–100 headline score:

- **Code** — median `code_health` score across all live files.
- **Architecture** — `100 − propagation_cost × 100`; higher propagation cost means structurally riskier coupling. When the active calibration artifact carries repo-level corpus pools, the tile's detail line also names the propagation-cost percentile among the corpus repos (e.g. `P49 of 79 corpus repos` — see [Corpus-relative percentiles](#corpus-relative-percentiles)).
- **Knowledge** — mean familiarity score across the team (decayed knowledge share, see [Code-familiarity analysis](#code-familiarity-analysis)).
- **Delivery** — complement of the mean Kamei delivery-risk score across recent commits.

Each tile also carries an XmR attention badge that fires when the trailing weekly series shows a statistically unlikely pattern under natural process variation. The test uses Shewhart limits: mean ± 2.66 × mean(|xᵢ − xᵢ₋₁|) (the average moving-range factor for individuals charts). A badge fires when either of two conditions holds:

1. **Process limit breach** — the most recent point falls outside the upper or lower Shewhart limit.
2. **Eight-run rule** — the last eight consecutive points all lie on the same side of the mean.

No badge means "no signal yet" — it does not mean the score is healthy. A file can have a poor but stable code-health score without triggering a badge. The badge fires only when the *rate of change* crosses the statistical threshold, so small week-to-week dips that are within natural variation are intentionally suppressed. The attention criterion is evaluated over the trailing `--window-days` window (default 90 days).

### Effort-exposure analysis

`--analysis effort-exposure` answers: "Are we spending most of our engineering effort in healthy code or in code we know is unhealthy?"

Each row covers one code-health band (red / yellow / green):

| Column | Meaning |
|---|---|
| `band` | `red`, `yellow`, or `green` |
| `files` | Count of distinct files in the band at HEAD |
| `loc_share_pct` | Percentage of total SLOC that sits in this band |
| `commit_share_pct` | Percentage of commits (over the trailing `--window-days`) that touched at least one file in this band; a commit touching files across bands is counted once per band touched |
| `churn_share_pct` | Percentage of total lines changed (added + deleted) that landed in files in this band |
| `commit_share_ci_low` / `commit_share_ci_high` | Wilson 95% confidence interval on `commit_share_pct` (as a fraction in [0, 1]); reflects binomial uncertainty on the commit count |

The window anchors to the last commit date in the repository, not the wall clock. `--window-days` (default 90) controls how far back the commit and churn counts reach; LOC counts always reflect HEAD.

The SPA renders this as a pair of horizontal share bars (LOC share and churn share, coloured by band) plus a 20-dot strip where each dot represents 5 % of the effort window, coloured by the band that received the plurality of activity in that slice.

#### `max_red_effort_pct` quality gate

```toml
[gates]
max_red_effort_pct = 30.0   # fail when > 30 % of churn (changed lines) is in red-band files
```

`codelore check` evaluates this against `churn_share_pct` for the red band — the share of changed lines (added + deleted) that landed in red-band files over the trailing window. The gate fails when red-band churn share exceeds the threshold. Set it to a value that reflects your team's current baseline and tighten over time.

### Code-familiarity analysis

`--analysis code-familiarity` measures how deeply the active team understands the live codebase, using a time-decayed knowledge model (a contribution's knowledge weight halves roughly every five months of inactivity).

The analysis emits one repo-scope summary row:

| Column | Meaning |
|---|---|
| `scope` | Always `repo` — the analysis summarises the whole codebase |
| `familiarity-pct` | SLOC-weighted share of the codebase actively known by current contributors, 0–100. Active = any commit within the trailing `--window-days` (default 90), anchored to the repo's newest commit |
| `active-authors` | Contributors with activity inside the window |
| `total-authors` | All contributors holding any decayed knowledge share |
| `islands-pct` | Share of SLOC (0–100) living in files where one person holds ≥ 80 % of the knowledge with no substantial second owner |
| `verdict` | `good` when `familiarity-pct` meets the configured threshold (default 70), else `risky` |

Knowledge shares come from the decayed-contribution model: each commit grows an author's share of a file, the share decays exponentially with inactivity, reviewer trailers earn partial credit, and AI-attributed commits are down-weighted. The SPA's **Knowledge** factor tile blends `familiarity-pct` with the islands complement.

#### `code_familiarity_min` quality gate

```toml
[gates]
code_familiarity_min = 40.0   # fail when team familiarity drops below 40 % (scale 0-100)
```

`codelore check` evaluates this against `familiarity-pct`. The gate fails when the value falls below the threshold. A floor of 40 catches codebases where the active team has collectively lost touch with well over half of the code.

### Team-composition analysis

`--analysis team-composition` classifies the active author pool into three tenure buckets and shows how commit activity is distributed across them.

Tenure is measured from the author's first commit in the repository to the most recent commit date (repo-wide, not just within the analysis window). Authors with at least one commit in the trailing `--window-days` window are counted as active.

| Column | Meaning |
|---|---|
| `author` | Canonical author name (post-mailmap); `__summary__` row carries bucket-percentage breakdown |
| `tenure_days` | Days from the author's first commit in the repo to the most recent repo-wide commit date |
| `bucket` | Tenure tier: `onboarded` (< 90 days), `experienced` (90–364 days), `veteran` (≥ 365 days) |
| `veteran_breadth_ok` | Boolean; `true` when a veteran has touched a breadth of files comparable to the current 80%-core set — veterans who haven't are capped at `experienced` |
| `active` | Boolean; `true` when the author has at least one commit within the trailing `--window-days` window |
| `commits` | Total commits by this author in the repo (all time) |
| `files_touched` | Distinct files this author has ever committed to |
| `onboarding_weeks` | Weeks from first commit to entering the weekly 80%-core set; `null` for veterans, non-active authors, and founder-period authors (first commit within the project's first 12 weeks) |

The `__summary__` row carries bucket-percentage breakdowns (share of active authors and commit share per tier) rather than per-author metrics. A healthy team shows positive throughput in the `onboarded` bucket (new contributors landing commits) without `veteran` over-concentration (> 80 % of commits from one tenure tier is a bus-factor signal). The SPA Knowledge card renders the distribution as a stacked proportional bar.

### Coordination-needs analysis

`--analysis coordination-needs` identifies files where co-change patterns and authorship interleaving indicate high coordination overhead. It combines three signals:

- **Fragmentation** — how evenly distributed knowledge is across active authors (derived from the `knowledge_shares` materialized view). High fragmentation = many partial owners, none dominant.
- **Interleave** — how frequently authorship switches between consecutive commits to the same file. Computed via a LAG() window function over the commit sequence; 0.0 for files touched exclusively by one author, approaching 1.0 when authorship alternates every commit.
- **Co-change entropy** — Shannon entropy of the co-change pair distribution for the file, restricted to commits touching ≤ 30 files (removes noise from mass-refactors). High entropy = the file couples to many different files across changes.

Each file is classified into a coordination tier:

| Tier | Condition |
|---|---|
| `single` | One author holds all commits — no coordination needed |
| `low` | Multiple authors, low fragmentation and interleave |
| `medium` | Moderate fragmentation or interleave |
| `high` | High fragmentation AND high interleave — strongest signal |

The SPA Knowledge card shows the top 10 files sorted by tier then entropy. Clicking a row opens the file detail drawer.

### Marginal-owner risk analysis

`--analysis marginal-owner-risk` fuses code health with knowledge concentration to surface files where the most knowledgeable active author holds too small a share to confidently lead a regression fix.

For each file in the yellow or red code-health band, the analysis queries the maximum `k_norm` knowledge share among authors active within `--window-days`. It then applies a two-tier classification:

| Risk tier | Condition |
|---|---|
| `high` | File in red band AND top active share < 10 % |
| `elevated` | File in red band AND share < 30 %, OR file in yellow band AND share < 10 % |

Files in the green band or with sufficient concentrated ownership are excluded. The SPA file-detail drawer shows a risk chip for any file in the elevated or high tier.

The ownership × code-quality interaction is correlational: a low top-active share on an unhealthy file predicts that the person most likely to fix it has little context to work with — it does not imply the file will regress.

### Health improvements & regressions feed

The SPA's improvements feed is populated by the health-trend scan that runs as part of `--format spa`. It records signal-bearing band transitions across the top hotspot files at each sampled historical revision. Two transition types appear in the feed:

- **Regressed** — a file crossed from yellow or green into red. Signal: the file's composite code-health crossed the red threshold (structural risk ≥ 0.55).
- **Improved** — a file left red (entered yellow or green), or entered green from yellow. Signal: structural health crossed out of the at-risk zone.

Transitions that move entirely within yellow (yellow → yellow re-sampling with no band change) are filtered out; only boundary crossings appear. The feed is ordered newest-first. Clicking a row brushes the file across all widgets and opens its drawer.

The Health tab in the file detail drawer renders the full historical series as a sparkline for any file in the top-50 hotspots — each sampled revision is one data point, coloured by its band.

The **X-Ray tab** in the file detail drawer shows a per-function change-frequency table for any of the top-10 hotspot paths for which `function-xray` data was computed during the SPA build. Each row shows the function name, a proportional inline bar for change frequency (red ≥ 80 % of max, amber ≥ 40 %, grey otherwise), LOC, and cyclomatic complexity. The tab only appears when X-Ray data exists for the selected path; the Overview "Functions" sunburst (cognitive complexity) remains visible for all paths regardless. The existing `function-xray` standalone analysis (`codelore analyze --analysis function-xray --target <path>`) provides the full sorted list with last-changed date; the drawer surface is the at-a-glance leaderboard.

### Function-coupling analysis

`--analysis function-coupling --target <repo-relative-path>` reports which pairs of functions within a single file co-change statistically more often than chance. Both `--target` and `--analysis function-coupling` are required; omitting `--target` returns an error.

For each pair of functions alive at HEAD that co-changed (both touched in the same revision via hunk-overlap attribution) in ≥ 2 revisions, the analysis emits:

| Column | Meaning |
|---|---|
| `a` | First function name, deduped as `name@start-end` to handle overloads and recycled names |
| `b` | Second function name, same format |
| `co_changes` | Count of revisions where both `a` and `b` were touched |
| `a_changes` | Revisions touching `a` (regardless of `b`) |
| `b_changes` | Revisions touching `b` (regardless of `a`) |
| `confidence` | `co_changes / min(a_changes, b_changes)` — fraction of the less-changed function's history that overlaps the other |
| `p_value` | Two-tailed Fisher exact p-value; `null` when the table is degenerate (no revisions touch neither function) — `null` sorts first as the strongest coupling signal |

Rows are sorted by `p_value` ascending (`null` first), then `confidence` descending, then `a` / `b` alphabetically. Population `n` is the count of distinct revisions that touched the target file at all, so pairs that both changed in commit 1 but where commit 2 touched only one function correctly count commit 2 as a `neither` cell in the Fisher table.

Pairs with a low p-value and high confidence are the highest-priority candidates for extract-and-share refactoring — they suggest the two functions are implicitly coupled and would benefit from a shared abstraction. Research baseline: Adams et al. ICSM 2006.

Supports `csv`, `json`, and `markdown` output.

### The biomarker composite (`structural_risk`)

`code-health` scores each file's structure by combining eight structural smells into a single `structural_risk ∈ [0, 1]`, which drives the composite score `100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv)`. Each smell contributes a per-file **intensity** ∈ [0, 1] weighted as follows (weights sum to 1.0, ordered by defect-correlation strength):

| Smell | Weight | Driver |
|---|---|---|
| complex-method | 0.22 | per-file MAX cyclomatic |
| god-class | 0.18 | cognitive × (fan-in + fan-out) |
| large-method | 0.12 | per-file MAX LOC |
| dry | 0.12 | clone count |
| shotgun-surgery | 0.12 | Fisher-significant coupling-partner count |
| deep-nesting | 0.10 | per-file MAX nesting depth |
| many-args | 0.07 | per-file MAX argument count |
| complex-conditional | 0.07 | per-file MAX boolean-operator count |

The complexity-driven intensities (complex-method, large-method, god-class, dry, deep-nesting, many-args, complex-conditional) are each a per-language `PERCENT_RANK` of the file's worst value across the analyzed file set, so a smell's intensity is *relative to the rest of this repository*. When clones are excluded from a run, the DRY term drops and the remaining weights are renormalized by `/ 0.88`.

**LCOM4 (lack-of-cohesion) is not among the smells** — this is the current contract, not an oversight. CodeLore does not extract field↔method membership for any language, so no cohesion metric is computed; the composite ships the eight smells above and no cohesion term.

### Corpus-relative percentiles

The eight-smell composite above answers "how does this file rank *within this repository*." Corpus-relative percentiles answer a different question: **"how does this file's raw complexity compare to the wider world?"** Each `code-health` row carries an optional `corpus_percentile ∈ [0, 1]`.

**What the number means.** For each file, CodeLore takes the file's worst per-metric value across five raw dimensions — `cyclomatic`, `cognitive`, `sloc`, `nargs`, `max_nesting` — and looks each up in a **per-language** reference distribution, then keeps the **maximum** of the resolved per-metric percentiles. So `corpus_percentile` is *the file's worst standing on any single raw dimension versus the corpus*, not an average. The reading is a CDF: a value of `0.74` means `P(X ≤ value) = 0.74` — roughly 74% of the corpus's functions in that language sit at or below this file's worst dimension. A companion `beyond_corpus` boolean is set when the file's value exceeds the corpus maximum for a metric (percentile pins to `1.0`). Percentiles are **additive**: a run with no active reference corpus leaves every pre-existing field (`path`, `cognitive`, `score`, `structural_risk`, `percentile`, `band`) byte-identical to a run without the lens.

The percentile is `None` (absent from output) for a file when its language is unknown to the reference corpus, when that language was pooled below the trust floor (500 sampled functions), or when none of the file's metrics resolve.

**The artifact and its vintage.** The reference distribution is a *calibration artifact*: compact JSON holding, per language, a 1001-point quantile-breakpoint vector for each metric — aggregated numeric distributions only, no source code. Each artifact carries a `corpus_vintage` label recording which corpus and era it represents. CodeLore ships an **embedded world corpus** (vintage `world-2026-07-14`, pooled from permissive-license open-source projects across the five Tier-1 languages: rust, python, java, javascript, typescript) that activates the lens by default — no configuration required. Pass `--calibration <artifact.json>` on `analyze` or `check` to override the embedded corpus with a hand-built or organization-specific one. Whichever artifact the lens actually applies is stamped into the provenance manifest as `corpus_vintage`, so a report records exactly which reference it was measured against.

**Repo-level architecture percentiles.** Besides the per-function language pools, an artifact can carry a `repo_metrics` section: for `propagation_cost` and `cycle_file_share` (the fraction of the import graph's files sitting in a non-trivial dependency cycle), the sorted raw values — **one observation per corpus repo** that had a resolvable import graph. When the active artifact has this section, `architecture-metrics` appends three rows: `corpus_percentile:propagation_cost` and `corpus_percentile:cycle_file_share` (midpoint-rank percentiles of this repo's values against the pools, `0..1`) plus `corpus_n`, the number of corpus observations backing them. Read these as **"percentile among N corpus repositories"** — the base is one value per repo, so it is coarse by construction; `corpus_n` states the sample size honestly, and the lens is a rough placement, not a fine-grained calibration. The rows are additive: no active artifact, or an artifact without `repo_metrics`, leaves `architecture-metrics` output exactly as it always was. The SPA's Architecture factor tile carries the propagation-cost percentile on its detail line when present.

**Building your own corpus with `codelore calibrate`.** To compare against your own organization's code rather than the public world, build a private artifact:

```sh
codelore calibrate \
  --repos org-corpus.toml \
  --vintage acme-2026-07 \
  --output acme.calib.json
```

The manifest is TOML; each `[[repos]]` entry names a `source` (clone URL or local path), a pinned `sha`, and the advisory `languages` it contributes:

```toml
[[repos]]
source = "https://github.com/acme/service-a"
sha = "a1b2c3d4e5f6..."
languages = ["rust"]

[[repos]]
source = "/abs/path/to/local/checkout"
sha = "0f1e2d3c4b5a..."
languages = ["typescript"]
```

`calibrate` ingests each repo at its pinned SHA, pools per-function raw metrics per language (by file extension — the `languages` field is advisory, not a filter) and reduces each pool to quantile vectors, and pools the repo-level architecture metrics (`propagation_cost`, `cycle_file_share`) into the artifact's `repo_metrics` section — a repo with no resolvable import graph contributes no observation there rather than a misleading zero. A repo that fails to clone, check out, or ingest is skipped with a logged reason; the artifact's `repos_included` / `repos_attempted` record the tally. Point analysis at the result with `--calibration acme.calib.json`, and use a distinct vintage label (e.g. `acme-2026-07`) so provenance stamps stay unambiguous. Use `--cache-dir` to redirect the per-repo ingest cache to scratch storage.

`--merge <existing.json>` folds a new build into an existing artifact via sample-count-weighted quantile blending. This is an **approximation** — exact pooled re-quantiling requires retaining the raw per-function observations, which the quantile-only artifact does not. The `repo_metrics` pools, whose raw values *are* retained, merge exactly (concatenation, re-sorted). For an exact combined corpus, re-run `calibrate` over the union of both manifests instead of merging.

### The `corpus_percentile_max` gate

`codelore check` supports a `corpus_percentile_max` gate: it fails when any file's `corpus_percentile` exceeds the configured ceiling.

```toml
[gates]
corpus_percentile_max = 0.9
```

**When the gate skips (read this carefully).** The gate records a `skipped` verdict — neither pass nor fail — whenever **no code-health row resolves a corpus percentile**. That happens in more than one situation, and the honest description is: *there is no percentile data to gate on.* Concretely, the skip fires when

- no calibration artifact is active (no `--calibration` file and the embedded corpus is a not-yet-built placeholder), **or**
- an artifact *is* active, but none of the analyzed files produce a percentile — e.g. every covered language was pooled below the 500-function trust floor, every file is in a language the corpus doesn't cover, or the health scan produced no rows at all.

The stderr notice printed on skip mentions passing `--calibration`, but the underlying condition is broader than "no artifact": it is "no row carried a percentile." If you see a skip while an artifact is embedded, check that your repository's languages are covered *and* cleared the sample floor in that artifact.

### Guided tour

The guided tour steps through the hero circle-pack map in a curated sequence designed to answer one coherent question per step before handing off to free-form exploration.

**Step 1 — Code health:** sets the circle-pack colour mode to the bivariate health×activity view. Shows which files are both structurally unhealthy and actively churning. The danger quadrant (unhealthy + high activity) is the refactoring priority list.

**Step 2 — Hotspots:** switches to the Cognitive complexity colour mode. Highlights the files with the highest cognitive burden per commit. Compare with Step 1: files that appear in both views are the highest-leverage refactoring candidates.

**Step 3 — Effort in red:** switches to the Friction (tech-debt) colour mode. Paired with the effort-exposure share bars, this step answers: "How much of our recent activity is landing in high-friction code?"

**Step 4 — Refactoring targets:** returns to the bivariate health×activity view and brushes the top-10 hotspots by `hotspot_score`. Use this as a starting list for sprint planning.

After Step 4, the tour exits to free-form mode — the brush and colour mode are yours to adjust. Click any chip to jump to that step, or click Exit at any time. The tour state is not persisted across page loads.

### `--format step-summary` GitHub Actions integration

The step-summary emitter writes a GFM Markdown report sized for GitHub's `$GITHUB_STEP_SUMMARY` cap (1 MB; oversize summaries are silently dropped). Typical output is 2–10 KB. It renders into the Actions UI as a native Markdown block with KPI tables, emoji band labels, and `<details>` collapsibles — no JavaScript required (GitHub sanitizes `<script>` tags from step summaries).

Copy-pasteable workflow snippet:

```yaml
name: codelore-pr-summary
on: [pull_request]

jobs:
  analyze:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # full history for behavioural analyses

      - name: Install codelore
        run: |
          curl -fsSL https://github.com/emrecdr/codelore/releases/latest/download/codelore-x86_64-unknown-linux-gnu.tar.gz \
            | tar -xz -C /usr/local/bin codelore

      - name: Run analysis and write step summary
        run: |
          codelore analyze \
            --analysis hotspots \
            --repo . \
            --format step-summary \
            >> "$GITHUB_STEP_SUMMARY"

      # Optional: also upload the full HTML dashboard as an artefact
      - name: Build full dashboard (optional)
        run: |
          codelore analyze \
            --analysis hotspots \
            --repo . \
            --format spa \
            --output codelore-dashboard.html
      - uses: actions/upload-artifact@v4
        with:
          name: codelore-dashboard
          path: codelore-dashboard.html
```

The step summary appears at the bottom of the workflow run page in the GitHub Actions UI. The HTML artefact is a separate downloadable file with the full interactive dashboard.

### SARIF rules CodeLore ships

| Rule ID | Tags | When it fires |
|---|---|---|
| `CODELORE-HOTSPOT` | `behavioral`, `hotspot` | One result per hotspot row; `security-severity = (100 − code_health) / 4`; `level` derived from severity band (≥7 = error, ≥4 = warning, else note) |
| `CODELORE-CLONE` | `behavioral`, `clone`, `type-1`, `type-2` | One result per clone family; `security-severity = 3 + family_size`, capped at 6 |
| `CODELORE-LIVE-CLONE` | `behavioral`, `clone`, `live-clone`, `co-change`, `x-ray` | One result per `(clone_group_id, file_a, file_b)`; `security-severity = combined_score × 10` |
| `CODELORE-MISSING-COCHANGE` | `behavioral`, `coupling`, `diff` | One result per absence: a historically-Fisher-significant coupling pair where this PR touched only one side. Surfaces missing partner-file edits |

Every `codelore diff --format sarif` result carries these versioned `partialFingerprints` keys so cross-run identity stays stable and GitHub Code Scanning deduplicates alerts:

| Key | Purpose |
|---|---|
| `primaryLocationLineHash` | The key GitHub uses to deduplicate alerts across SARIF uploads (SHA-256 of repo root + path). Computed with the identical recipe `codelore check` uses, so a file flagged by both `check` and `diff` collapses to one alert. |
| `diffFinding/v1` | Stable diff-domain identity (SHA-256 of rule id, file path, and a per-finding discriminant — the diff classification, the clone's AST fingerprint, or the canonical coupling pair). Deliberately omits base/head SHAs and numeric scores so the same finding keeps its identity as the PR's commits and metrics move. |
| `couplingPair/v1` | `CODELORE-MISSING-COCHANGE` only: the alphabetically-canonical `<file_a>::<file_b>` pair, stable regardless of which side the PR touched. |

## 3. Every CLI flag explained

### `codelore analyze`

```
codelore analyze [OPTIONS]
  -a, --analysis NAME           Which analysis [default: revisions]
                                (any of the 54 above; passing an unknown
                                name prints the full valid list)
  -r, --repo PATH               Git repo path [default: .]
  -f, --format FORMAT           Output format [default: csv]
                                csv | json | ndjson | sarif | markdown | gha | html | parquet | sqlite | spa | step-summary
  -o, --output PATH             Write to file instead of stdout
      --min-revs N              Min revisions per entity [default: 5]
      --rows N                  Cap output to N rows
      --complexity-sample STRATEGY
                                head (default) | adaptive | full
                                (only `head` is wired up today; the other two parse but warn)
      --window-days N           Trailing-window length in days for activity-scoped
                                analyses (effort-exposure, team-composition, etc.)
                                [default: 90]; anchored to the repo's last commit date
      --rework-window-days N    Rework-detection window in days for `delivery-metrics`
                                [default: 21]
      --release-tag-glob GLOB   Tag-name glob for `release-cadence` [default: v*]
      --target PATH             Target file path (repo-relative) for single-file
                                analyses; required by `function-xray` and
                                `function-coupling`
      --knowledge-model MODEL   Knowledge model for `bus-factor`:
                                commits (default) | doe

  # ── Coupling-family thresholds ────────────────────────────────────
      --min-shared-revs N       Per-pair shared-commit floor [default: 5]
      --min-coupling N          Min coupling degree percentage [default: 30]
      --max-coupling N          Max coupling degree percentage [default: 100]
      --max-changeset-size N    Drop commits touching more than N files
                                (refactor-sweep filter) [default: 30]

  # ── SoC threshold ─────────────────────────────────────────────────
      --min-soc N               Minimum Sum-of-Coupling per entity for `soc`
                                analysis [default: 1]. Under
                                --code-maat-compat, --min-revs falls back to
                                the legacy "minimum SoC sum" semantic.

  # ── Messages analysis ─────────────────────────────────────────────
  -e, --expression-to-match REGEX
                                Required for `--analysis messages`.
                                Server-side `regexp_matches(message, REGEX)`
                                (RE2 flavor) joined with `changes`.

  # ── Time-bucket coupling ──────────────────────────────────────────
      --time-bucket UNIT        day | week | month
                                Modern replacement for code-maat's
                                sliding-window --temporal-period. Materializes
                                `changes_bucketed` via DuckDB
                                `date_trunc(<unit>, commit.date)` and routes
                                the coupling-family analyses through it.
                                Non-overlapping buckets — no commit-duplication
                                artifact.

  # ── Code-age cutoff ───────────────────────────────────────────────
      --age-time-now YYYY-MM-DD Override "now" for the `code-age` analysis
                                (defaults to system clock UTC; useful for
                                reproducible historical reports).

  # ── Commit walk filters ────────────────────────────────────────────
      --after YYYY-MM-DD        Only include commits authored on or after
                                this date. Applied at repo-walk time so
                                the filter survives across every analysis.
                                Mirrors `git log --after`. Honored by both
                                GixRepo (default) and GitCliRepo backends.
      --before YYYY-MM-DD       Only include commits authored on or before
                                this date. Mirrors `git log --before`.
      --include-merges          Include merge commits in coupling / churn /
                                ownership analyses. Off by default (matches
                                code-maat semantics: merges duplicate
                                authorship and inflate co-change pairs).

  # ── Architectural grouping ────────────────────────────────────────
  -g, --group-file PATH         Architectural grouping definition file with
                                full lookaround regex support (powered by
                                fancy-regex 0.14). Rewrites file paths at
                                ingest BEFORE coupling/hotspot/code-health
                                aggregation, so groups show up as first-class
                                entities. First-match-wins; plain-text LHS is
                                escaped + prefix-anchored + slash-bound;
                                explicit ^...$ regex on LHS is used as-is.

      --strict-grouping         When set, fail-fast if any change path matches
                                no group rule (default: paths with no rule are
                                kept under their original filename).
                                Auto-implied by --code-maat-compat.

  # ── Code-maat compatibility ───────────────────────────────────────
      --code-maat-compat        Migration helper for code-maat scripts. Flips:
                                  • --strict-grouping ON
                                  • `main-dev-by-revs` CSV emits legacy
                                    `added`/`total-added` headers (matches
                                    code-maat output for piped tooling)
                                  • `soc` falls back to --min-revs for its
                                    threshold (the legacy overloaded semantic)

      --exclude PATTERN         Path glob to exclude (repeatable)
      --no-cache                Skip the persistent cache; always fresh ingest
      --cache-dir PATH          Override XDG cache root
  -v, --verbose                 Verbose logging (info,codelore=debug)

  # ── Corpus calibration ────────────────────────────────────────────
      --calibration <FILE>      Corpus-calibration artifact for the code-health
                                corpus-percentile lens. Overrides the embedded
                                world corpus with a hand-built or org-specific
                                artifact (build one with `codelore calibrate`).
                                When omitted, the embedded artifact is used if
                                present; otherwise the corpus lens is absent and
                                a one-time notice is printed. Applies to both
                                `analyze` and `check`.

  # ── Team mapping ──────────────────────────────────────────────────
  -p, --team-map-file <FILE>    Optional CSV `author,team` mapping that aliases
                                author identities to logical teams in every
                                author-bearing analysis. Mirrors code-maat's
                                `-p / --team-map-file` flag; applied after
                                mailmap normalization and bot filtering. If not
                                passed, `<repo>/.codelore-teams` is auto-loaded
                                when present. Unmatched authors pass through
                                unchanged.

  # ── Ignore / lineage ──────────────────────────────────────────────
      --include-ignored         Analyse files normally excluded by `.gitignore`
                                and `.codeloreignore`. Default: respect them so
                                vendored deps, build outputs, and lockfiles don't
                                dominate hotspots. Use when analysing a vendored
                                fork or when the lockfile IS the signal.
      --no-canonical-lineage    Disable rename-aware aggregation. By default a
                                file's pre-rename history is merged onto its
                                current canonical path. Set this flag to fall back
                                to code-maat's literal-path behaviour. Implied by
                                `--code-maat-compat`.

  # ── SQL planner / debug ───────────────────────────────────────────
      --explain                 Print the DuckDB optimizer plan for the analysis's
                                underlying SQL to stderr before running the query.
                                Useful for debugging performance or verifying that
                                an index is being used.

  # ── Global ────────────────────────────────────────────────────────
      --no-banner               Suppress the pre-flight banner printed to stderr
                                at the start of every analyze run. Also
                                auto-suppressed when stderr is not a TTY.
```

### `codelore diff` (PR-mode)

```
codelore diff <RANGE> [OPTIONS]
  RANGE                     <base>..<head>     direct compare
                            <base>...<head>    three-dot: resolves via git merge-base
                            (three-dot recommended for PR mode)

  -a, --analysis KIND       hotspots | coupling | clones | all  [default: hotspots]
                            (NB: diff's `coupling` corresponds to analyze's
                            `change-coupling`; the diff subcommand uses the
                            shorter form throughout)
  -r, --repo PATH           Git repo path [default: .]
      --top-n N             Hotspot rank threshold for entrant detection [default: 10]
      --score-threshold F   Min hotspot score delta to report [default: 0.05]
      --base-cache PATH     JSON file cache for the BASE rev analysis
                            (cuts dual-analysis cost in half across PRs)
  -f, --format FORMAT       text | json | sarif | markdown [default: text]
  -o, --output PATH         Write to file instead of stdout
      --fail-on CONDITION   Exit non-zero (4) when condition fires:
                            none (default) | rank-entrant | score-increase | any
      --absence-min-shared N
                            Min historical shared-revs for a coupling-absence
                            finding to be reportable [default: 5]
      --absence-fisher-p F  Max Fisher p-value for coupling-absence finding
                            [default: 0.05]
      --min-revs N          Same as analyze [default: 5]
      --exclude PATTERN     Same as analyze (repeatable)
```

The diff subcommand emits five SARIF rule types: CODELORE-HOTSPOT (newly-entering or score-rising hotspots), CODELORE-CLONE (PR-introduced clone families), CODELORE-LIVE-CLONE (PR-introduced live-clones), CODELORE-MISSING-COCHANGE (historically-coupled partner files this PR didn't touch), and CODELORE-DELTA-HEALTH (degrading delta-health functions, one result per degrading file).

## 4. PR-mode: `codelore diff`

The form you actually deploy in CI. Three findings per range:

### Hotspot deltas

- **`rank_entrants`** — files newly entering the top-N at head. "This PR promoted `auth/login.rs` into the top-10 hotspots."
- **`score_increased`** — files in both top-N at base AND head, with `head.score − base.score ≥ --score-threshold`. "Worsened existing hotspot."
- **`pr_touched_existing`** — informational: PR-modified files that were already top-N at base. Context for the reviewer.

### Coupling absences (the CodeScene-signature signal)

Fires when a historically-strong pair (`shared >= 5 AND fisher_p < 0.05`) has **exactly one** member in the PR's changed set. "You changed `auth/login.rs` but historically `auth/session.rs` always changes with it. Did you forget?"

### Clone deltas

- **`new_families`** — clone families introduced by the PR (head fingerprints absent from base).
- **`pr_touched_existing`** — PR modified an existing clone-family member (didn't introduce new debt but didn't fix the existing kind either).

### Delta health

Every `codelore diff` run includes a `delta_health` section that judges the **change**, not the snapshot. Each function added, removed, or modified between base and head is classified by risk, given an outcome, and aggregated into a 0–100 ratio.

**Risk classification** (worst property wins):

| Condition | Risk class |
|---|---|
| LOC ≥ 71, or cyclomatic ≥ 11, or member of a clone group | `high` |
| LOC ≥ 31, or cyclomatic ≥ 6 | `medium` |
| Otherwise | `low` |

Clone membership forces `high` regardless of size — copy-pasted code carries the full structural debt of its source regardless of how small the paste is.

**Outcome per function:**

- `good` — ends `low`, strictly improves (before > after), or removes a `high`-risk function.
- `bad` — ends `high` or strictly degrades.
- `neutral` — everything else (stayed `medium`, removed a non-`high` function).

**Ratio and verdicts:** `ratio = 100 × good_weight / total_weight`, where weight is the function's physical LOC. Functions in files that were `red`-band at base carry a 1.5× weight multiplier on good and bad outcomes (the most critical files amplify the signal).

| Ratio | Verdict |
|---|---|
| < 40 | `degrading` |
| 40 – 70 | `indeterminate` |
| > 70 | `improving` |

When no changed file contains an analyzable function (docs-only, config, unsupported language), the verdict is `no-code-change` and `ratio` is omitted.

**Stale base-cache skip:** delta health is skipped when the base analysis has an empty `functions` list, a non-empty `hotspots` list, and the head analysis has functions — the data fingerprint of a base cache written by an older binary that predates function-metric collection. This prevents misreading every head function as newly added. Delete the cache file (or omit `--base-cache`) to recompute.

### Quality gate

```bash
codelore diff origin/main...HEAD --fail-on rank-entrant   # block PRs that create new hotspots
codelore diff origin/main...HEAD --fail-on score-increase # block PRs that worsen any hotspot
codelore diff origin/main...HEAD --fail-on any            # block on any finding
```

Exit 4 (the analysis-failure code) when the condition fires. Start with `--fail-on none` for a sprint to calibrate the noise floor, then raise the bar.

The thresholds file (`.codelore-thresholds.toml`, auto-discovered at the repo root) gates on structure too, not just per-file metrics:

```toml
[gates]
max_dependency_cycles = 0     # forbid any import-graph cycle repo-wide
max_propagation_cost = 0.15   # ceiling on change-reach density (0..1)
max_red_effort_pct = 30.0     # fail when > 30 % of churn (changed lines) is in red-band files
code_familiarity_min = 40.0   # fail when team familiarity drops below 40 % (scale 0-100)

[diff]
no_new_cycles = true          # a PR may not introduce a dependency cycle the base lacked
delta_health_min = 40.0       # ratio must be ≥ 40 (indeterminate or better)
deny_degrading_verdict = true # a "degrading" verdict fails the PR gate
```

`max_dependency_cycles` / `max_propagation_cost` are evaluated against HEAD by `codelore check`; `no_new_cycles` compares the base-rev and head-rev import graphs in `codelore diff` and fails the PR when head has more cycles than base. `delta_health_min` and `deny_degrading_verdict` both act on the `delta_health` section: `delta_health_min` fails when `ratio < threshold` (skipped on `no-code-change` diffs where no ratio exists); `deny_degrading_verdict` fails when the verdict is exactly `"degrading"`. `max_red_effort_pct` gates on the `effort-exposure` churn share (share of changed lines, added + deleted) for the red band; `code_familiarity_min` gates on the repo-scope `familiarity-pct` (0–100) from `code-familiarity` (see the dedicated subsections in the SPA widget surface above).

## 5. Configuration: `.codeloreignore` + thresholds

### `.codeloreignore`

Drop a file at the repo root with one glob per line. `#` comments + blank lines ignored (gitignore convention). Honored by `clones` today; rolling out to the rest of the analyses next.

```
# .codeloreignore — vendored / generated code
vendor/**
**/*_generated.rs
node_modules/**
target/**
```

### Built-in defaults

These thresholds match code-maat unless noted. Override via CLI flags (some) or the `Options` struct (all, if you call from Rust):

| Knob | Default | Source |
|---|---:|---|
| `min_revs` | 5 | code-maat parity |
| `min_shared_revs` | 5 | code-maat parity |
| `min_coupling_pct` | 30 | code-maat parity |
| `max_changeset_size` | 30 | code-maat parity |
| `fisher_significance` | 0.05 | conventional statistical-significance threshold |
| `min_clone_node_count` | 30 | ≈ 5–8 statements |
| `min_clone_shared_revs` | 3 | research brief (Fisher reliability floor) |
| `clone_similarity_floor` | 0.70 | SourcererCC BCB benchmark optimum |
| `clone_skip_same_dir` | true | drops intentional mirroring like `foo_test.rs ↔ foo.rs` |

## 6. Identity resolution (mailmap, bot filtering, AI authorship)

CodeLore's author-based analyses (`code-ownership`, `authors`, `author-churn`, `communication`) depend on resolving the *same person* across the different identities they commit under. Three layers do this work:

### 6.1 Mailmap consolidation

If a developer commits under multiple emails (`alice@oldcorp.com`, `alice@newcorp.com`, `alice.smith@personal.dev`), the repository's `.mailmap` file is the canonical place to declare them as one person. CodeLore reads `.mailmap` at the repo root and applies it before any author-based aggregation. Both name-and-email and email-only lines are supported per git's mailmap format.

Example `.mailmap`:

```
Alice Smith <alice@canonical.dev> <alice@oldcorp.com>
Alice Smith <alice@canonical.dev> <alice@newcorp.com>
Alice Smith <alice@canonical.dev> Alice S. <alice.smith@personal.dev>
```

After resolution, all three of Alice's identities count as one author in every output.

### 6.2 Bot filtering

Automated commits (dependency-bump bots, CI bots, release bots) skew Conway-style metrics — a Dependabot PR that touches 47 files isn't a human collaboration signal. Each commit is checked against a built-in substring-match list (`identity/bots.rs::DEFAULT_BOT_PATTERNS`); a match in either the author email or the author name marks the commit as a bot commit:

- `dependabot[bot]`
- `github-actions[bot]`
- `claude-code[bot]`
- `copilot[bot]`
- `renovate[bot]`
- `pre-commit-ci[bot]`

Match is plain substring containment, so `dependabot[bot]@noreply.github.com` matches `dependabot[bot]`. Bot commits still land in the fact store (so you can still query them in SQL via the SQLite/Parquet export) but they get the `ai-authored` attribution and the author-based analyses treat them as automated agents rather than human contributors.

### 6.3 AI-authorship classification

Each commit is classified into one of three buckets and stamped in the `commits.ai_attribution` column:

| Class | Trigger (in priority order) |
|---|---|
| `ai-authored` | Author or committer matches one of the bot patterns above |
| `ai-assisted` | Commit message contains `Co-Authored-By: Claude`, `Co-Authored-By: Copilot`, or `Co-Authored-By: GitHub Copilot` |
| `human` | Default — no AI signals found |

The bot list and the assisted-trailer list are intentionally narrow; tools that don't publish a standardized trailer (or that you don't want to count as AI-assisted) won't be detected. The classification is informational today — no published analysis filters by it — but every commit carries the column so you can query it directly from the SQLite/Parquet export:

```sql
SELECT ai_attribution, COUNT(*) AS n FROM commits GROUP BY 1 ORDER BY n DESC;
```

## 7. Kamei change-feature vector

Every commit ingested by CodeLore is enriched with the 14-feature change vector from [Kamei et al.'s JIT-SDP work](https://ieeexplore.ieee.org/document/6341763) (Just-In-Time Software Defect Prediction). These features describe the *shape* of each change and are written to the `commits` table, so any analysis can join against them in SQL.

| # | Feature | Description |
|---|---|---|
| 1 | `ns` | Number of modified subsystems (top-level directories) |
| 2 | `nd` | Number of modified directories |
| 3 | `nf` | Number of modified files |
| 4 | `entropy` | Shannon entropy of the per-file change distribution — high entropy = tangled change across many files |
| 5 | `la` | Lines of code added |
| 6 | `ld` | Lines of code deleted |
| 7 | `lt` | Average size of touched files at the pre-change state |
| 8 | `fix` | 1 if the commit message matches bug/fix regex patterns, else 0 |
| 9 | `ndev` | Number of distinct developers who previously modified the touched files |
| 10 | `age` | Average days since the last modification of each touched file |
| 11 | `nuc` | Number of historical commits touching the same files (their "history density") |
| 12 | `exp` | Author's lifetime commit count in the repo as of this commit |
| 13 | `rexp` | Same as `exp` but with recent commits weighted higher (exponential decay) |
| 14 | `sexp` | Author's prior commit count in the **same subsystem** as the touched files |

These features land in `commits` for every commit. The published analyses don't yet expose them directly via CLI flags — they're foundation for future bug-prediction work — but you can query them right now via `--format sqlite` or `--format parquet` and the columns are there:

```sql
SELECT rev, fix, entropy, la, ld, ndev FROM commits WHERE fix = 1 ORDER BY entropy DESC LIMIT 10;
```

This surfaces the 10 highest-entropy bug-fix commits — useful for retrospective "tangled fix" detection.

## 7.5. Delivery analyses: what they measure (and what they don't)

Three analyses form the Delivery signal family. They are **git-only proxies** — they approximate real flow metrics from commit graph topology and text overlap, without a deployment system, a ticketing system, or a CI/CD webhook. They are useful for getting a first read on delivery patterns; they are not replacements for a full DORA pipeline.

### `delivery-metrics` — flow-metric proxy distributions

```bash
codelore analyze --analysis delivery-metrics --include-merges
```

`--include-merges` is required: without merge commits there is no branch topology to measure. Outputs one row per metric, each with p50 / p75 / p90 / N and a caveat string explaining the approximation.

| Metric | What it approximates | Caveat |
|---|---|---|
| `batch_size_files` | Files changed per merge | All merge commits; no PR-draft filtering |
| `batch_size_loc` | Lines changed per merge | Same |
| `branch_duration_hours` | Time from first branch commit to merge | Detects main parents via commit-parent topology; squash/rebase workflows undercount |
| `rework_pct` | Rework: fraction of added hunks re-touched within `--rework-window-days` | Hunk-pair text overlap; line drift between commits is not tracked |
| `lead_proxy_hours` | Author-date → committer-date gap per commit | Proxy only — does not include time waiting before first review or in CI queues |

**Rework band thresholds** (from Pluralsight Flow's published benchmarks — correlational, not causal): green < 9 %, yellow 9–14 %, red ≥ 15 %.

### `release-cadence` — inter-release velocity

```bash
codelore analyze --analysis release-cadence
codelore analyze --analysis release-cadence --release-tag-glob "release/*"
```

Tags are a proxy for releases, not deployments. Outputs per-tag rows (sorted by date ascending) plus a `__summary__` row carrying: median gap in days, IQR, and a trend label.

| Trend label | Meaning |
|---|---|
| `accelerating` | OLS slope of gap series < −0.1 d/release |
| `stable` | OLS slope within ±0.1 d/release |
| `slowing` | OLS slope > +0.1 d/release |

Returns an empty result when no tags match the glob — add `--release-tag-glob '*'` to see all tags.

### `delivery-friction` — where flow friction concentrates

```bash
codelore analyze --analysis delivery-friction
```

Ranks files by a composite of historical churn, lead-time proxy, and cognitive complexity. The top rows are the files where slow flow is most concentrated. Use alongside `delivery-metrics` to decide where to invest in automation or refactoring for flow improvement.

### The Delivery factor tile in the SPA dashboard

The SPA's four-factor header includes a **Delivery tile** when any delivery data is available. Unlike the Code, Architecture, and Knowledge tiles — which collapse to a single score on a 0–100 scale — the Delivery tile deliberately shows no composite score. Instead it surfaces three raw proxies:

- **rework %** — `rework_pct` p50, band-colored by the Pluralsight rework benchmark
- **branch p75 h** — `branch_duration_hours` p75 (hours branches stay open)
- **cadence median d** — median inter-release gap in days from `release-cadence`

A composite score would imply that rework %, branch duration, and cadence can be weighted against each other with known coefficients. There is no validated weighting in the literature; showing the numbers directly is more honest.

The tile is absent when all three proxy inputs are unavailable (no merge commits, no matching tags, or `delivery-metrics` was not run).

## 8. Persistent cache mechanics

CodeLore caches the DuckDB fact store at `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`. Second invocation on the same `(repo_path, HEAD sha, options, schema_version, codelore_version)` opens read-only in ≈ 10 ms instead of re-walking history.

```bash
# Skip the cache (always fresh in-memory)
codelore analyze --analysis hotspots --no-cache

# Override the XDG root (useful in CI with per-job caches)
codelore analyze --analysis hotspots --cache-dir /tmp/codelore-cache

# Inspect the cache
ls "$(dirs -c codelore 2>/dev/null || echo $XDG_CACHE_HOME)/codelore/"
```

Eviction: 5 entries per repo + 2 GB global cap (LRU). Pruning runs after every successful miss-and-write.

**Parquet + SQLite formats bypass the cache** by design — they need a writable DuckDB connection to run `INSTALL/LOAD sqlite` and `COPY TO parquet`.

### Dirty-worktree cache hit warning

The cache key includes `head_sha` but NOT the working tree. That's correct for analyses that read only committed history (`revisions`, `coupling`, `ownership`, `churn`, `messages`, ...), but `hotspots`-style HEAD-time metrics computed by `ingest_complexity_at_head` and `populate_clones_at_head` read files from disk at ingest time. If you change files without committing and then re-run codelore, the cache hits on `head_sha` — and you get the previous run's metrics computed from the previous worktree state, not your current edits.

To surface this, codelore emits a `tracing::warn!` whenever a cache hit lands on a working tree with uncommitted modifications or untracked Tier-1 source files:

```
WARN cache hit on a working tree with uncommitted changes; HEAD-time metrics
     (hotspots' complexity, clones) may be stale relative to disk.
     Pass `--no-cache` to recompute against the current working tree.
```

Detection is cheap (gix `Repository::status` for the pure-Rust walker, `git status --porcelain` for the CLI walker). Pass `--no-cache` if the dirty state matters for your analysis. The warning is informational — codelore still serves the cached result by default to preserve the 10–100× speedup on clean repeated runs. Auto-invalidation via worktree-content hashing was considered and rejected: hashing every tracked file on every invocation costs 100ms–1s on large trees, which would erase the cache's perf win for the majority case where the cache is correct.

## 9. Tool stack: why these choices

Every dependency in CodeLore was picked for a specific reason. The short version:

| Layer | Choice | Alternative considered | Why we picked this |
|---|---|---|---|
| Git read | `gix` (gitoxide) | `git2-rs` (libgit2 binding) | Pure Rust → no LGPL question, native `Send + Sync`, no C build deps, gix-blame is more accurate |
| Fact store | DuckDB (bundled) | Polars / SQLite / custom | Columnar analytics, spill-to-disk for kernel scale, SQL surface as a power-user feature, ZERO setup |
| Parsing | tree-sitter via vendored `rust-code-analysis` | per-language hand-rolled parsers | Battle-tested, language-agnostic, AST structural hashing for clones falls out for free |
| Concurrency | Rayon + crossbeam-channel | tokio | Workload is CPU-bound batch; async runtime is overkill and would force `Send` constraints we don't want |
| Statistics | `fishers_exact` | hand-rolled chi-square | Exact test (not approximate), zero-config, methodologically defensible at small N |
| CLI | `clap` 4 (derive macros) | `argh`, `gumdrop` | Industry standard, automatic `--help`, subcommand parsing |
| Output | `serde_json` + hand-rolled CSV + `sha2`/`hex` for SARIF fingerprints | — | Standard, minimal |
| Caching | `dirs` for XDG paths + DuckDB read-only mode | rolling our own | Conform to OS conventions (works on macOS, Linux, Windows) |
| Tests | `criterion` for benches + `assert_cmd`/`predicates` for CLI | — | Standard Rust test surfaces |

### What we deliberately don't use

- **No async runtime** — workload is CPU-bound batch; an async runtime would add binary size and `Send` constraints for no measurable throughput gain.
- **No libgit2** — gix already does everything we need, and pure-Rust matters for our supply chain story.
- **No LLM** — we're transparency-first. CodeScene's ML hotspot ranking is the opposite of what we ship. (LLM-based bug-link induction is a long-horizon research item with a pluggable interface.)
- **No web UI** — explicitly out-of-scope. Power users want SQL access to the fact store and SARIF in their existing CI dashboard; both are first-class outputs.

## 10. Performance characteristics

Per `docs/perf-evidence-v1.md` (warm-cache numbers):

| Repository | Commits | Source files | Wall (warm) | Peak RSS |
|---|---:|---:|---:|---:|
| codescene (this workspace) | ~95 | 131 .rs | 0.24 s | 89 MB |
| gitoxide (shallow 2000) | 9,985 | 2,903 | 1.16 s | 75 MB |
| tokio (shallow 3000) | 4,523 | 854 | 2.09 s | 230 MB |
| Linux kernel | 1.4M | 70k | < 10 min target | < 4 GB target |

The Linux kernel row is the spec's release-blocker target; the weekly CI bench job (`.github/workflows/bench.yml`) publishes the actual measurement once the cached snapshot reaches a stable baseline.

### Why tokio uses more memory than gitoxide despite fewer commits

Tree-sitter parsing + AST traversal dominate RSS for the Tier-1 file complexity extraction pass. tokio has roughly 3.5× the Rust source-line density per commit (deep generics in the runtime internals) compared to gitoxide. The commit-walk work scales with commit count; the complexity-extraction RSS scales with the number of Tier-1 source files at HEAD.

### Parallel vs serial complexity extraction

The complexity-extraction pass uses Rayon by default (one task per source file). On the `medium_repo` fixture (25 Rust files), parallel vs serial measure within bench noise (≈ 56 ms either way) because the bottleneck is the commit walk + change-feature enrichment SQL, not the parse pass. The parallel pass beats serial measurably on codebases with hundreds of Tier-1 files. Set `RAYON_NUM_THREADS=1` in the env before invoking `codelore` to force serial mode for comparison runs.

## 11. CI/CD integration patterns

### GitHub Actions (the canonical pattern)

See [`examples/.github/workflows/codelore-pr.yml`](../examples/.github/workflows/codelore-pr.yml) for the full template. Critical configuration:

- **`fetch-depth: 0`** in `actions/checkout` is mandatory. Without full history, hotspot scores are truncated to one commit and become meaningless. This is the single most common failure mode.
- **Three-dot merge-base notation** (`origin/main...HEAD`) scopes correctly to PR-only commits even when the base branch has moved since branch creation.
- **`security-events: write` permission** is required for SARIF upload to Code Scanning.
- **GHA cache integration** — pass `--cache-dir ${{ runner.temp }}/codelore-cache` and wrap with `actions/cache@v4` to persist across PRs.

### Quality gate rollout

| Phase | `--fail-on` | What it catches | When to advance |
|---|---|---|---|
| Pilot | `none` (default) | Nothing — advisory only | After 2 sprints of green runs |
| Soft enforce | `rank-entrant` | PRs that create new top-N hotspots | After team is comfortable interpreting findings |
| Strict | `score-increase` | PRs that worsen any existing hotspot | Once your codebase has stabilised |
| Maximum | `any` | Anything (including new clones + missing co-changes) | Mature teams in active refactor |

## 11.5. Per-stage timing (`RUST_LOG=codelore::bench=info`)

CodeLore instruments the three load-bearing stages of `analyze` —
opening the repo, looking up the cache (or ingesting from scratch),
and running the analysis + emitting output — with `tracing` spans
under the `codelore::bench` target. Default WARN-level filtering
suppresses them entirely (zero overhead), but opting in produces a
breakdown without any new flag:

```bash
RUST_LOG=codelore::bench=info codelore analyze \
  --repo path/to/repo --analysis hotspots > /dev/null
```

Each stage prints a `close` event with the elapsed time:

```
INFO bench.open_repo: close time.busy=2.4ms time.idle=15µs
INFO bench.cache_or_ingest: close time.busy=187ms time.idle=24µs
INFO bench.analyze_and_emit: close time.busy=43ms time.idle=18µs
```

For finer-grained timing, raise the level:
`RUST_LOG=codelore=debug` also shows the per-analysis spans inside
`codelore-lib` (cache hit/miss, materialize_changes_bucketed,
etc.). The default `--verbose` flag enables `info` for
`codelore` but not `codelore::bench`, so the bench-specific
spans stay out of normal-verbosity output.

## 11.8. Using codelore as a local quality hook

`codelore check` is designed to run as a git hook. It exits 0 on pass and 1 on any gate violation, with no interactive prompts, making it safe to call from any hook without modification.

### `pre-push` hook (recommended)

Drop this at `.git/hooks/pre-push` and make it executable (`chmod +x .git/hooks/pre-push`):

```sh
#!/usr/bin/env sh
# Runs codelore check before every push. Blocks if any configured gate fails.
# Warm-cache runs (after the first ingest) typically complete in under a second.
set -e
codelore check --repo . --quiet
```

The `--quiet` flag suppresses diagnostic noise (per-violation detail lines, inline warnings) while keeping the final verdict line (`✅ PASS`, `❌ FAIL`, `⚠ WARNING`) on stderr. Exit codes follow the standard contract: 0 = pass, 1 = gate violations, 2 = CLI/arg error, 3 = repo error, 4 = analysis error, 5 = output/I/O error.

### Exit-code contract for hooks

| Exit code | Meaning |
|---|---|
| 0 | All gates pass (or no thresholds configured — vacuous pass) |
| 1 | One or more gate violations |
| 2 | CLI/argument error |
| 3 | Repository error (not a git repo, no HEAD, etc.) |
| 4 | Analysis error |
| 5 | Output/I/O error |

### Warm-cache performance

The first `codelore check` run on a repo ingests the full git history into a DuckDB cache file. Subsequent runs (same HEAD SHA, same options) read from the cache in milliseconds. The cache lives at `<XDG_CACHE_HOME>/codelore/<repo_hash>/` and is keyed on HEAD SHA + package version, so a push that changes HEAD re-ingests only when the cache entry is cold.

### PR-mode in hooks

For pre-push hooks that run on feature branches, `codelore diff` gives a PR-scoped delta view against the integration branch:

```sh
#!/usr/bin/env sh
set -e
# Gate + PR delta report in one hook
codelore check --repo . --quiet
codelore diff origin/main...HEAD --analysis all --format markdown --output -
```

### `--ratchet` in hooks

`--ratchet` pairs naturally with pre-push: the committed `.codelore-ratchet.toml` travels with the repo, so every developer's push is checked against the same quality baseline. When a metric improves the file is rewritten tighter; when it regresses the push is blocked with a clear summary.

```sh
#!/usr/bin/env sh
set -e
codelore check --repo . --quiet --ratchet
```

When `fail_on_degraded = false` is set in `[gates]` and a gate produces no evaluable data, the summary prints `⚠ codelore check: WARNING — N gate(s) degraded (non-degraded gates pass)` and exits 0 — the push proceeds. With the default `fail_on_degraded = true` (the recommended setting for hooks), a degraded gate blocks the push.

**Metric-sourcing coupling:** `red_effort_pct_observed` and `dependency_cycles_observed` are only populated in the ratchet when the corresponding threshold gates (`max_red_effort_pct`, `max_dependency_cycles`) are configured in `.codelore-thresholds.toml`. Without those gates, `--ratchet` tracks only `code_health_min_observed` — the initialization message names exactly which metrics are being tracked so you know what the ratchet is guarding. To ratchet effort and cycles, add the matching `[gates]` keys to your thresholds file even if you set the threshold very permissively (e.g. `max_red_effort_pct = 100.0`).

**`--quiet` with `--ratchet`:** when a ratchet regression is detected, the detail table (which metrics regressed and by how much) still prints even under `--quiet` — it is the only actionable diagnostic and is intentionally preserved.

**`--format sarif` with `--ratchet`:** `--ratchet` composes with `--format sarif`. The gate SARIF document is written to stdout on every ratchet outcome (initialize, tighten, regression), identical to a non-ratchet `check --format sarif` run; the human-readable ratchet summary is routed to stderr so stdout stays a clean SARIF document. Exit codes are unchanged — a ratchet regression still exits non-zero while emitting a valid document.

### Local run history

`codelore check --history` prints the last 20 gate-run records grouped by HEAD SHA from the per-repo ledger, giving you a local audit trail of how each gate has trended across pushes — no server required.

### SARIF output with evidence chains

`codelore check --format sarif` emits a SARIF 2.1.0 document to **stdout** while keeping the verdict lines (`✅ PASS`, `❌ FAIL`) and per-violation detail on **stderr**. Exit codes are unchanged — a gate failure is still exit 1 regardless of format.

On a pass, the document contains zero results (valid SARIF; the GitHub Code Scanning upload action handles an empty result set without error). The caller decides whether an empty result set is interesting.

#### What the evidence chain contains

For each per-file gate violation (paths that are not `(repo-wide)` or `(degraded)`), the SARIF result carries a **commit evidence chain**: the top-5 commits that most recently touched that file, newest-first (ordered by commit date, then revision, for deterministic ties). Each entry's message reads `{date} {author}: {message} (+{churn} lines)`, combining:

- The ISO date
- The canonical (mailmap-resolved) author
- The first line of the commit message, capped at 80 characters
- The churn for that path in that commit (lines added + deleted)

The chain is populated from the same lineage-aware fact store that powers the `hotspots` and `code-health` analyses, so renamed and moved files are traced through their history correctly.

`codelore diff --format sarif` carries a tighter chain of up to 3 commits per affected file — enough to identify the source of a change without overwhelming a PR review comment.

#### GitHub rendering

GitHub Code Scanning consumes both structures that carry the chain:

- **`codeFlows → threadFlows → locations`** — rendered as a "Show paths" thread in the finding detail view, letting reviewers step through the commit history that led to the violation.
- **`relatedLocations`** — shown in the "Show more" context panel of the finding.

Each result also carries two `partialFingerprints` keys:

| Key | Purpose |
|---|---|
| `gateFinding/v1` | Stable identity of this finding across check runs (SHA-256 of gate name, file path, and HEAD SHA). Changes when the HEAD SHA changes — expected. |
| `primaryLocationLineHash` | The key GitHub uses to deduplicate alerts across SARIF uploads (SHA-256 of repo root + path). Stable across HEADs as long as the violation stays on the same file. |

Both keys are versioned (`/v1`) as required by the SARIF 2.1.0 spec (§3.5.4.2).

#### GitHub Actions upload

```yaml
- name: Run quality gates
  run: codelore check --repo . --format sarif > codelore-check.sarif
  continue-on-error: true       # let the upload step always run

- name: Upload to Code Scanning
  uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: codelore-check.sarif
    category: codelore-check
```

The `continue-on-error: true` is required because `codelore check` exits 1 on violations — without it, a failing gate would skip the upload step and the SARIF document would never reach Code Scanning.

### Behavioral×static fusion (`ingest-sarif` + `finding-hotspot-overlap`)

External scanner results can be fused with CodeLore's behavioral signal to answer the question *"which external findings sit in the files we touch most often and that already carry a red code-health band?"* — the intersection of static and behavioral evidence.

#### Step 1 — ingest external findings

```bash
codelore ingest-sarif --repo . scan.sarif
codelore ingest-sarif --repo . clippy.sarif semgrep.sarif  # multiple files in one call
```

Findings are stored in a per-repo sidecar at `<cache_root>/codelore/<repo_hash>/external-findings.duckdb-ext`. The `.duckdb-ext` extension is intentional — the LRU pruner skips it, so the sidecar survives fact-store eviction.

Re-ingesting a file is **idempotent**: findings are replaced per engine, so two passes with the same file produce the same row count as one pass.

**Supported SARIF dialects:**

- **Semgrep** — fingerprints in `fingerprints.matchBasedId/v1` (not `partialFingerprints`); `%SRCROOT%` base URI without `originalUriBaseIds`; level on rule defaults, not on individual results.
- **clippy-sarif** — no fingerprints (self-hash fallback used); relative URIs without a `file://` scheme.
- **CodeQL** — `partialFingerprints.primaryLocationLineHash` guaranteed; `ruleIndex` indirection into the `rules` array; absolute `file://` URIs (stored as-is; join on repo-relative paths will not match these — pass a post-processed SARIF or strip the host prefix before ingesting).

Any other SARIF 2.1.0 producer is accepted; unparseable individual results are skipped with a warning, not a hard error.

#### Step 2 — run the overlap analysis

```bash
codelore analyze --analysis finding-hotspot-overlap --repo .
```

For each path in the sidecar the row carries:

| Column | Meaning |
|---|---|
| `findings` | Total finding count (all engines) |
| `engines` | Comma-joined engine names |
| `worst_level` | Most severe level (`error` > `warning` > `note`) |
| `hotspot_score` | From the hotspots analysis; 0.0 when absent |
| `revs_percentile` | SQL-equivalent `PERCENT_RANK` of revision count within the hotspot set; 0.0 when absent |
| `health_band` | `red` / `yellow` / `green` from code-health; `unknown` when absent |
| `priority` | `act-now` / `plan` / `note` (see below) |

**Priority rules** (first match wins):

- `act-now` — findings > 0 AND revs_percentile ≥ 0.9 AND health_band = `red`
- `plan` — revs_percentile ≥ 0.7 OR health_band = `red`
- `note` — everything else

Paths absent from the hotspot result set (new files, below `min_revs`, unsupported language) appear with `hotspot_score = 0.0` and `revs_percentile = 0.0` — the honest left-join contract.

#### Step 3 — gate on `act-now` count

```toml
# .codelore-thresholds.toml
[gates]
max_findings_in_hot_files = 0   # fail when any act-now file exists
```

The `max_findings_in_hot_files` gate fails when the number of `act-now` rows exceeds the threshold. The gate is **skipped** — not failed — when the sidecar is absent or empty, so adding the gate to an existing thresholds file is safe before running `ingest-sarif`.

## 11.9. MCP server (`codelore mcp`)

`codelore mcp --repo <path>` starts a Model Context Protocol server over stdio. AI agents connect to it and call the tools below; the server answers using the same persistent fact store the CLI uses. It is **fully local** — no account, no API key, no telemetry, no network access.

### Starting the server

```bash
codelore mcp --repo /path/to/repo
```

The server blocks and reads JSON-RPC 2.0 messages on stdin (newline-delimited), writes responses to stdout. It runs until the client closes the connection.

### Client configuration

Add an entry to your client's MCP config (exact filename varies by client):

```json
{
  "mcpServers": {
    "codelore": {
      "command": "codelore",
      "args": ["mcp", "--repo", "/absolute/path/to/your/repo"]
    }
  }
}
```

For Claude Desktop this is `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows). For Cursor it is `.cursor/mcp.json` in the project root or the global `~/.cursor/mcp.json`.

### Tool reference

#### `repo_overview`

Returns a JSON object with `summary` (commit count, unique authors, file count, first/last commit dates) and `options` (the active analysis options used for cache-keying — useful for diagnosing why two calls return different results).

Parameters: none.

Cost: warm-cache call is fast (milliseconds). Cold-cache triggers full history ingest.

#### `hotspots`

Returns the top hotspot files ranked by revision count, with composite hotspot score and complexity.

Parameters:
- `limit` *(optional, u32)* — cap the number of rows returned. Default: 20.

Cost: warm-cache fast. Cold-cache triggers ingest.

#### `code_health`

Returns per-file composite health scores: a `band` (`red` / `yellow` / `green`) and a numeric `score` (0–100). Files in the `red` band are the highest-priority health risks.

Parameters:
- `path` *(optional, string)* — filter to a single file path relative to the repo root. Omit to return all files with complexity data.

Cost: warm-cache fast. Cold-cache triggers ingest.

#### `delta_health`

Returns a function-level health delta between two revisions. Shows which functions were added, removed, or changed in LOC/complexity, and whether the overall change is `improved`, `neutral`, or `degraded`.

Parameters:
- `base` *(required, string)* — base revision. Any string accepted by `git rev-parse` (branch, tag, full SHA, `HEAD~N`).
- `head` *(required, string)* — head revision. Same format.

Both revisions are validated before any work starts — an unresolvable ref returns a tool error rather than a server crash.

Cost: **high** — ingests history twice (once per revision) using temporary git worktrees. Expect the same cost as two fresh `codelore analyze` calls on a cold cache. On a warm cache (both SHAs previously analysed), cost is lower but still involves two in-memory ingest passes.

#### `refactoring_targets`

Returns the highest-priority refactoring candidates, ranked by a risk-to-LOC ratio that combines hotspot score (frequency × recency) with code health. The files at the top of this list carry the highest maintenance burden relative to their size.

Parameters:
- `limit` *(optional, u32)* — cap the number of rows. Default: all.

Cost: warm-cache fast. Cold-cache triggers ingest.

#### `function_xray`

Returns per-function change-frequency and complexity for a specific file. Each row identifies a function by name and reports how many revisions touched it and its current cyclomatic complexity — the intersection of "changed often" and "high complexity" is the highest-value refactoring target within the file.

Parameters:
- `path` *(required, string)* — file path relative to the repo root (e.g. `src/main.rs`).

Cost: warm-cache fast. Cold-cache triggers ingest.

#### `check_gates`

Evaluates the quality gates declared in `.codelore-thresholds.toml` at HEAD and returns a JSON object:

```json
{
  "verdict": "pass" | "fail" | "no_thresholds",
  "violation_count": 3,
  "violations": [
    { "gate": "code_health_min", "path": "src/core.rs", "actual": "42.1", "threshold": "60.0" }
  ]
}
```

`no_thresholds` is returned when no `.codelore-thresholds.toml` exists at the repo root. Gates covered: `cognitive_max`, `hotspot_score_max`, `code_health_min`, `disallow_clone_type_1`, `max_red_effort_pct`, `max_dependency_cycles`, `max_propagation_cost`, and `code_familiarity_min`. This is a subset of what `codelore check` evaluates: the `max_findings_in_hot_files` gate (external SARIF findings × hotspots), the `corpus_percentile_max` gate, degraded-gate semantics (`fail_on_degraded`), and `--ratchet` are only available in `codelore check` proper. When a config file configures any of those, this tool's verdict can therefore differ from a CI run of `codelore check` — treat `codelore check` as the authoritative gate; use `check_gates` for a fast interactive read of the shared subset.

Parameters: none.

Cost: warm-cache fast. Cold-cache triggers ingest.

#### `finding_hotspot_overlap`

Returns the behavioral×static fusion table: external scanner findings joined with hotspot rank and code-health band. Each row carries a `priority` label (`act-now` / `plan` / `note`) derived from the three signals.

When the external findings sidecar is absent or empty (no prior `codelore ingest-sarif` run for this repo), the tool returns a structured note instead of an error:

```json
{ "findings": [], "note": "run codelore ingest-sarif first" }
```

Parameters: none.

Cost: warm-cache fast after `ingest-sarif`; does not trigger history re-ingest.

### Architecture note

Each tool call opens its own `FactsDb` connection via the warm-cache path. This is intentional: `duckdb::Connection` is `!Send + !Sync` and cannot cross thread or async boundaries, so each call runs entirely on a dedicated blocking thread (`tokio::task::spawn_blocking`) from connection open to result serialization. The connection is dropped before the future resolves. All tools are read-only; no tool modifies the repository or the fact store.

### Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Server not appearing in client tool list | Client config path wrong or JSON syntax error | Check the config file location for your client; validate JSON syntax; restart the client |
| `repo path does not exist` error on first tool call | Absolute path required; relative paths are resolved at server startup, not at call time | Use an absolute path in `args` |
| First tool call is very slow (30 s+) | Cold-cache ingest running — normal for large repos | Wait for it to complete; subsequent calls in the same session use the warm cache |
| `delta_health` returns a tool error for a valid branch | Branch name is valid locally but not yet fetched | Run `git fetch` in the repo, then retry |
| Tools return stale data after commits | The cache key includes HEAD SHA; new commits produce a new cache entry automatically | No action needed — the next call after a commit will re-ingest |

## 12. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: ingest commits: repository error: find_parent_commit ... could not be found` | Shallow clone (`--depth=N`) is missing parent ancestry for analyses that walk back | Use a full clone or run only HEAD-only analyses (`clones` works on shallow clones — it short-circuits the ingest) |
| Hotspot scores are all `0.0` | Repo has only one commit, OR `fetch-depth: 0` not set in CI | Set `fetch-depth: 0` in `actions/checkout` |
| `codelore analyze --analysis bogus` errors with help-text | Typo on analysis name | The error message lists all supported analyses |
| Same file appears twice in `revisions` output (e.g. `crates/bca-lib/foo.rs` AND `crates/codelore-lib/foo.rs`) | Git rename split — CodeLore doesn't follow renames yet | Known limitation; tracked in [`roadmap-v1.x-and-beyond.md`](roadmap-v1.x-and-beyond.md) (Tier 3, "Rename tracking") |
| `clone-coupling` returns 0 rows on a small repo | Fisher exact test needs ≥ 3 shared commits AND non-degenerate contingency table | Verify with `--analysis coupling` first; if that's empty too, the repo doesn't have enough history |
| `--format parquet` fails with "requires --output" | Binary format can't stream to stdout | Pass `--output FILE.parquet` |
| `--format sarif` fails with "supported: hotspots, clones, clone-coupling" | Other analyses don't have a SARIF rule yet | Use one of the supported analyses, or `--format json` |
| Disk space warning during `cargo test` | DuckDB bundled build is heavy (~3-4 GB target dir) | `cargo clean -p codelore-lib` to free; the next build is faster than a full clean |
| `cargo bench` errors on parallel/serial benches | rayon `build_global()` can only run once per process | The bench file uses per-iteration `pool.install()` which sidesteps this; only an issue if you write your own bench |

## 13. Workspace layout

```
codelore/
├── Cargo.toml                            # workspace manifest
├── README.md                             # the 5-min pitch
├── CHANGELOG.md                          # all releases
├── Containerfile                         # distroless image
├── examples/
│   └── .github/workflows/                # GHA integration templates
├── crates/
│   ├── codelore-lib/                     # the library
│   │   ├── src/
│   │   │   ├── facts/                    # DuckDB fact store + ingest pipeline
│   │   │   ├── analyses/                 # analyses (one file each)
│   │   │   ├── output/                   # 11 format emitters
│   │   │   ├── repo/                     # GixRepo + GitCliRepo + Repo trait
│   │   │   ├── complexity/               # tree-sitter dispatch + ComplexityEntity
│   │   │   ├── clones/                   # Type 1+2 fingerprinting
│   │   │   ├── identity/                 # mailmap + bots.toml
│   │   │   ├── kamei/                    # 14-feature change vector
│   │   │   ├── cache.rs                  # persistent fact-store cache
│   │   │   ├── provenance/               # manifest sidecar
│   │   │   └── options.rs                # the runtime config struct
│   │   ├── tests/                        # integration tests
│   │   └── benches/end_to_end.rs         # criterion harness
│   ├── codelore-cli/                     # clap CLI
│   │   └── src/
│   │       ├── main.rs                   # analyze dispatch
│   │       ├── args.rs                   # CLI surface
│   │       ├── diff.rs                   # codelore diff implementation
│   │       ├── diff_output.rs            # diff output emitters
│   │       └── mcp.rs                    # MCP server implementation
│   └── codelore-rca/                     # vendored Mozilla rust-code-analysis (MPL-2.0)
├── docs/
│   ├── advanced-usage.md                 # ← you are here
│   ├── codebase_analysis.md              # architecture overview (workspace + data flow)
│   ├── perf-evidence-v1.md               # release-blocker performance numbers
│   ├── roadmap-v1.x-and-beyond.md        # near-term and long-term backlog
│   └── superpowers/
│       ├── specs/                        # full design specification
│       └── plans/                        # every implementation plan, executed task-by-task
├── scripts/pgo.sh                        # PGO scaffolding (queued post first stable tag)
├── .github/workflows/
│   ├── ci.yml                            # cargo test + clippy + fmt + deny
│   ├── bench.yml                         # weekly perf regression gate
│   ├── release.yml                       # cargo-build matrix + SLSA L3 + Homebrew (on tag push)
│   └── container.yml                     # distroless image (on tag push)
└── .codeloreignore                       # optional, user-supplied
```
