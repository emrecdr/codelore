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

CodeLore ships **57 behavioral analyses** across four tiers. The table below is split into the code-maat-parity analyses (drop-in successors to legacy code-maat), a modern signal (`top-committers` — a first-class per-author leaderboard that code-maat approximated via `-a author-churn` + sort), modern additions marked ★ (the SARIF-backed differentiators including `hotspots`, `code-health`, `clones`, `clone-coupling`, `hotspot-velocity`, `refactoring-targets`, and `finding-hotspot-overlap`), graph-analytics analyses marked ★ (knowledge-islands + code-familiarity + team-composition + coordination-needs + marginal-owner-risk + centrality + communities), and architecture-analytics analyses marked ★★ (god-classes + architecture-violations + dependency-cycles + cycle-health + architecture-roles + instability + architecture-metrics + architecture-trend + cycle-origins + modularity-violations + unstable-interface + crossing + stale-code + pair-programming + lead-time + bus-factor + delivery-friction — `dependency-cycles` (Tarjan SCC), `architecture-roles` (Core/Shared/Control/Periphery), `instability` (Martin Ca/Ce/I) and `architecture-metrics` (Lakos ACD/NCCD + propagation cost) all run on a shared import-graph kernel; `architecture-trend` reruns that kernel at sampled historical revisions to show structural decay over time; `modularity-violations`, `unstable-interface` and `crossing` fuse the structural import graph with the temporal co-change graph (the DV8 hotspot-pattern trilogy); see `docs/maximum-feature-plan.md`).

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
| `hotspots` ★ | "Which files are both complex AND change a lot?" | `percentile_rank(revs) × percentile_rank(cognitive) × (100 − cognitive_health) / 4` — `cognitive_health` here is the inline cognitive-only proxy `100 × (1 − 0.40 · normalize(cognitive))` ∈ [60, 100], so the unscaled product caps at 40; dividing by 4 maps output to [0, 10] ([see design spec](superpowers/specs/2026-06-06-codelore-design.md)) | The headline ranking signal — refactor priorities |
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
| `delivery-metrics` ★★ | "What do our batch size, rework, branch lifetime, lead-time, and gatekeeping proxies look like?" | Percentile distributions (p50/p75/p90) of six flow metrics derived from git topology and hunk overlap; requires `--include-merges` | Git-only proxy snapshot of flow-metric distributions — run before deciding whether full DORA tooling is warranted |
| `release-cadence` ★★ | "How often do we ship, and is the pace changing?" | Inter-release tag gaps (days), median, IQR, OLS trend; tags filtered by `--release-tag-glob` (default `v*`) | Release-velocity monitoring without a deployment system; trend direction (`accelerating` / `stable` / `slowing`) at a glance |
| `architecture-trend` ★★ | "Is the architecture getting better or worse over time?" | Propagation cost / cycle count / largest tangle recomputed at sampled historical revisions (the same metrics as `architecture-metrics`, time-sliced) | Structural decay detector — see when a tangle started growing or a refactor paid off |
| `cycle-origins` ★★ | "When and where did each dependency cycle start?" | Bisects history to find the commit each HEAD dependency cycle first appeared | Commit-level archaeology: pinpoints the change that introduced a cycle so the root cause (not just the symptom) can be fixed |
| `delivery-friction` ★★ | "Which files slow down delivery most?" | Composite of `percent_rank(revs) × percent_rank(median lead-time) × percent_rank(cognitive)` per file; p95 lead-time + WIP-age side columns | Only files elevated on all three axes (churn × review-time × complexity) rank high — eliminates single-axis false positives |
| `effort-exposure` ★★ | "Are we spending engineering effort in healthy or unhealthy code?" | Per-band (red/yellow/green) breakdown of commit share and LOC share over the trailing window; drives the effort-exposure share bars on the SPA dashboard | Answers whether refactoring investment or technical-debt paydown is needed — the fraction of effort in red-band code is the key leading indicator |
| `health-trend` ★★ | "How has file-level code health changed over sampled commits?" | Code-health score series per file at sampled historical revisions; feeds the health-trend sparklines and improvements feed on the SPA dashboard | Distinguishes files that are genuinely improving from those that briefly recovered before deteriorating again |
| `function-xray` ★★ | "Which functions in a file change most often?" | Per-function hunk-overlap attribution: counts revisions where at least one diff hunk overlaps the function's line span; requires `--target <path>` | Gall et al. ICSM 2003 HistoryFinder — per-function change-frequency leaderboard with LOC, cyclomatic, and cognitive complexity; more precise than file-level churn |
| `function-coupling` ★★ | "Which function pairs in a file always change together?" | Per-function-pair co-change frequency with two-tailed Fisher exact significance; requires `--target <path>`; emits pairs with co-change count ≥ 2, sorted by p-value ascending | Adams et al. ICSM 2006 — function-level logical coupling within a file; pairs with low p-value are candidates for extract-and-share refactoring |
| `function-hotspots` ★★ | "Which individual FUNCTIONS are hot, repo-wide?" | `percentile_rank(revs) × percentile_rank(cognitive) × (100 − cognitive_health) / 4` — the `hotspots` formula, computed per HEAD-live function via `function-xray`'s hunk↔span overlap predicate instead of per file | Function-granularity complement to `hotspots`: finds the one hot function hiding inside an otherwise-quiet large file |

Almost all analyses are SQL views over the DuckDB fact store + thin Rust orchestrators (the architecture tier additionally builds the import graph in Rust). The exception is `defect-validation`, which reads a defect-calibration artifact instead of the fact store — see [Defect calibration](#defect-calibration-does-the-health-score-predict-where-defects-land-here) below. You can run any analysis at any output format.

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
codelore explain <path>             # per-file evidence dossier (add --llm for a grounded
                                    # advisory narrative — see §8.5)
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

The dashboard composes its widgets in one HTML file — grouped into the six titled sections described below — plus a tabbed click-target file detail drawer:

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

### Dashboard layout: sections, navigation, responsive behavior

The widgets above are grouped into six titled sections, each ordered internally overview → ranked → diagnostics:

| Section | Widgets |
|---|---|
| **Overview** | Quality dimensions (factor tiles) · Codebase at a glance (KPI tiles) · Guided tour · Hotspots hero (circle-pack) |
| **Hotspots & Risk** | Hotspot table · Hotspots treemap · Function X-Ray |
| **Code Health** | Repo health timeline · Trends · Effort distribution · Health improvements & regressions · Cognitive distribution · Multi-metric comparison |
| **Architecture** | Architecture graph · Dependency structure matrix · Architecture trend · Module coupling · Change coupling |
| **Knowledge** | Knowledge surfaces · Knowledge islands |
| **Delivery** | Delivery · Delivery risk (Kamei) · Commit activity |

A sticky navigation bar sits below the header with one chip per section. Clicking a chip smooth-scrolls that section into view; an `IntersectionObserver` highlights the chip for whichever section is currently in view as you scroll, and a back-to-top button appears once you've scrolled past the fold. The four factor tiles double as the same jump links, so clicking "Architecture" in the Overview section jumps straight to the Architecture section. Navigation never touches the URL — reloading or sharing a link always returns to the top of the dashboard.

Each section heading carries a collapse chevron. Sections always render fully expanded on load (collapse state is never persisted) so every chart initializes in a visible container; collapsing hides the section's widgets, and re-expanding resizes any chart that needs it to recover correct dimensions.

Below 1280px-wide viewports (laptops and narrower), every widget renders at full width — one chart per row, the most readable presentation at cramped widths. At 1280px and above, each section's grid becomes two columns: designated pairs share a row (Code Health's health-improvements-feed + cognitive-distribution; Knowledge's surfaces + islands), the Delivery card may sit alone in its row, and every other widget spans both columns. Widgets with wide inner content (the dependency-structure matrix, sortable tables) scroll horizontally inside their own card; the page itself never scrolls sideways.

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

**Exempting improving churn.** Red-band churn is not all bad: a commit that *refactors* a red file toward health lands in the same red band as one that degrades it. The optional companion key splits the two:

```toml
[gates]
max_red_effort_pct = 15.0
red_effort_exempt_improving = true   # gate only the DEGRADING share of red churn
```

With `red_effort_exempt_improving = true`, `codelore check` decomposes the red band's window churn by each red file's own net health movement — improving vs degrading, judged by the same fixed complexity risk bands as `codelore diff`'s `delta-health` — and compares only the **degrading** share against the ceiling. Churn that refactored a red file toward health is exempt. The failure message discloses all three numbers, e.g. `actual 6.20 (red 18.30, improving 12.10 exempt) vs threshold 15.00`, so the exemption is never silent. The key defaults to `false` (the gate compares the full red churn share — behaviour unchanged), and it has no effect unless `max_red_effort_pct` is also set. A file is exempted only on *demonstrable* net improvement; a file that both refactored and degraded within one window is classified by the net of those movements. The `effort-exposure` analysis surfaces the split directly in its `churn-share-improving-pct` / `churn-share-degrading-pct` columns for the red band.

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

**The artifact and its vintage.** The reference distribution is a *calibration artifact*: compact JSON holding, per language, a 1001-point quantile-breakpoint vector for each metric — aggregated numeric distributions only, no source code. Each artifact carries a `corpus_vintage` label recording which corpus and era it represents. CodeLore ships an **embedded world corpus** (vintage `world-2026-07-26`, pooled from permissive-license open-source projects across the five Tier-1 languages: rust, python, java, javascript, typescript) that activates the lens by default — no configuration required. Pass `--calibration <artifact.json>` on `analyze` or `check` to override the embedded corpus with a hand-built or organization-specific one. Whichever artifact the lens actually applies is stamped into the provenance manifest as `corpus_vintage`, so a report records exactly which reference it was measured against.

**Repo-level architecture percentiles.** Besides the per-function language pools, an artifact can carry a `repo_metrics` section: for `propagation_cost` and `cycle_file_share` (the fraction of the import graph's files sitting in a non-trivial dependency cycle), the sorted raw values — **one observation per corpus repo** (every corpus repo contributes, since the import graph now counts every live source file). When the active artifact has this section, `architecture-metrics` appends three rows: `corpus_percentile:propagation_cost` and `corpus_percentile:cycle_file_share` (midpoint-rank percentiles of this repo's values against the pools, `0..1`) plus `corpus_n`, the number of corpus observations backing them. Read these as **"percentile among N corpus repositories"** — the base is one value per repo, so it is coarse by construction; `corpus_n` states the sample size honestly, and the lens is a rough placement, not a fine-grained calibration. The rows are additive: no active artifact, or an artifact without `repo_metrics`, leaves `architecture-metrics` output exactly as it always was. The SPA's Architecture factor tile carries the propagation-cost percentile on its detail line when present.

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

### Defect calibration: does the health score predict where defects land here?

Corpus percentiles ask "how does this file compare to the wider world." **Defect calibration** asks the question the whole code-health initiative was built on: *does the health score actually predict where defects land in **this** repository?* — and, when the evidence clears an honesty floor, tunes the eight smell weights to the repo's own defect history. Everything is mined from git alone, fully local, and delivered as an opt-in vintage-stamped artifact so scores stay byte-reproducible.

**Mining the artifact — `codelore calibrate-defects`.**

```sh
codelore calibrate-defects \
  --repo . \
  --output defects.calib.json \
  [--vintage defects-2026-07] \
  [--window-days 365]
```

One pass runs three stages and writes a compact `defects.calib.json`:

- **Fix oracle.** A commit is a *fix* when its message matches a conventional-commit `fix:` / `fix(scope):` prefix or a word-boundary defect term (`bug`, `bugfix`, `fix`/`fixes`/`fixed`, `defect`, `regression`, `hotfix`), and it is neither a merge nor a `Revert "…"`. This oracle is deliberately narrower than the kamei `fix` feature (which keeps its broad `issue|error|patch` alternation for JIT-SDP — a documented SZZ precision trap); the oracle model also carries extra include patterns for tracker-id conventions, recorded in the artifact; they are not yet exposed as a CLI option.
- **AG-SZZ linkage.** For each fix, CodeLore takes the fix's deleted pre-image lines and `git blame`s the fix's parent to find the commit that last introduced each line — the candidate defect-introducing commit — then drops cosmetic (blank / comment-only) blamed lines and any candidate at-or-newer than the fix (the AG filter plus a clock-skew guard). This is annotation-graph SZZ (Śliwerski, Zimmermann & Zeller 2005; Kim et al. 2006 AG-SZZ) behind a pluggable seam. A single file's blame or blob-read failure is skipped and counted, never fatal.
- **Validation + constrained tuning.** Each defect-introducing commit is matched to the nearest historical code-health band sample at-or-before its date; the report tallies where those changes landed by band and scores HEAD `structural_risk` against the defect-implicated file labels (AUC, precision@k). A deterministic coordinate search over the eight smell weights then tunes them on a temporal 60/40 older-train / newer-validate split (never a random split — a leakage guard) — **but only when the evidence clears an honesty floor**: at least 30 linked defect-changes, at least 10 implicated files, a tuned *validation*-split AUC of at least 0.5 (weights that rank below random on unseen recent defects are never adopted, however large their margin), and the tuned weights must beat the defaults' *validation*-split AUC by a margin (+0.02). If any floor is unmet the defaults are kept and the reason is recorded; the artifact always states which branch was taken and shows both AUCs. Finding no fix commits at all is not an error — the artifact is written with empty linkage and defaults kept.

Two runs over the same history produce byte-identical artifacts. Mining reads only committed state, so `calibrate-defects` refuses a dirty working tree unless you pass `--allow-dirty` (the artifact still describes the committed HEAD, not your edits).

**Applying it — `--defect-calibration`.** Pass the artifact on `analyze`, `check`, or `explain <path>` (see [§8.5](#85-llm-enrichment-advisory-narratives) for the dossier's defect-evidence section):

```sh
codelore analyze --analysis code-health --defect-calibration defects.calib.json
```

When active, `code-health` substitutes the artifact's weights for the built-in smell weights and the provenance manifest stamps `defect_vintage` alongside `corpus_vintage`. For a *defaults-kept* artifact those weights **are** the defaults, so applying it changes no score while the vintage stamp still records that the artifact was consulted. Applying an artifact mined from a **different repository** is a hard error — a repo-identity fingerprint (a hash of the canonical repo path, recorded at mining time) is checked before the weights are used; pass `--allow-foreign-calibration` to override it for a fork. **Without the `--defect-calibration` flag, behavior is byte-identical to today** — contract-tested by strip-and-compare, the same guarantee the corpus lens carries. The two calibrations compose cleanly: corpus percentiles are additive columns, defect weights change the composite, and both journeys are recorded in provenance.

Instead of passing `--defect-calibration` on every invocation, declare the artifact once in `.codelore-thresholds.toml`'s `[calibration]` section (see [§5](#5-configuration-codeloreignore--thresholds)) — `analyze`, `check`, `explain <path>`, and `codelore mcp` all resolve it the same way, with the explicit flag always taking precedence.

**Reading the evidence — the `defect-validation` analysis.** `defect-validation` reads the artifact (it never mines) and flattens its evidence into `(metric, value)` rows:

```sh
codelore analyze --analysis defect-validation --defect-calibration defects.calib.json --format markdown
```

- `band:red` / `band:yellow` / `band:green` — the share of defect-introducing changes that landed in files at each code-health band *at the time*, each carrying its `count/total` (the total is the linked defect-changes that had band data — `excluded_no_data` counts the rest).
- `auc_default` — AUC of HEAD `structural_risk` against the defect labels; `precision_at_10` and `precision_at_red` — precision among the 10 highest-risk files and among the files banded red at HEAD.
- `implicated_files`, `linked_defects`, `excluded_no_data`, `band_samples`, and the mining tallies (`fixes_found`, `links_found`, `files_blamed`, `lines_considered`, `lines_dropped_cosmetic`, `pure_addition_fixes`, `blame_failures`).
- `weights_source` (`tuned (applied)` or `defaults kept: <reason>`) and the tuning AUCs (`tuning_auc_train`, `tuning_auc_validation_default`, `tuning_auc_validation_tuned`) — surfaced **whenever present**, so you can see for yourself when tuning was applied yet the validation AUCs still sit below 0.5.

Presentation follows the project's honesty framing: **association, not causation** — a defect-introducing commit touching a red file is evidence the score ranks that file high, not proof the score *caused* the defect. Every count carries its `n`; an absent metric renders as an explicit `n/a (<why>)`, never silently dropped; there are no vendor-style multipliers. Without a configured artifact the analysis returns zero rows and prints a one-line hint pointing at `codelore calibrate-defects` — an honest absence, not an error.

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

**MI bands are repo-relative, not absolute** (see [`analyses::mi`](../crates/codelore-lib/src/analyses/mi.rs) for the full rationale): `High`/`Moderate`/`Low` are derived from each file's Maintainability Index percentile rank within the repo (`PERCENT_RANK`), not a fixed Coleman/SEI cutoff — the literature's absolute thresholds were calibrated on much smaller 1990s modules and would misclassify most modern files as "low". One consequence: a repository where every scored file has an identical MI value (a toy fixture, or a corpus of near-duplicate files) bands every file `Low` — `PERCENT_RANK` assigns rank `0` to every row of a tied population, and rank `0` falls in the bottom-quartile `Low` cut. This is the deliberate cost of relative banding on a degenerate population, not a defect; the raw `mi` value and `mi_rank` percentile are always surfaced alongside the band so you can see the absolute context too.

### SARIF rules CodeLore ships

| Rule ID | Tags | When it fires |
|---|---|---|
| `CODELORE-HOTSPOT` | `behavioral`, `hotspot` | One result per hotspot row; `security-severity = (100 − cognitive_health) / 4`; `level` derived from severity band (≥7 = error, ≥4 = warning, else note) |
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
      --fdr-correction          Select `coupling`'s significant pairs by a
                                Benjamini-Hochberg false-discovery-rate
                                correction over the whole family of
                                Fisher-tested pairs, in place of the per-pair
                                `p < 0.05` gate. Off by default; controls the
                                expected false-positive fraction across all
                                tested pairs (fewer, higher-confidence pairs
                                on large repos). `--code-maat-compat` still
                                bypasses the significance gate entirely.

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
      --temp-dir PATH           Override the DuckDB spill directory (must
                                already exist and be writable). Defaults to a
                                subdirectory of the cache root, or the system
                                temp directory when there is no cache root in
                                play (e.g. --no-cache).
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
      --llm                 Append an advisory LLM PR narrative (text/markdown
                            only; degrades to a stderr warning on failure — see §8.5)
      --llm-refresh         Regenerate the narrative even when a cached one exists
```

The diff subcommand emits four SARIF rule types: CODELORE-HOTSPOT (newly-entering or score-rising hotspots), CODELORE-CLONE (PR-introduced clone families), CODELORE-MISSING-COCHANGE (historically-coupled partner files this PR didn't touch), and CODELORE-DELTA-HEALTH (degrading delta-health functions, one result per degrading file). CODELORE-LIVE-CLONE is an analyze-mode rule (`--analysis clone-coupling --format sarif`), not a diff rule.

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
delta_code_health_min_per_file = 0.0  # working-tree gate only: no changed file may lower its own health
new_file_health_min = 50.0    # working-tree gate only: each ADDED file must clear this projected score

[calibration]
defect_artifact = "defects.calib.json"  # repo-declared default, see below
```

`max_dependency_cycles` / `max_propagation_cost` are evaluated against HEAD by `codelore check`; `no_new_cycles` compares the base-rev and head-rev import graphs in `codelore diff` and fails the PR when head has more cycles than base. `delta_health_min` and `deny_degrading_verdict` both act on the `delta_health` section: `delta_health_min` fails when `ratio < threshold` (skipped on `no-code-change` diffs where no ratio exists); `deny_degrading_verdict` fails when the verdict is exactly `"degrading"`. `max_red_effort_pct` gates on the `effort-exposure` churn share (share of changed lines, added + deleted) for the red band; `code_familiarity_min` gates on the repo-scope `familiarity-pct` (0–100) from `code-familiarity` (see the dedicated subsections in the SPA widget surface above).

`new_file_health_min` sits beside `delta_code_health_min_per_file` as the other change-scoped floor, but for the population `delta_code_health_min_per_file` structurally cannot see: a brand-new added file carries no baseline score to delta against, so it never reaches the per-file delta floor no matter how unhealthy it is. `new_file_health_min` closes that gap by floor-checking each added file's own *projected* score directly — one violation per offending added file, naming the file and its projected score. A deleted file has no projected score and never triggers it. Like `delta_code_health_min_per_file`, it is evaluated only by the working-tree gate surfaces (`codelore gate` / `gate_changes`); `codelore diff` ignores the key.

Three surfaces evaluate this file, each against a different input: `codelore check` gates the committed tree at HEAD, `codelore gate` (and its MCP twin `gate_changes`, [§11.9](#gate_changes)) gates the uncommitted working tree against HEAD, and `codelore diff` gates a rev range. Same file, three non-overlapping readings:

| | `codelore check` | `codelore gate` / `gate_changes` (MCP) | `codelore diff` |
|---|---|---|---|
| Input | committed tree at HEAD | working tree vs HEAD | rev range (`base...head`) |
| Keys evaluated | `[gates]` (all, plus degraded-gate semantics and `--ratchet`) | `[diff]`: `delta_code_health_min`, `delta_code_health_min_per_file`, `new_file_health_min`, `no_new_cycles` | `[diff]`: `delta_code_health_min`, `new_hotspot_max`, `no_new_cycles`, `delta_health_min`, `deny_degrading_verdict` |
| `delta_code_health_min` metric | not evaluated | `code-health`'s composite `score`, range `[0, 100]` — whole-repo median over all scoreable files (1-revision floor) | `hotspots`' inline `cognitive_health` proxy, range `[60, 100]` — whole-repo median over the min-revs-filtered hotspot rows |
| Cycle semantics | `max_dependency_cycles`: cycle count at HEAD | `no_new_cycles`: cyclic-node *membership* — one violation naming each newly cyclic file; still fires when two existing cycles merge into one | `no_new_cycles`: cycle-*count* comparison — fails when head has more cycles than base |
| Exit code on violation | 1 | `gate` exits 1; `gate_changes` reports, never exits | 4 |

**`delta_code_health_min` compares two different metrics depending on the surface.** The same `[diff]` config key is evaluated against `code-health`'s composite `score` (`[0, 100]`, the field `--analysis code-health` reports and `code_health_min` gates) on `codelore gate` / `gate_changes`, but against the `hotspots` analysis's inline `cognitive_health` proxy (`[60, 100]`, structural-complexity-only — see the module doc in `hotspots.rs` for why the two scores never claim to agree) on `codelore diff`. A file can read healthy under one metric and unhealthy under the other, and the same numeric threshold (e.g. `-5.0`) represents a different-sized health movement on each surface. This is a **documented, deliberate divergence** — not a bug — carried forward until unifying the two metrics is made as an explicit decision; do not assume the two surfaces agree just because they share a config key.

`delta_code_health_min_per_file` and `new_file_health_min` are evaluated only by the working-tree gate surfaces — `codelore diff` ignores both. Conversely, `new_hotspot_max`, `delta_health_min`, and `deny_degrading_verdict` are diff-only and never evaluated by the working-tree gate.

#### The `[new_code]` period gate

`[gates]`' absolute floors bind on the legacy tail: a `code_health_min` must sit below the worst old file, so it says nothing about whether the code being written *this quarter* is healthy, and it ratchets only when someone re-bases it by hand. `[diff]`'s `new_file_health_min` floors new files, but only within *one pull request*. `[new_code]` fills the period-scoped gap — "new and recently-touched code is held to a strict standard; legacy code is only required not to degrade" — over a rolling window rather than a single PR:

```toml
[new_code]
window_days = 90            # the rolling working-set window (anchored to the last commit date); [7, 365]
born_health_min = 60.0      # files BORN in the window must score ≥ this at HEAD; [0, 100]
touched_no_degradation = true  # files TOUCHED (not born) in the window must not net-degrade over it
```

Two bands split the window's live-at-HEAD source files by how much of each the window actually wrote:

- **Born in window** — a file whose first commit lands inside the window must meet `born_health_min` at HEAD. This is the period-scope generalization of `[diff]`'s `new_file_health_min`: same "new files must be born healthy" floor, but over a rolling period instead of one PR. A violation reads `born_health_min: <path> — actual 41.2 (born in window) vs threshold 60.0`.
- **Touched (not born) in window** — a file touched inside the window but first seen earlier owes *non-degradation*: its net health movement over the window must be non-negative when `touched_no_degradation` is on (the default once the section is present). The signal is the **same** per-file net-movement the `red_effort_exempt_improving` exemption computes — delta-health good-minus-bad LOC weight between the window-start revision and HEAD, over a scoped per-file parse of the touched files only (no second full-tree health scan, no blame machinery on the gate path). The effort exemption asks whether that movement is strictly positive; this band asks whether it is non-negative, so a window that touched a file without moving any function across a risk band (a typo fix, a comment) nets zero and passes. A violation reads `touched_no_degradation: <path> — actual net -3.0 over 90d vs threshold ≥ 0`.
- **Untouched** — legacy files nobody edited in the window are exempt; only the absolute `[gates]` apply to them.

The section is **opt-in by presence**: any `[new_code]` table — even an empty one — enables the gate and makes the thresholds file non-empty; its absence is byte-identical to before everywhere, including the `change_context` briefing. `born_health_min` is optional (omit it to run only the touched band). The gate is evaluated wherever `[gates]` is — `codelore check` and the `check_gates` MCP tool — and is **skipped** (recorded `verdict = "skipped"`, exit code unaffected, disclosed on stderr) when the repository's history is shallower than the window: with the whole repo inside the window there is no legacy tail to contrast the working set against, so flagging every file as born would be a surprise. `codelore gate` / `gate_changes` and `codelore diff` do not evaluate `[new_code]` — they gate the `[diff]` scope, not the `[gates]` scope.

`new_file_health_min` (PR scope) and `[new_code]`'s `born_health_min` (period scope) are complementary and **both stay**: the PR floor answers "is this pull request acceptable now", the period floor answers "is the active working set healthy" — different questions, evaluated by different commands. The agent-loop `change_context` briefing ([§11.9](#gate_changes)) adds a one-line `new-code:` disclosure for a briefed file that is born or touched in the window, but only when the section is configured; it never changes a `gate_changes` verdict.

Why a rolling window rather than a pinned baseline ("all code after tag X is new")? A fixed baseline reintroduces the manual re-basing this repo's own floor history shows people forget. A rolling window is self-maintaining — remediation and growth churn age out on their own — so a team that wants a ratchet instead can pin `window_days` high and tighten `born_health_min` over time; the reverse is not possible.

`[calibration]` is not a gate — it's a config *selector* that declares the repo's default defect-calibration artifact once, so `analyze`, `check`, `explain <path>`, and `codelore mcp` all pick it up without repeating `--defect-calibration` on every invocation. Precedence: an explicit `--defect-calibration` flag (or the MCP server's startup flag) always wins; otherwise the `[calibration] defect_artifact` path is used, resolved relative to the repo root (absolute paths pass through as-is); otherwise the run is uncalibrated. A thresholds file containing only `[calibration]` still leaves `check` vacuously passing — see [Defect calibration](#defect-calibration-does-the-health-score-predict-where-defects-land-here) for what the artifact does once applied.

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
| `landed_by_other_pct` | Share of non-merge commits where the committer differs from the author (case-normalized email) — a peer-review/gatekeeper proxy | `commits` has no `committer_name`, so unlike `canonical_author` the committer side can't be mailmap-resolved; a person authoring and landing under two emails they own reads as a false "gatekept" commit — not ownership-grade signal |

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

To surface this, codelore emits a `tracing::warn!` whenever a cache hit lands on a working tree with uncommitted changes to a tracked file (untracked files don't count — they can't affect a HEAD-time scan):

```
WARN cache hit on a working tree with uncommitted changes; HEAD-time metrics
     (hotspots' complexity, clones) may be stale relative to disk.
     Pass `--no-cache` to recompute against the current working tree.
```

Detection is cheap (gix `Repository::is_dirty` for the pure-Rust walker, `git status --porcelain --untracked-files=no` for the CLI walker). Pass `--no-cache` if the dirty state matters for your analysis. The warning is informational — codelore still serves the cached result by default to preserve the 10–100× speedup on clean repeated runs. Auto-invalidation via worktree-content hashing was considered and rejected: hashing every tracked file on every invocation costs 100ms–1s on large trees, which would erase the cache's perf win for the majority case where the cache is correct.

### Memory ceiling and disk spill

Every `DuckDB` connection codelore opens — cache hit, cache write, and the `--no-cache`/dirty-worktree in-memory path alike — carries a `memory_limit` PRAGMA (4 GB, matching the peak-memory target in this project's performance targets) and a `temp_directory` PRAGMA. Once a query's resident state would exceed the ceiling, `DuckDB` spills intermediate hash-join/sort/aggregation state to `temp_directory` instead of growing unbounded, so a very large repository degrades to slower (disk-bound) execution rather than getting OOM-killed.

The spill directory defaults to a subdirectory of the active cache root (`<cache_root>/codelore/spill`), or the system temp directory when there is no cache root in play (e.g. `--no-cache`). Override it with `--temp-dir PATH`:

```bash
# Analyze a very large repo with an explicit scratch volume for spill
codelore analyze --analysis hotspots --temp-dir /mnt/scratch/codelore-spill

# codelore check honors the same flag
codelore check --temp-dir /mnt/scratch/codelore-spill

# codelore calibrate-defects mines entirely in memory (no cache root), so its
# spill directory always defaults to the system temp directory unless overridden
codelore calibrate-defects --repo . --output defects.calib.json --temp-dir /mnt/scratch/codelore-spill
```

`--temp-dir` must already exist and be writable; codelore validates it up front rather than failing deep inside an ingest. The directory choice has no effect on analysis output — it changes only where `DuckDB` writes scratch files — so it is not part of the persistent-cache key.

## 8.5. LLM enrichment (advisory narratives)

CodeLore's numbers are deterministic. An opt-in LLM layer can synthesize them into reviewer-legible prose — but the differentiation is grounding, not generation: the model's only input is a deterministic **fact sheet** of values the analyses already computed, and every number the reply quotes is checked back against that sheet after generation. Advice with receipts, never generated code.

Everything in this section is strictly advisory. Scores, gates, SARIF, exit codes, and the provenance manifest are computed exactly as if the feature did not exist.

### The three outputs

| Surface | What it prints | LLM required |
|---|---|---|
| `codelore explain <path>` | The file's **evidence dossier**: ordered fact-sheet sections — code-health score/band, biomarker intensities, hotspot rank, coupling partners, ownership, function churn leaders, import-cycle membership, and (with `--defect-calibration`) the configured artifact's defect-evidence metrics. Deterministic, free, offline. | No |
| `codelore explain <path> --llm` | The dossier plus a grounded **Diagnosis** narrative. A **Refactoring direction** section appears only when the sheet carries structural evidence for one (an import-cycle or functions section); when the evidence is absent the section is omitted rather than invented. | Yes |
| `codelore diff <range> --llm` | The deterministic diff output exactly as today, followed by a delimited **LLM narrative (advisory)** block: one reviewer-ready read of what the change does to the codebase's health and which files carry the risk. Rendered for `text` and `markdown` output only; ignored (with a stderr note) for `json`/`sarif`. | Yes |

```bash
# Free, deterministic, offline — the evidence dossier:
codelore explain src/core/engine.rs --repo .

# The dossier + the grounded narrative (requires a configured endpoint):
codelore explain src/core/engine.rs --repo . --llm

# PR narrative appended to the diff output:
codelore diff origin/main...HEAD --repo . --llm
```

The MCP server exposes the same per-file surface as the `explain_file` tool — see [§11.9](#119-mcp-server-codelore-mcp).

The dossier resolves any single tracked source file: its analyses run with a 1-revision floor instead of the default corpus gate, so a file the default `analyze` run would hide still gets its own dossier (its hotspot/coupling/ownership numbers can therefore differ from a default run's).

Passing `--defect-calibration <path>` (see [Defect calibration](#defect-calibration-does-the-health-score-predict-where-defects-land-here)) adds a **defect-evidence** section: the artifact's `vintage`, its headline validation numbers (`auc_default`, `precision_at_10`, `precision_at_red` when available), `implicated_files`, `linked_defects`, and the band table (`band:<band>:changes` / `band:<band>:share`). Per-file defect implication is not derivable from the artifact, so only its artifact-wide metrics are surfaced. `--allow-foreign-calibration` applies an artifact mined from a different repository, exactly as it does for `analyze`/`check`. Without `--defect-calibration` the dossier carries no such section — byte-identical to today.

### Grounding: fact sheet in, citation check out

The prompt embeds the fact sheet verbatim as the model's sole evidence and instructs it to use only facts on the sheet, cite the exact numbers, and say "the data doesn't show" rather than guess. After generation, a citation check extracts every numeric token from the narrative and matches it against the sheet's values, tolerant of the narrative's own rounding (a narrative "0.79" is grounded by a fact of 0.786; "80%" by 0.803). The extraction is sign-aware: a leading minus binds to the token unless it is an infix hyphen in a date or range (`2026-07-15`, `defects-2026`), so `-0.5` is only grounded by a fact of `-0.5`. Every narrative then carries an inline provenance stamp:

```
advisory — model <id>, grounded ✓
advisory — model <id>, ⚠ contains uncited claims: -0.5, 42.5%
```

**Honest limits: the check labels magnitudes, it does not prove claims.** `grounded ✓` means "every number the narrative quotes appears in the evidence" — not "every claim is true". The check cannot detect a fabricated small count (whole numbers up to 12 in magnitude are exempt as prose scaffolding — list positions, "the 3 files"), a percent that happens to collide with an unrelated fraction on the sheet, or a real number attached to the wrong claim. The narrative is advisory; the dossier above it is the authority.

### Configuration (environment only)

The posture is local-first: with nothing configured but a model name, requests go to a local OpenAI-compatible endpoint (`http://localhost:11434/v1` — ollama's default). Out of the box nothing leaves the machine; a hosted provider requires an explicit environment change. Keys live in the environment only and are never persisted by codelore, and the fact sheet — repository evidence — is the only content ever sent.

| Variable | Meaning | Default |
|---|---|---|
| `CODELORE_LLM_PROVIDER` | `anthropic` or `openai-compat`. Unset: an `ANTHROPIC_API_KEY` in the environment selects the Anthropic dialect, otherwise the local-first OpenAI-compatible one. | unset |
| `ANTHROPIC_API_KEY` | Credential for the Anthropic dialect (required on it). | unset |
| `CODELORE_LLM_BASE_URL` | Base URL for the OpenAI-compatible endpoint (ollama, llama.cpp, LM Studio, vLLM, OpenAI, OpenRouter). | `http://localhost:11434/v1` |
| `CODELORE_LLM_API_KEY` | Optional bearer token for the OpenAI-compatible endpoint; local runners typically need none. | unset |
| `CODELORE_LLM_MODEL` | Model name. **Required** on the OpenAI-compatible dialect (any name from `ollama list` works); on the Anthropic dialect it overrides the default model. | none on OpenAI-compatible; a Sonnet-class default on Anthropic |

Note: `provider=anthropic` always pins the Anthropic API base URL — `CODELORE_LLM_BASE_URL` applies to the OpenAI-compatible dialect only and is ignored on the Anthropic path.

```bash
# Fully local via ollama (nothing leaves the machine):
export CODELORE_LLM_MODEL=llama3.2        # any model from `ollama list`
codelore explain src/core/engine.rs --repo . --llm

# Hosted Anthropic endpoint (explicit opt-in):
export ANTHROPIC_API_KEY=sk-ant-…
codelore explain src/core/engine.rs --repo . --llm
```

### Narrative cache

Each generated narrative is persisted as a JSON sidecar under the per-repo cache directory (`…/enrichment/<key>.json`, next to the fact store). The key is content-derived — a hash of the fact-sheet text, the prompt and fact-sheet schema versions, and the model id — so a change to the file's evidence, the prompt wording, or the target model misses naturally, and re-running over unchanged evidence is free (no model round-trip). `--llm-refresh` regenerates and replaces the cached entry. The cache is strictly best-effort: a corrupt or unwritable sidecar degrades to a warning, never a failure.

`explain <path>` without `--llm` prints a one-line staleness note when the file's own previously generated narrative no longer matches the current fact sheet. The note is scoped to that file — a sibling file's fresh narrative never triggers it.

### Failure postures

| Surface | On LLM or configuration failure |
|---|---|
| `explain <path> --llm` | Hard error with a setup hint — the narrative is the requested product. |
| `explain <path>` (no flag) | Never touches the network; cannot fail for LLM reasons. |
| `diff --llm` | One-line stderr warning; the deterministic output and exit code are untouched. |
| MCP `explain_file` | The call succeeds; the fact sheet is returned with a `narrative_error` field instead of a narrative. |

Requests use a single bounded timeout and no retries — enrichment is interactive, not batch.

### Advisory guarantees

- **Byte-identical without the flag.** Without `--llm`, every command's output is byte-identical to a build without the feature — no network reads, no default-path behavior change. (`explain <path>` is itself additive: a path argument was previously an unknown-topic error.)
- **Additive with the flag.** With `--llm`, analysis rows, SARIF, gate verdicts, exit codes, fact-store cache keys, and the provenance manifest are unchanged; narratives are additive text (or additive MCP fields) only.
- **Scoring isolation.** No module in the scoring path imports the enrichment layer; the dependency arrow points one way and is enforced by a structural guard test.
- **Grounding is visible.** Every narrative carries its model id and groundedness verdict inline.

`analyze` and `check` deliberately have no `--llm` flag — the parser rejects it — so the advisory layer cannot even be requested on the surfaces whose output feeds gates and CI.

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
- **No LLM in the scoring path** — we're transparency-first. CodeScene's ML hotspot ranking is the opposite of what we ship: every score is a published deterministic formula. The opt-in advisory narrative layer (§8.5) sits strictly outside the scoring path — it reads the analyses' outputs, never feeds them, and a structural guard test enforces that one-way arrow.
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

`codelore mcp --repo <path>` starts a Model Context Protocol server over stdio. AI agents connect to it and call the tools below; the server answers using the same persistent fact store the CLI uses. It is **fully local** — no account, no API key, no telemetry, no network access. The single exception is opt-in: when the operator configures an LLM endpoint through the `CODELORE_LLM_*` environment (see [§8.5](#85-llm-enrichment-advisory-narratives)), the `explain_file` tool additionally requests an advisory narrative from that endpoint; with nothing configured it stays offline like every other tool.

### Starting the server

```bash
codelore mcp --repo /path/to/repo
```

The server blocks and reads JSON-RPC 2.0 messages on stdin (newline-delimited), writes responses to stdout. It runs until the client closes the connection.

Pass `--defect-calibration <path>` (built with `codelore calibrate-defects`) to add a **defect-evidence** section to every `explain_file` response for the lifetime of the server — the same section `codelore explain <path> --defect-calibration` adds to the CLI dossier (see [§8.5](#85-llm-enrichment-advisory-narratives)). Unlike the per-file tool parameters, this is a startup-only flag: it applies uniformly to all `explain_file` calls in the session, not per-request. The artifact is loaded and its repo-identity checked before the server starts serving, so a bad path or an artifact mined from a different repository (without `--allow-foreign-calibration`) is a startup error rather than a failure on the first tool call:

```bash
codelore mcp --repo /path/to/repo --defect-calibration defects.calib.json
```

This does not add a network dependency — the artifact is a local JSON file produced by a prior `codelore calibrate-defects` run, consulted entirely offline like every other tool.

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

`no_thresholds` is returned when no `.codelore-thresholds.toml` exists at the repo root. Gates covered: `cognitive_max`, `hotspot_score_max`, `code_health_min`, `disallow_clone_type_1`, `max_red_effort_pct`, `max_dependency_cycles`, `max_propagation_cost`, `code_familiarity_min`, and `corpus_percentile_max` (the corpus lens is active by default via the embedded world calibration artifact, so no `--calibration` flag is needed; the gate reports `skipped` — not a pass — only when no calibration artifact is active at all). This is a subset of what `codelore check` evaluates: the `max_findings_in_hot_files` gate (external SARIF findings × hotspots), the `hotspot_anchored_max` gate (this tool's hotspot scan is the plain, unanchored variant), degraded-gate semantics (`fail_on_degraded`), and `--ratchet` are only available in `codelore check` proper. When a config file configures any of those, this tool's verdict can therefore differ from a CI run of `codelore check` — treat `codelore check` as the authoritative gate; use `check_gates` for a fast interactive read of the shared subset.

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

#### `explain_file`

Returns the same per-file evidence surface as `codelore explain <path>` ([§8.5](#85-llm-enrichment-advisory-narratives)). `fact_sheet` is always present: the ordered analysis sections (code-health, biomarkers, hotspots, coupling, ownership, functions, and import cycles) as an array of `{section, facts}` objects preserving the dossier's order. When the server was started with `--defect-calibration`, the fact sheet also carries a `defect-evidence` section — see the flag's description above.

When the server's environment has an LLM configured (the `CODELORE_LLM_*` variables, §8.5), the response also carries a grounded advisory `narrative` with its `model` id and a `grounded` citation-check verdict. When it does not — or when the request fails — a `narrative_error` field is returned instead. The fact sheet is always returned and the tool call never fails because the LLM is unavailable, so agents without a configured endpoint still receive structured evidence to narrate themselves.

Parameters:
- `path` *(required, string)* — file path relative to the repo root (e.g. `src/main.rs`).

Cost: warm-cache fast for the fact sheet; cold-cache triggers ingest. With an LLM configured, a cache-miss narrative adds one model round-trip (the narrative sidecar cache makes repeat calls on unchanged evidence free).

#### `change_context`

Returns a compact, temporal **pre-write briefing** for the 1–20 repo-relative files an agent is about to modify — what the committed history already knows about each path, so an edit can be planned with the file's hotspot standing, coupling, and ownership in view. Unlike every other tool, the result is plain fixed-format text (roughly 150 tokens per file), not JSON: it is written to drop straight into an agent's context.

Each requested path heads its own block, in request order, with five lines:

```text
crates/codelore-lib/src/cache.rs
  health 67.3 (yellow) · risk 0.42 · calibrated defects-2026-07-15
  hotspot #12 (score 0.67, 23 revs)
  co-change: options.rs (68%, p=0.003) · facts/mod.rs (54%, p=0.011)
  owner: Emre Camdere 82% (sole owner, active 12d ago)
  recent: 4 commits, 310 lines churned in last 90d
```

- **health** — composite score with its band and the `structural_risk` value, plus a `calibrated <vintage>` suffix when the server was started with `--defect-calibration` (or a repo `[calibration]` section), else `uncalibrated`. A path with no code-health row renders `health: no code-health row`.
- **hotspot** — 1-based rank in the full hotspot ranking; a path outside the ranking renders `not in the hotspot set`.
- **co-change** — up to three historically co-changed partners (edit those too), with any further significant partners disclosed as ` (+n more)` at the end of the line; when none clear the significance filter it renders `co-change: none significant`.
- **owner** — main author, ownership share, sole-vs-shared concentration, and a `departed <n>d` flag when the main author has been inactive past the departed threshold (else `active <n>d ago`); no attributable ownership renders `owner: inconclusive`.
- **recent** — commit count and churned lines over the recent window; a path untouched in the window renders `recent: quiet in last <window>d`.

A genuinely unknown path — absent from every feed (health, hotspots, co-change, ownership, recent churn), i.e. brand-new, untracked, or mistyped — renders a two-line block instead: the path followed by `no history at HEAD (new or untracked file)`. When the repository is partway through a merge, rebase, cherry-pick, or revert, one leading note precedes every block, disclosing that the briefing reflects committed HEAD history.

This is a **committed-history** view — it never inspects the working tree. To evaluate the committed tree against the repo's quality gates, use [`check_gates`](#check_gates); to gate the uncommitted working tree, use [`gate_changes`](#gate_changes).

Parameters:
- `paths` *(required, array of strings)* — 1–20 repo-relative paths the caller intends to modify. An empty list or a list longer than 20 is a tool error naming the limit.

Cost: warm-cache fast; cold-cache triggers a one-time history ingest.

#### `gate_changes`

The working-tree **quality verdict** for the agent loop: what the current uncommitted edits do to the repository *before* they are committed. The tool enumerates the tracked working-tree changes vs HEAD (staged and unstaged; untracked files excluded), re-parses only the changed files, projects their effect through the same code-health scoring engine every committed analysis uses, splices the working-tree import edges into the import graph, and evaluates the repo's working-tree `[diff]` gates against the projection — `delta_code_health_min`, `delta_code_health_min_per_file`, `new_file_health_min`, and `no_new_cycles`; see the comparison table in [§4's Quality-gate subsection](#quality-gate). Like `change_context`, the result is compact plain text, not JSON.

Line 1 is the verdict: `PASS`, `FAIL — <n> violation(s)`, or `no thresholds configured — advisory only` — findings and the delta table still render without thresholds, so the tool is useful before a repo commits to gating. A clean tree returns `PASS (no working-tree changes to gate)`. Violations follow in `codelore check`'s row form, then one line per advisory finding (`health-drop`, `newly-cyclic`, `coupling-absence`, `clone-introduction`, `new-file`, `unparseable`), then a per-file delta table capped at the ten largest `|delta|` rows with a `(+n more files)` tail:

```text
FAIL — 1 violation(s)
  - delta_code_health_min_per_file: src/core.rs — actual -12.40 vs threshold +0.00
[health-drop] src/core.rs: projected code health drops from 85.2 to 72.8 (-12.4).
src/core.rs  85.2 → 72.8  (-12.4)
```

A file the projection cannot score renders its honest absence in the delta table instead of a fabricated number: `new file (no history baseline)`, `not a Tier-1 source file`, `binary content`, `file exceeds the AST size limit`, `deleted at gate time`, or `no code-health row at HEAD`. Unmerged (conflict) paths are a tool error — resolve conflicts before gating; a conflict-free in-progress merge or rebase proceeds with a leading note that the projection reflects committed HEAD history. The verdict is recomputed on every call from the repo's current thresholds — a cached report can never serve a stale verdict. This tool reports and never exits; the exit-code-bearing surface with the same engine and semantics is `codelore gate`.

Parameters: none — the change set is discovered from the working tree.

Cost: warm-cache fast; the measured change-set report is additionally memoised by content in a sidecar, so repeated calls on an unchanged dirty tree skip the projection. Cold-cache triggers ingest — and a dirty working tree ingests in-memory without persisting the analysis cache, so it pays that ingest on each cold call until the cache is warmed at the same HEAD (for example by a clean-tree run).

### Architecture note

Each tool call opens its own `FactsDb` connection via the warm-cache path. This is intentional: `duckdb::Connection` is `!Send + !Sync` and cannot cross thread or async boundaries, so each call runs entirely on a dedicated blocking thread (`tokio::task::spawn_blocking`) from connection open to result serialization. The connection is dropped before the future resolves. All tools are read-only with respect to the repository and the fact store; the only writes any tool performs are best-effort sidecar caches — `explain_file` persisting its advisory narrative, and `gate_changes` memoising its measured change-set report.

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
