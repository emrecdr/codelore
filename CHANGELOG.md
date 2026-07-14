# Changelog

Conventional Commits format. All notable changes documented here.

## [Unreleased]

### Added

- **Corpus-relative architecture percentiles on `architecture-metrics`.** When an active calibration artifact carries a `repo_metrics` section (populated by `codelore calibrate`; see below), `architecture-metrics` appends `corpus_percentile:propagation_cost` and `corpus_percentile:cycle_file_share` — midpoint-rank percentiles of this repo's values against the corpus pools — plus `corpus_n`, the number of corpus observations backing them. The base is coarse by construction (one observation per corpus repo, so `corpus_n` is on the order of the corpus repo count): `corpus_n` states it honestly, and the lens reads as "percentile among N corpus repositories", never a fine-grained calibration. Rows are absent entirely when no artifact is active or the active one lacks `repo_metrics` — the existing seven `architecture-metrics` rows are unaffected either way. The embedded world corpus has been rebuilt with these pools (vintage `world-2026-07-14`; 99 repos, 79 of which resolve a non-empty import graph and contribute one observation each), so the percentile rows are live by default with no configuration; `--calibration` overrides as usual. The SPA's Architecture factor tile appends the propagation-cost percentile to its detail line (`, P<nn> of <n> corpus repos`) whenever the rows resolve.

- **`cycle-health` analysis.** New analysis (`--analysis cycle-health`) ranking every non-trivial import cycle (SCC size ≥ 2) by behavioral urgency. Each row reports `heat_pct` — the cycle members' share of repo LOC churn over the trailing `--window-days` window — and a `live`/`fossil` verdict based on whether any member was touched in that window. `extract_candidate` identifies the member whose removal best dismantles the tangle (trial-removal Tarjan minimising the largest surviving SCC; ties by fewest surviving cyclic nodes then lexicographic path). `predicted_pc_drop` gives the whole-graph MacCormack propagation-cost drop if that candidate were extracted, computed for cycles of ≤ 64 members; above that bound the drop is absent (honest absence) and the candidate falls back to the highest in-cycle degree. Outputs: csv, json, markdown.

- **DSM Fusion cell-mode in the SPA.** The Dependency Structure Matrix widget gains a `Fusion` toggle (persisted, default `Structure` = today's rendering, unchanged) that reclassifies each above-diagonal cell by structure×history agreement against the change-coupling data already in the SPA payload, aggregated to the same module depth as the import edges: `agree` (import + co-change, opacity graded by co-change strength), `struct-only` (import, never co-changes, dimmed), and `temporal-only` (co-change with no import edge at all — a modularity violation, in the same amber the architecture force-graph uses for its dashed violation edges). Below-diagonal back-edges stay red in both modes. Every cell's tooltip names its class and a legend row lists all four renderings as text, so the encoding is never color-only. With no coupling data, Fusion mode falls back to the structural view plus a one-line hint instead of a misleading classification.

### Changed

- **`codelore calibrate` builds corpora shallow and HEAD-only.** Each pinned corpus repo is now fetched with a depth-1 fetch of exactly its pinned SHA (with an automatic full-clone fallback when the server disallows shallow SHA fetches — GitHub allows them) and ingested HEAD-only: only the pinned tree's per-function complexity and import facts are extracted, with no commit-history walk and no kamei/clones passes. Corpus builds need a fraction of the previous disk and wall-time; pooled per-function metrics on the same pinned trees are near-identical, not byte-identical: the HEAD tree enumeration is ground truth for the live file set, where the previous history-walk derivation could miss files at the margins of repos with complex histories — across the world corpus the rebuild pools ≤ 0.3% more functions per language, moving a handful of the 1001 quantile breakpoints by about one integer step. The per-repo progress line now reports the checkout mode (`shallow` / `full` / `worktree`).

- **Head-only ingest now extracts import edges too.** Alongside `complexity_metrics`, HEAD-only ingest (`opts.head_only_ingest`) populates the `imports` table — tree-sitter extraction plus the per-language resolver pass — from the same live-at-HEAD file set the complexity pass already scans, no commit walk required. `imports.rev` no longer carries a foreign-key reference to `commits(rev)` (schema v6): head-only ingest never populates `commits`, so the old constraint rejected every head-only import row; no analysis relies on that referential link, since each fact store holds exactly one HEAD snapshot. The cache epoch is bumped so caches written by an older head-only ingest, which carry no import facts, are rebuilt.

- **`codelore calibrate` pools repo-level architecture metrics.** Each corpus repo's resolved import graph now also contributes `propagation_cost` and `cycle_file_share` (fraction of the graph's files sitting in a non-trivial dependency cycle) to the artifact's `repo_metrics` section, populating the optional pools introduced for the corpus-relative architecture percentile lens. A repo whose import graph is empty (no Tier-1 language, or no resolvable imports) contributes no observation for either metric rather than a misleading zero. `--merge` pools these values by exact concatenation across corpora.

### Fixed

- **`--group-file` works again on real repositories.** Any analysis under `--group-file` failed at ingest with `Violates foreign key constraint … is still referenced` on any repository containing modified files — i.e. nearly all real repositories: the grouping swap cleared `changes` while surviving diff-hunk rows (paths kept under their own name) still referenced it, and DuckDB checks the `hunks → changes` foreign key immediately per statement. The swap now snapshots surviving hunks, rebuilds the `changes` and `hunks` tables from the schema (child-first DELETE is not enough — DuckDB also verifies the constraint against index entries of already-deleted rows), and restores the survivors after the grouped rows are in place. Additionally, `code-health` under `--group-file` read complexity columns (`name`, `cyclomatic`, `loc`, `sloc`, `nargs`, `max_nesting`, `bool_ops`) that the grouped complexity rollup did not provide, failing with a binder error; the rollup now carries every column the grouped-source consumers bind, each as the per-group worst-function `MAX`. Group names carry no file extension, so `code-health` ranks groups against groups in the `other` language bucket and reports no corpus percentile for groups. The cache epoch is bumped so previously written grouped caches are re-ingested with the widened rollup.

## [0.17.0] - 2026-07-13

### Added

- **Corpus-relative percentile lens on `code-health`.** Every `code-health` row now carries an optional `corpus_percentile` — where the file's *worst raw complexity dimension* (`cyclomatic`, `cognitive`, `sloc`, `nargs`, `max_nesting`) sits versus a per-language reference corpus, read as a CDF: `P(X ≤ value)`. A `beyond_corpus` flag marks values past the corpus maximum. The lens is additive: a run with no active calibration data leaves every pre-existing field byte-identical. An embedded **world corpus** (vintage `world-2026-07-13`, built from 99 permissive-license OSS repos across the five Tier-1 languages) ships in the binary and activates the lens by default; `--calibration <artifact>` on `analyze`/`check` overrides it. The percentile surfaces in the CLI (`corpus-pct` column), the SPA file drawer, and the `code_health` MCP tool.

- **`codelore calibrate` subcommand.** Builds a corpus-calibration artifact by ingesting a TOML manifest of pinned repos (`--repos`), pooling per-function raw metrics per language, and reducing each pool to a quantile-breakpoint vector (`--output`). `--vintage` labels the artifact (defaults to `corpus-YYYY-MM`); `--merge <existing>` folds a build into an existing artifact via sample-count-weighted quantile blending (an approximation — exact pooling requires re-running over the union manifest); `--cache-dir` overrides the per-repo ingest cache root. Enables organization-specific corpora ("compare against our own codebases").

- **`corpus_percentile_max` quality gate.** `codelore check` fails when any file's `corpus_percentile` exceeds a configured ceiling. When no row resolves a corpus percentile — no calibration artifact is active, *or* one is active but no covered language clears the sample floor and no file's metrics resolve — the gate records a `skipped` verdict (not pass, not fail), mirroring the sidecar-absent skip.

- **Corpus vintage in provenance.** The provenance manifest stamps `corpus_vintage` — the vintage of whichever calibration artifact the lens actually applied (the `--calibration` file when passed, else the embedded world corpus) — so a report records which reference corpus its percentiles were measured against. Absent from the manifest JSON when no artifact is active.

- **`partialFingerprints` on every `codelore diff --format sarif` result.** Diff results now carry `primaryLocationLineHash` (SHA-256 of repo root + path, computed with the exact recipe `codelore check` uses, so a file flagged by both tools collapses to one GitHub Code Scanning alert) and `diffFinding/v1` (SHA-256 of rule id + path + a per-finding discriminant, stable across re-runs of the same diff). Previously diff results carried no dedup fingerprints, causing asymmetric alert deduplication versus check. The primary key is derived from the real `--repo` path, not the internal per-run worktree, so it stays stable across runs. See docs/advanced-usage.md for the key table.

### Changed

- **Three new biomarkers expand the `structural_risk` composite (schema v5).** `code-health` now scores eight structural smells instead of five, adding **Deep Nesting** (per-file max nesting depth), **Many Args** (per-file max argument count), and **Complex Conditional** (per-file max boolean-operator count). The weight table is rebalanced to sum to 1.0 (Complex Method 0.22, God Class 0.18, Large Method 0.12, DRY 0.12, Shotgun Surgery 0.12, Deep Nesting 0.10, Many Args 0.07, Complex Conditional 0.07). This deliberately shifts `structural_risk`, `score`, and `band` values for files exercising the new smells. The fact-store schema bumps from `4` to `5` to persist the new `nargs`, `bool_ops`, and real `max_nesting` complexity metrics; opening a v4 cache triggers a re-ingest.

### Fixed

- **Ingesting a large repository no longer aborts with a foreign-key error.** On high-volume histories (thousands of commits with many changed files and diff hunks), `analyze`/`check`/`diff` could fail during ingest with `append hunk: Failed to append: Violates foreign key constraint …`. The `commits`, `changes`, and `hunks` tables are written through three independent DuckDB Appenders whose buffers perform their foreign-key-checked physical write at different, uncorrelated points; a child buffer could be written while the parent rows it references were still buffered, so the referent was absent. Ingest now flushes the parent tables in foreign-key order at a fixed cadence, guaranteeing every referent is present before any child buffer is written. The fix is invisible below the volume threshold and adds no measurable ingest overhead; small and medium repositories produce byte-identical output.

- **Failed evidence lookups during SARIF emission now warn instead of degrading silently.** When the per-finding commit-evidence query fails (a fact-store error), `codelore check --format sarif` and `codelore diff --format sarif` still emit their results — now without the commit chain — but print a single `⚠` warning to stderr per run so the missing chains are explained rather than silent. Previously the error was swallowed and results lost their chains with no signal.

- **`codelore check --ratchet --format sarif` now emits a SARIF document.** Previously every `--ratchet` code path (initialize, tighten, regression) returned before the SARIF emission, so combining `--ratchet` with `--format sarif` silently printed nothing on stdout — breaking the documented upload-sarif pipeline. All three ratchet outcomes now emit the standard check SARIF document to stdout, identical to a non-ratchet run, with the human-readable ratchet summary routed to stderr. Exit codes are unchanged: a regression still exits non-zero while emitting a valid document.

- **Concurrent readers on the external-findings sidecar no longer fail to open.** The read paths (`check` gate evaluation, the `finding-hotspot-overlap` MCP tool, and any reader) now open the `external-findings.duckdb-ext` sidecar read-only, taking a shared lock instead of DuckDB's exclusive write lock. Running `check`, an MCP call, and `analyze` against the same repo at once no longer contends on that lock. Readers also no longer run schema DDL: a truncated or corrupt sidecar (one missing its findings table) is treated as "nothing to read" — the same as an absent or empty sidecar — instead of being silently re-created as a side effect of reading. Healing the schema remains exclusive to `ingest-sarif`.

## [0.16.0] - 2026-07-11

### Added

- **`codelore check --format sarif`** — emits a SARIF 2.1.0 document to stdout while keeping verdict lines on stderr and exit codes unchanged. A pass produces a valid zero-result document. Each per-file gate violation carries a commit evidence chain (up to 5 contributing commits, newest-first, lineage-aware) in both `relatedLocations` and `codeFlows → threadFlows → locations`, consumable by GitHub Code Scanning and any SARIF-aware CI tool. Results carry two `partialFingerprints` keys: `gateFinding/v1` (stable finding identity across runs) and `primaryLocationLineHash` (used by GitHub for cross-upload alert deduplication). See docs/advanced-usage.md §11.8 for the GitHub Actions upload snippet.

- **`codelore check --cache-dir <PATH>`** — overrides the XDG cache root for the persistent fact-store, gate-run ledger, and external-findings sidecar, matching the existing `analyze --cache-dir` flag. Useful in CI environments that keep per-job caches on a shared runner.

- **SARIF evidence chains in `codelore diff --format sarif`** — CODELORE-HOTSPOT, CODELORE-CLONE, and CODELORE-DELTA-HEALTH results now each carry a commit evidence chain (up to 3 contributing commits per file, lineage-aware) in `relatedLocations` and `codeFlows`, giving PR reviewers direct commit lineage for every finding.

- **`codelore ingest-sarif --repo . <file.sarif> …`** — ingests findings from one or more SARIF 2.1.0 documents into a per-repo sidecar store (`external-findings.duckdb-ext` alongside the fact-store cache, immune to the LRU prune). Re-ingesting the same file is idempotent (replace semantics per engine). Supported dialects: CodeQL, Semgrep, clippy-sarif, and any SARIF 2.1.0 producer.

- **`finding-hotspot-overlap` analysis** — behavioral × static fusion: for each file in the external findings sidecar, reports the total finding count, contributing engines, worst severity level, hotspot score, revision-count percentile (SQL-equivalent `PERCENT_RANK`), code-health band, and a priority label (`act-now` / `plan` / `note`). `act-now` fires when a file has findings, sits in the top-10 % of the revision distribution, and carries a red health band — the intersection of scanner signal and behavioral evidence.

- **`max_findings_in_hot_files` quality gate** — `[gates]` in `.codelore-thresholds.toml` now accepts `max_findings_in_hot_files = <count>`. `codelore check` fails when the number of `act-now` rows from `finding-hotspot-overlap` exceeds the ceiling. Skipped (not failed) when the external findings sidecar is absent or empty.

- **`--window-days <DAYS>`** — shared trailing-window option for activity-scoped
  analyses. Anchored to the repo's last commit date (not wall-clock time) so
  results are reproducible on old or archived repos. Valid range: 1–3650;
  default: 90.
- **`commit_parents` table (schema v4)** — ingest now persists one row per commit parent so graph-topology analyses can query the DAG without shelling out to git. Bumps `CURRENT_SCHEMA_VERSION` to `"4"`.
- **`effort-exposure` analysis** — reports what fraction of engineering activity (commits, LOC churn, SLOC) falls in each code-health band (red / yellow / green) over the trailing window. Answers the hero KPI question: "Are we spending most effort fighting fires or extending healthy code?" Wilson 95% CI on commit-share is included per band. Window anchors to the repo's last commit date via `--window-days`.
- **`Repo::tags()`** — both `GixRepo` and `GitCliRepo` backends now enumerate repository tags; `TagInfo { name, target_rev, date }` sorted ascending by `(date, name)`. Annotated tags use the tagger date; lightweight tags use the target commit's committer date.
- **`max_red_effort_pct` quality gate** — `[gates]` in `.codelore-thresholds.toml` now accepts `max_red_effort_pct = <pct>` (0–100). `codelore check` fails when the red code-health band's window LOC churn share exceeds the ceiling. Missing red band counts as 0 %, which passes any positive threshold.
- **`code-familiarity` analysis** — decayed-knowledge familiarity score and islands percentage for the active team.
- **`code_familiarity_min` quality gate** — `[gates]` in `.codelore-thresholds.toml` now accepts `code_familiarity_min = <pct>` (0–100). `codelore check` fails when the team's SLOC-weighted familiarity score drops below the floor. A value of 0 always passes; 100 always fails unless the whole codebase is actively known.
- **`--knowledge-model {commits|doe}` flag for `bus-factor`** — selects between the default Filatov 2010 commit-coverage mode (`commits`) and the Cury & Avelino SBES'24 truck-factor procedure (`doe`). DOE mode greedily removes the author expert on the most remaining files (per `doe_scores`) until >50% of files lack an expert; `bus_factor` = count of authors removed.
- **Per-file health series and improvements feed** — the SPA now exposes two new data layers populated by the health-trend scan. `file_health_series` records each top-50 hotspot file's composite code-health score and band at every sampled historical revision; the drawer's new **Health** tab renders this as a sparkline for any file in the top-50. `health_transitions` records signal-bearing band changes (enter red → `"regressed"`, leave red or enter green → `"improved"`) across all paths and all sampled revisions, newest-first; the new **Health improvements & regressions** SPA widget renders them as two clickable feed lists with linked-brushing into the drawer.
- **Factor header with XmR attention (SPA)** — a four-tile dashboard header (Code, Architecture, Knowledge, Delivery) above the KPI grid, rendered as part of `--format spa`. Each tile shows a 0–100 headline score and an XmR-gated attention badge that fires only when a signal is statistically unlikely to be noise (last point outside Shewhart limits OR eight consecutive points same side of mean).
- **Banded share bars and effort dot strip** — two HTML/CSS visuals in the Code Health section showing what fraction of LOC and churn landed in each health band (red/yellow/green) over the trailing window, plus a 20-dot strip where each dot represents 5% of window churn. Wilson 95% CI appears as a tooltip on the churn caption. Colors use semantic CSS tokens and rerender on theme toggle.
- **Guided 4-step tour over the hero map** — a martini-glass walkthrough above the hotspot circle-pack. Each step sets the colour lens (Code health → Cognitive complexity → Friction effort → Refactoring targets) and brushes the relevant file set across all widgets. Numbered chips allow direct step navigation; the final step exits to free-form exploration. Respects `prefers-reduced-motion`.
- **`coordination-needs` analysis** — per-file coordination overhead: knowledge fragmentation (HHI complement over decayed knowledge shares), author-switch interleave between chronologically adjacent commits, and co-change graph entropy contribution (EASE 2025, arXiv 2504.18511; window-scoped, commits touching >30 files excluded). Tier classification (`single` / `low` / `medium` / `high`) and code-health band join surface the highest-leverage files: high-fragmentation, high-interleave code in the red band.
- **`team-composition` analysis** — per-author contribution-span tenure buckets (`onboarded` < 90 d, `experienced` 90–364 d, `veteran` ≥ 365 d) with a behavioral veteran-breadth gate (veterans who have not touched a breadth of files comparable to the current 80%-core set are capped at `experienced`) and an onboarding-velocity metric (`onboarding_weeks`: weeks from first commit to entering the weekly 80%-core set, per arXiv 2601.23142). Founder-period authors (first commit within the project's first 12 weeks) receive `NULL` for `onboarding_weeks`. A `__summary__` row reports bucket-percentage breakdown.
- **`marginal-owner-risk` analysis** — ownership concentration × code-health fusion. For each file in the yellow or red health band, reports `top_active_share`: the maximum decayed knowledge share held by any author who committed within `window_days`. Risk tier: `high` (red band AND share < 0.10) or `elevated` ((red AND share < 0.30) OR (yellow AND share < 0.10)); files that do not meet either threshold are excluded. The ownership × code-quality interaction is correlational (Palomba et al., EASE 2023, arXiv 2304.11636). A risk chip appears in the SPA file-detail drawer for flagged paths.
- **SPA Knowledge surfaces widget** — new `widget-knowledge-surfaces` dashboard panel wiring all four knowledge analyses into one view: familiarity and islands-ratio bullet bars (band-coloured via the shared thresholds), a stacked tenure-bucket bar (onboarded/experienced/veteran commit share), and a top-10 coordination-needs table sorted by tier then co-change entropy (click → file detail drawer). Populated from `code_familiarity`, `team_composition`, and `coordination_needs` fields in the `SpaDashboard` JSON blob; degrades to a "No knowledge data" hint when none of the prerequisite analyses produced rows.
- **`release-cadence` analysis** — inter-release gap statistics derived from git tags. Tags matching `--release-tag-glob` (glob syntax, default `v*`) are treated as release markers. Emits one row per matched tag (`date`, `days_since_prev`) sorted ascending, plus a `__summary__` row carrying the median gap, IQR (P75 − P25, linear interpolation), and a trend label: `accelerating` (OLS slope < −0.1 d/release), `slowing` (slope > +0.1 d/release), or `stable` (within ±0.1). Tags are a proxy for releases, not deployments; cadence reflects tagging discipline as much as actual release velocity.
- **`delivery-metrics` analysis** — five repo-level delivery-flow distributions expressed as percentile summaries (p50/p75/p90): `batch_size_files` and `batch_size_loc` (size of each merge unit), `branch_duration_hours` (wall-clock time from earliest branch-side commit to merge), `rework_pct` (hunk-overlap fraction within `--rework-window-days`; approximate), and `lead_proxy_hours` (author→committer date gap on non-merge commits, positive values only). Requires the `commit_parents` table (schema v4) and `--include-merges`. Emits a warning when merge count < 3 and commit count > 50 (squash/rebase workflow likely). `--rework-window-days <DAYS>` (1–365, default 21) controls the rework-detection window.
- **Delivery factor tile in the SPA dashboard** — the four-factor header now includes a Delivery tile when delivery-metrics or release-cadence data is available. The tile deliberately carries no composite score; instead it surfaces three raw git-only proxy numbers: `rework %` (p50, band-colored green < 9 %, yellow 9–14 %, red ≥ 15 % per Pluralsight Flow benchmarks — correlational, not causal), `branch p75 h` (branch_duration_hours p75), and `cadence median d` (inter-release median from release-cadence). The tile is absent when all three sources are unavailable. The SPA delivery card shows the full metric table plus a "where is friction" top-5 drill from delivery-friction. `FactorTile` gains a `numbers: Vec<(String,String)>` field (serde-skipped when empty) for the label–value pairs; `headline` is `Option<f64>` to express the honest shape of a no-composite tile.
- **Gate-run ledger and degraded-result contract** — `codelore check` now appends one `GateRunRecord` per evaluated gate to a per-repo JSONL ledger at `<cache_root>/codelore/<repo_hash_8>/gate_runs.jsonl` after every run. Records carry timestamp, HEAD SHA, gate name, threshold, measured value, verdict (`passed` | `failed` | `degraded`), and invocation mode. A new `fail_on_degraded` field in `[gates]` (default `true`) controls whether a degraded gate — analysis returns no evaluable data where data was expected — counts as a failure. `codelore check --history` prints the last 20 runs grouped by HEAD SHA without triggering a new evaluation. Ledger IO errors warn via `tracing::warn!` and never alter the exit code.
- **`--ratchet` quality snapshot** — `codelore check --ratchet` implements a Betterer-style committed quality baseline via `.codelore-ratchet.toml` at the repo root. First run writes current observed values (`code_health_min_observed`, `red_effort_pct_observed`, `dependency_cycles_observed`) and exits 0 ("ratchet initialized"). Subsequent runs compare against the committed snapshot: any metric worse exits 1 listing regressions; all same-or-better rewrites the file tighter and exits 0, printing which keys tightened. Commit the snapshot file — its git history is then minable as a quality audit trail. Invalid TOML returns a typed error. Ratchet outcomes are also appended to the gate-run ledger with `mode = "ratchet"`.
- **`codelore check --quiet`** — suppresses diagnostic noise (vacuous-pass messages, per-violation detail lines, inline degraded warnings) on stderr while preserving the final verdict line (`✅ PASS` / `❌ FAIL` / `⚠ WARNING`) and all exit codes unchanged. Designed for git hook scripts (`pre-push`, `pre-commit`) where only the machine-readable exit code and final verdict matter. See docs/advanced-usage.md §11.8 for a ready-to-paste hook template.
- **`function-xray` analysis** — per-function change frequency for a single target file. Requires `--target <repo-relative-path>`. For each function or method alive at HEAD, counts the revisions where at least one diff hunk overlapped the function's line span (hunk-overlap attribution from Gall et al. ICSM 2003 HistoryFinder). Outputs function name (deduped as `name@start-end` to handle overloads and recycled names), change frequency, SLOC, cyclomatic complexity, cognitive complexity, and last-changed date. Supports `csv`, `json`, and `markdown` output. Pure deletions (`new_lines=0`) are attributed to the function whose span contained the anchor line. Sorted by change frequency descending.
- **`function-coupling` analysis** — per-function-pair co-change frequency with Fisher exact significance for a single target file. Requires `--target <repo-relative-path>`. For each pair of HEAD-alive functions that co-changed (both touched in the same revision via hunk-overlap attribution) in ≥2 revisions, emits function names (deduped as `name@start-end`), co-change count, per-function change counts, confidence (`co/min(a,b)`), and two-tailed Fisher p-value. Sorted by p-value ascending. Supports `csv`, `json`, and `markdown` output. Research: Adams et al. ICSM 2006.
- **`codelore mcp` subcommand** — starts a Model Context Protocol server over stdio, exposing eight tools: `repo_overview` (repository summary), `hotspots` (top hotspot files by revision count), `code_health` (per-file composite health scores with optional path filter), `delta_health` (function-level health delta between two revisions — accepts any `git rev-parse`-able strings, validates both revs before ingesting, returns verdict/ratio/per-function breakdown), `refactoring_targets` (highest-priority refactoring candidates), `function_xray` (per-function change-frequency and complexity for a given file), `check_gates` (evaluates `.codelore-thresholds.toml` gates at HEAD, returns JSON verdict + violations), and `finding_hotspot_overlap` (behavioral×static fusion: external findings joined with hotspot rank and code-health band; returns structured note when sidecar absent). Each tool call opens its own `FactsDb` via the warm-cache path so the `!Send + !Sync` DuckDB connection never crosses thread boundaries. Read-only — no network, no account, no telemetry. Uses `rmcp 2.2` with stdio transport.
- **X-Ray tab in the SPA file detail drawer** — the file detail drawer now gains an **X-Ray** tab for any of the top-10 hotspot paths for which `function-xray` data was computable at SPA build time. The tab renders a per-function change-frequency table (function name, proportional inline bar coloured red ≥ 80 % / amber ≥ 40 % / grey of the per-file max frequency, LOC, cyclomatic complexity). The tab is absent for paths with no X-Ray data; the Overview "Functions" cognitive complexity sunburst remains as the always-present fallback. The `SpaDashboard` JSON gains a `function_xray` array (omitted when empty via `skip_serializing_if`).

### Changed

- **`bus-factor` CSV and markdown output now includes a `model` column** (`commits` or `doe`) indicating which knowledge model produced the row. Existing `commits`-mode output is otherwise unchanged.
- **KPI tile health bands now match the shared health-band thresholds.** The
  median code-health KPI tile previously used a separate 90/80/70 label scale
  (`healthy`/`fair`/`concern`/`critical`). It now derives its band from the
  centralized `HEALTH_GREEN_MIN` (70) and `HEALTH_YELLOW_MIN` (40) constants
  in the new `bands` module, returning `green`/`yellow`/`red` — the same
  labels used by the health-trend timeline and code-health analysis.

### Fixed

- **SPA delivery card was never rendered** — `renderDeliveryCard` in `30_coupling_trends.js` was defined but never registered in `00_setup_boot.js`'s `WIDGETS` array, and its target element `#widget-delivery-card-body` did not exist in `template.html`. Added the `<section id="widget-delivery-card">` widget section to `template.html` (after the coupling sankey, with subtitle describing the git-only proxy caveat) and registered `{ name: 'delivery-card', rerender: false, render: () => renderDeliveryCard(data) }` in the boot loop. Also added the supporting CSS classes (`delivery-table`, `delivery-value`, `delivery-caveat`, `delivery-friction-header`, `delivery-friction-list`, `delivery-disclaimer`) to the template style block.

- **Theme toggle no longer cascades through layout and offboarding changes.** The
  single `Alpine.effect` in the SPA previously subscribed to `store.theme.isDark`,
  `store.scenario.departed`, and every `store.layout.*` depth setting in one block,
  so a theme switch also chained through depth-tab and scenario subscriptions (and
  vice versa, layout clicks triggered the theme path's CSS-token invalidation).
  The effect is now split: the theme effect reads only `store.theme.isDark` and
  fires registered re-renderers with cooperative yield; a separate layout/offboarding
  effect reads depth and scenario stores without triggering the CSS-token invalidation
  pass. (Layout and scenario changes still re-render all registered widgets; per-widget
  routing is a separate improvement.) The cross-widget selection and brush effects were
  already isolated.

- **Health-trend toggle unreadable.** The `<button id="ht-toggle">` carried
  `class="toggle"` which collided with DaisyUI's global `.toggle` switch
  component, collapsing the button into an unreadable knob blob. Renamed
  the class to `wt-btn` and updated the three matching CSS rules in the
  template (`widget-toolbar .wt-btn`, `:hover`, `.active`).

- **Health-trend overlay renders nothing.** The `#ht-charts` chart host had no
  explicit height, so `echarts.init` operated on a 0-height container and
  produced an empty canvas despite the parent widget having a `min-height`.
  Fixed by adding `#ht-charts { height: 320px; }` to the template CSS.

- **Health-trend split view unreadable** (y-axis labels merged, x-axis labels
  colliding across panels, panels too short). Each panel now uses `height:
  180px` (was 130px), y-axis is fixed to three ticks at 0 / 50 / 100
  (`interval: 50`), x-axis labels are shown only on the last panel, and the
  bottom grid margin is 8px for non-last panels vs 24px for the last.

- **Arch-trend cycles line mistaken for propagation cost.** The Dependency
  cycles series (genuinely 0 in most repos) is now dashed (`lineStyle.type:
  'dashed'`), making it visually distinct from the solid propagation-cost line.
  Both y-axis names are also no longer clipped: `nameGap: 10`, `fontSize: 10`,
  and `grid.left: 24` ensure the full "Propagation %" label renders.

- **Theme-switch stale colors on ECharts widgets.** Seven widgets
  (`arch-trend`, `health-trend`, `arch-graph`, `arch-matrix`,
  `kamei-risk-sparkline`, `parallel-coords`, `cognitive-boxplot`) used the
  cached `token()` color reader but were registered without the
  `rerender: 'theme'` flag that triggers `invalidateTokenCache()` before
  re-render. After a theme switch, those widgets displayed colors from the
  previous theme. All seven now carry `rerender: 'theme'`.

- **Stale SARIF `$schema` URL and `informationUri` in `codelore diff --format sarif`** — the diff SARIF emitter used an `azurewebsites.net` schema URL (a stale mirror) and a placeholder `informationUri`. Both are now sourced from the shared constants in the lib emitter: schema URL `https://json.schemastore.org/sarif-2.1.0.json` (the canonical SchemaStore location), `informationUri` `https://github.com/emrecdr/codelore`.

## [0.15.0] - 2026-07-07

### Added

- `codelore diff` now emits a `delta_health` section: a change-level health
  verdict (`improving`/`indeterminate`/`degrading`) from a 0–100 ratio of
  changed-function weight ending low-risk or improved, with clone-membership as a
  copy/paste penalty and heavier weighting inside red-band files. Two new
  `[diff]` gates: `delta_health_min` and `deny_degrading_verdict`.

- **`health-trend` analysis + SPA timeline.** A new `--analysis health-trend`
  plots three 0–100 scores across up to 12 evenly-spaced historical commits:
  **architectural** health (purely structural — propagation cost plus
  dependency-cycle tangle), **code** health (the rev-parameterized `code-health`
  engine run at each rev with duplication excluded, averaged over files), and
  their equal-weighted **combined** score. Each is banded green (≥ 70) /
  yellow (40–69) / red (< 40). Emits csv/json/markdown; on-demand, never cached
  (roughly `2×` the `architecture-trend` cost). The dashboard gains a
  health-trend widget — an overlaid 3-line chart (combined emphasized) over
  faint red/yellow/green band backgrounds, with a toggle to split into three
  stacked small multiples. The per-rev code score uses the same reduced form at
  every sample (including the newest), so the series is internally consistent;
  its most-recent point is not directly comparable to the standalone HEAD
  `code-health` number, which includes duplication and full external fan-out.

### Fixed

- **`code-health` under `--group-file` now routes complexity through the
  grouped table consistently.** The biomarker pipeline read the ungrouped
  `complexity_metrics` while the composite's cognitive term read the grouped
  table, so grouped entities silently scored `structural_risk = 0` (a join
  miss). Grouped and biomarker reads now share one source. Non-grouped output is
  unchanged.

- **Bumped `crossbeam-epoch` to 0.9.20** to resolve RUSTSEC-2026-0204 — an
  invalid-pointer-dereference advisory in its `fmt::Pointer` impl for `Atomic`
  and `Shared`. A transitive dependency; no API impact.

## [0.14.0] - 2026-07-05

### Added

- **`refactoring-targets` analysis.** A new `--analysis refactoring-targets` ranks files by return-on-investment for refactoring: `priority = (code-health structural_risk × hotspot_score) / max(loc, 25)` — the intersection of low health and high development activity, divided by inspection effort. Each target is annotated with its `dominant_type` (highest-intensity biomarker) and a `manual_up_rank` (the ascending-size "inspect small dense files first" baseline the composite is designed to beat). Supported formats: csv, json, markdown, ndjson, html.

- **Code-health structural-risk fusion.** The `code-health` composite now derives its structural term from five named behavioral-code biomarkers — **Complex Method** (per-language cyclomatic `PERCENT_RANK`), **Large Method** (LOC `PERCENT_RANK`), **God Class** (normalized `god_score` from `run_god_classes`), **DRY** (normalized cloned-function count from `run_clones`), and **Shotgun Surgery** (coupling centrality `PERCENT_RANK` from Fisher-significant pairs). Each smell's intensity is a per-language `PERCENT_RANK` of the file's worst value (ranked across FILES, so the metric spreads instead of saturating), combined as a bounded weighted sum whose per-smell weights sum to 1.0 (absent smells contribute 0, so co-occurrence is implicit). Coupling enters the composite once, as the Shotgun Surgery biomarker — it is not also a separate behavioral term. The retained behavioral terms are churn and ownership fragmentation. Score formula: `100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv)`.

- **Code-health R/Y/G banding and self-relative percentile.** Every `code-health` row now carries a `band` (`red`/`yellow`/`green`, derived from `structural_risk` thresholds 0.55/0.28) and a `percentile` (per-language `PERCENT_RANK` of `structural_risk`, Alves/Ypma/Visser 2010). The complexity, size, god-class, and duplication biomarkers all rank each file against the full per-language file set (a per-file percentile), so the score discriminates across the codebase instead of saturating. Both fields are exposed in all `code-health` output formats (csv, json, markdown, ndjson, html).

- **Code-health determinism gate.** A new integration test (`code_health_v2_is_deterministic`) runs `run_code_health` twice against independent in-memory DuckDB instances ingested from the same fixture and asserts byte-identical `path`/`score`/`band` across both runs. Locks score stability against unordered-aggregate drift.

- **SPA bivariate health×activity map.** The dashboard hotspot circle-pack now defaults to a bivariate color mode: each file's glyph encodes its code-health band (green→red) *and* its development activity (low→high) at once, so the danger quadrant (unhealthy **and** churning) is visible without swapping color lenses. A 3×3 legend keys the encoding; the previous single-signal modes (Cognitive, Code Health, Friction, Author, AI, Knowledge-loss, Clones) remain available as tabs. The palette is colorblind-safe (health read via lightness, not hue alone).

- **SPA linked brushing across all widgets.** Selecting a file in any dashboard view now highlights the same file everywhere at once — the hotspot table row (also flagged `aria-current` for assistive tech), the coupling sankey node, the architecture DSM row/column, the trends and parallel-coordinates series, and the file's coupling arcs on the hotspot map. One shared focus, highlight (not hide) — clearing the selection downplays everything back to neutral. A selection can now be **originated from any of those widgets**: clicking a circle on the map, a sankey node, a treemap cell, or an X-Ray function broadcasts the shared focus (previously only the hotspot table, parallel-coordinates plot, and knowledge-islands list did). The sankey highlight tracks the selected file across module-depth views as well as the default file view. Clicking the map's empty background clears the shared selection everywhere, not just the map's own coupling arcs.

- **Tabbed file-detail drawer.** Clicking a file now opens a drawer split into Overview / Coupling / People tabs instead of one long scroll — the risk summary (hotspot, health, clones, functions, radar) is on the first tab, with change-coupling partners and ownership/contributors one click away. Keyboard-navigable (arrow keys) and screen-reader-labelled.

- **Clustered coupling chord.** The module change-coupling chord now colours each module by its top-level group (falling back to a distinct colour per module in single-root repos), so related modules read as a cluster instead of a uniform ring.

- **Coupling partners are legible on the hotspot map.** Selecting a file now outlines it and its change-coupling partners in blue on the hotspot circle-pack (the file a touch heavier than its partners), and the selected file's tooltip lists those partners with their co-change percentage — so it is clear *which* files are coupled, not just that arcs exist between them.

- **SPA bivariate quadrant set-brush.** Clicking a cell in the hotspot map's 3×3 health×activity legend now brushes every file in that quadrant at once — emphasising them on the circle-pack map (dimming the rest) and marking their rows in the hotspot table — so you can isolate, say, the "unhealthy × high-activity" danger group in one click. The brush is a separate layer from single-file selection: a file can be the focused selection and part of the brushed set at the same time. Click the same cell again to clear. The legend cells are keyboard-focusable and Enter/Space-activatable.

- **Screen-reader announcements for cross-widget selection.** Selecting a file now updates a polite ARIA live region and marks the hotspot-table row with `aria-current`, so assistive-technology users are told which file is focused instead of only seeing the visual highlight.

### Changed

- **`check` gate `code_health_min` now evaluates the composite code-health score** — the value `--analysis code-health` reports — instead of the hotspots inline cognitive-only proxy. Previously a file the analysis banded `red` (composite ~20) could pass a `code_health_min = 70` gate, because the gate read the inline proxy (floored at 60, ~85 for the same file). **Behavioral change** for CI configs using `code_health_min`: the gate is now stricter and consistent with the analysis users look at. `cognitive_max` and `hotspot_score_max` continue to evaluate the hotspots output.

- **`CACHE_EPOCH` bumped to `schema_v8`.** Existing caches are automatically invalidated to reflect the widened `CodeHealthRow` output (new `structural_risk`, `percentile`, `band` columns) and the biomarker-derived scoring change.

- **`codelore explain code-health` updated to v2 formula.** The explain entry now describes the biomarker structural-risk term, the composite weights, the `structural_risk` threshold banding, and the per-language percentile rank.

### Fixed

- **`--format html` is now correctly advertised for `authors`, `top-committers`, `knowledge-islands`, and `clone-coupling`.** These analyses already emitted HTML, but the "unsupported format" error listed only csv/json/markdown, so the option looked unavailable.
- **SPA trends chart no longer keeps a stale highlight when switching files.** The detail drawer is non-modal, so selecting file A then file B without closing it left A's trend line still bold under B (ECharts `highlight` is additive). The trends listener now clears first, so only the current file is emphasised. The coupling-sankey highlight also now fires in module-depth view (previously it matched only full file paths, so it silently did nothing once the sankey was collapsed to modules).
- **Parallel-coordinates plot now visibly reflects the cross-widget selection.** The parallel-coords listener was wired but inert — ECharts emphasis is disabled on that series (a hover-disappears workaround), so a selection produced no visible change. The selected file's polyline now restyles directly (bold, the rest fade), so linked brushing reaches the parallel plot too.

## [0.13.0] - 2026-07-02

### Added

- **Inline PR annotations for quality gates.** When `codelore check` fails inside GitHub Actions, every gate violation is now emitted as a `::error` workflow command, so the failure shows up against the offending file in the PR's Files-changed view — not just as a red check. Per-file gates (`cognitive_max`, `code_health_min`, `hotspot_score_max`) anchor to the file; repo-wide architectural gates (`max_dependency_cycles`, `max_propagation_cost`) emit a file-less annotation in the run summary. Local runs are unaffected (annotations are gated on `GITHUB_ACTIONS`).

### Fixed

- **Change-coupling Sankey + file-detail drawer now show real coupling strength.** The SPA dashboard's coupling Sankey read fields that don't exist on the coupling row, so every band drew at the same width and the "top-30 by strength" ordering was a silent no-op. Bands are now weighted by shared-revision count and ranked by coupling degree, and the drawer shows the real shared-revs + coupling percentage.
- **Config-file read failures now report the input-error exit code (3), not the analysis code (4).** A missing or unreadable `--arch-rules-file`, `--thresholds-file`, or `--group-file` is now categorised the same as `--team-map`, so CI can distinguish "you pointed me at unreadable input" from "the analysis failed". An invalid `--complexity-sample` value now exits `2` (configuration error) instead of `1`.
- **Calendar heatmap renders on flat-activity repos.** A repo where every active day has the same commit count no longer collapses the heatmap's colour scale.
- **Hotspot table shows a "no matches" message** when a filter matches nothing, instead of a blank body; **per-widget "reset zoom" buttons** now appear on the architecture graph and hotspot circle-pack (they were dropped by the async widget boot); the **trends chart** no longer merges files whose abbreviated legend labels collide; and clicking a metric-help **"?"** no longer re-sorts the column.

### Changed

- **Determinism + performance hardening across the engine.** Time-bucketed and `--group-file` ingest now use deterministic aggregates (replacing `ANY_VALUE`); the structural import graph and `cycle-origins` historical graphs are memoised so the architecture analyses and SPA build reuse a single build instead of rebuilding per analysis; `--group-file` mapping bulk-loads through the DuckDB appender; and hot-loop allocations were removed from bot detection and clone-coupling. Dashboard SQL functions gained tracing spans and integration-test coverage. All output-affecting changes are byte-identical or determinism-only.
- **Dropped the vendored `libduckdb-sys` fork.** The MSVC 19.40 build fix (duckdb-rs#786) shipped upstream in `libduckdb-sys 1.10504.0`, so `duckdb` is now pinned to `=1.10504.0` from crates.io and the whole vendoring apparatus is gone — the `[patch.crates-io]` block, `vendor/duckdb-rs/`, `scripts/vendor-duckdb-rs.sh`, the `patches/` file, the `.gitignore` stub-path handling, and the "Vendor patched libduckdb-sys" step in all CI/release/container/bench workflows. Fresh checkouts build with a plain `cargo build`; no pre-build vendor step.
- **Dependency currency.** Bumped `gix 0.84 → 0.85` (differential backend tests pass unchanged), plus routine patch/minor bumps (`clap_complete`, `regex`, `time`, `ignore`, `insta`, `headless_chrome`) and pinned GitHub-Action SHAs.

### CI

- **Dependabot auto-merge for patch/minor bumps.** A new `dependabot-auto-merge.yml` workflow enables GitHub auto-merge (squash) for Dependabot patch + minor updates once all required checks pass, so routine dependency PRs stop piling up. Major bumps are intentionally excluded — they can carry breaking API changes and still open as normal PRs for review.

## [0.12.0] - 2026-06-29

### Added

- **Architectural quality gates.** `codelore check` can now fail CI on structure, not just per-file metrics. New `[gates]` keys `max_dependency_cycles` (e.g. `0` to forbid any import-graph cycle) and `max_propagation_cost` (a ceiling on change-reach density), evaluated via the shared `graph_metrics` kernel. New `[diff]` key `no_new_cycles = true` makes a PR fail when it introduces a dependency cycle the base branch didn't have — computed by comparing the base-rev and head-rev import graphs (`codelore diff` already analyses both revs in worktrees). The "don't let me merge a cycle" guard.
- **`cycle-origins` analysis.** For each dependency cycle at HEAD, binary-searches history — reading + resolving source at past revisions via `Repo::read_blob_at` — to find the commit that first closed the loop. Reports the forming commit's SHA + date and the member files: "the 9-file `rca` tangle formed at `93ea0d1` on 2026-06-06." Commit-level archaeology that `dependency-cycles` (which only shows what's tangled *now*) can't give. Traces the largest cycles first (`log₂(commits)` graph rebuilds each) to bound cost.
- **Architecture-trend SPA chart.** The decay timeline (`architecture-trend`) is now a dual-axis line in the dashboard — propagation cost and dependency-cycle count over the sampled revisions — so the architecture's history is visible, not just CLI-only. Degrades gracefully (empty chart) if the historical scan is skipped.

## [0.11.0] - 2026-06-29

### Added

- **`hotspot-velocity` analysis — change-acceleration early warning.** Hotspots rank all-time churn; velocity asks whether a file is *accelerating*. Per file: `acceleration = recent_per_week − baseline_per_week` over a 30-day recent window vs the 90 days before it, anchored at `MAX(commits.date)` (reproducible, back-testable). Positive = heating up (becoming a hotspot before its all-time count shows it); negative = cooling down. Subtracting per-week rates keeps brand-new files at the top instead of dividing by zero. Pure SQL over the existing tables.
- **`architecture-trend` analysis — structural decay over the commit sequence.** Recomputes propagation cost, dependency-cycle count and largest tangle at up to 12 historical revs (evenly spaced across history), rebuilding the import graph from scratch in memory at each: files-live-at-rev from history, source blobs read at that rev, imports extracted + resolved in memory, then the shared SCC + reachability kernel. Answers "is the architecture decaying, and when did it start?" — structure × history over *time*. The one analysis that re-parses source at past revisions (computed on demand, never cached); needs repository access via the new `Repo::read_blob_at`.
- **`Repo::read_blob_at(rev, path)`.** Generalises blob reads to any revision (`read_blob_at_head` is now a wrapper). Both backends override it (gix `rev_parse_single`, git `git show <rev>:<path>`); differential-tested for byte-identical reads at a historical commit SHA.
- **Layered DAG layout for the architecture-graph SPA widget.** A Force/Layered toggle stacks modules in topological bands so forward dependencies flow downward (arrows) and back-edges (cycles) run upward; dense bands wrap into sub-rows. Reuses architecture-roles' per-file level. De-hairballs medium graphs.

### Changed

- **Dependency-structure-matrix readability.** Square cells (true 45° diagonal), a drawn diagonal guide, and calmer column-only banding so the "triangular = clean" framing is legible on real repos.

## [0.10.0] - 2026-06-27

### Added

- **`crossing` analysis — completes the DV8 hotspot-pattern trilogy.** Flags a structural "X": a file with high fan-in **and** high fan-out (a hub *and* a sink) that co-changes with **both** its importers and its imports, coupling upstream and downstream together *through itself* — the hardest shape to change safely, because edits ripple in both directions at once (Mo, Cai & Kazman 2015 *Hotspot Patterns* / DV8). Fuses the `imports` table (both directions) with `run_coupling`; `crossing_score = coupled_upstream + coupled_downstream`. With `modularity-violations` and `unstable-interface`, CodeLore now ships all three history-fused DV8 patterns.

- **`instability` analysis.** Robert C. Martin's per-file package-coupling metrics (Martin 1994): afferent coupling `ca` (files importing it / in-degree), efferent coupling `ce` (files it imports / out-degree), and **Instability `I = ce/(ca+ce)`** ∈ [0,1] (0 = stable, 1 = unstable). Surfaces Stable-Dependencies-Principle violations — a widely-depended-on file (high `ca`) that is itself unstable (high `I`) is the dangerous shape. Computed from the import graph's in/out degree (no new query cost). Abstractness/Distance ("Zone of Pain") need symbol-level data and are out of scope.
- **`architecture-metrics` analysis.** Repo-level structural-health numbers as `(metric, value)` rows, the kind you trend in CI: **propagation cost** (MacCormack — density of the transitive-closure matrix), Lakos **ACD** / **NCCD** (<1 flat, >1 layered, >2 likely cyclic), dependency-cycle count + largest tangle, and **architecture type** (hierarchical / core-periphery / multi-core; Baldwin/MacCormack 2014). All derived in one pass from the shared import-graph kernel; cross-validates the DSM and `dependency-cycles`.

- **Dependency Structure Matrix (SPA dashboard).** A new "Dependency structure matrix" panel renders the same import graph as a layer-ordered matrix (Steward 1981; Sangal et al. 2005) — the scalable view that doesn't hairball as the module count grows. Modules are ordered by architectural layer (the `architecture-roles` topological level); each cell `(row imports col)` is blue for a healthy forward dependency (above the diagonal) and **red for a back-edge** (below the diagonal) — which only occurs inside a dependency cycle. A clean, acyclic architecture is a triangular all-blue matrix. Shares the depth selector with the force graph.

- **Architecture-graph SPA overlay for the structural analyses.** The dashboard's Architecture graph now colours every module node by its **architectural role** (core = red, control = orange, shared = blue, periphery = neutral), draws a high-contrast **ring** around modules in a dependency cycle, marks **unstable interfaces** as diamonds, and shows the system **propagation cost** + files-in-cycles in the panel title — alongside the existing import edges and dashed modularity-violation edges. Six signals, one picture.
- **`architecture-roles` analysis.** Classifies every file as **Core** / **Shared** / **Control** / **Periphery** from the import graph's transitive "hidden structure" (Baldwin, MacCormack & Rusnak 2014): Core = the largest cyclic group; Shared = depended on as widely as the Core but depends on little (utilities); Control = depends on as much as the Core but little depends on it (orchestrators); Periphery = the healthy leaf bulk. Carries per-file visibility fan-in/out, a topological `level` (longest dependency path from an entry point — the layering depth; back-edges that violate the layering are exactly the dependency cycles), and `reach_pct` (downstream blast radius); the repo-level mean of `vfo/n` is MacCormack's **propagation cost**. Adds `reachability()` + `topo_levels()` passes to the shared import-graph kernel (SCC-condensation + reverse-topological reach-sets — no N×N matrix materialised).
- **`dependency-cycles` analysis.** Finds import-graph tangles — non-trivial strongly-connected components (size ≥ 2) of the resolved import graph, i.e. files that import each other transitively and so can't be compiled/tested/understood/replaced in isolation (Arcan's "Cyclic Dependency" smell; the red diagonal block of a Dependency-Structure-Matrix). Backed by a new shared import-graph kernel (`analyses/import_graph.rs`) with a hand-rolled **iterative** Tarjan SCC (no `petgraph`; iterative so deep import chains can't overflow the stack). Ranked largest-tangle-first.
- **`modularity-violations` analysis — the structure×history fusion.** Surfaces Fisher-significant co-change pairs that have *no* structural import edge between them: the "implicit cross-module dependency" of Mo, Cai & Kazman 2015 *Hotspot Patterns* (DV8). These are files that change together but don't import each other — coupled through a shared global, a leaky abstraction, or a contract honoured through a third party, and empirically more change-prone. Computed by filtering the existing Fisher-significant coupling pairs against the import graph's **transitive reachability** — a pair is a violation only when no directed dependency path connects the two files in either direction (so `a → b → c` chains are not false positives). No new extraction. Available in CSV/JSON/Markdown and the SPA.
- **`unstable-interface` analysis.** Flags heavily-imported files (high afferent fan-in) that change often AND co-change with their dependents, so their instability propagates outward (Mo, Cai & Kazman 2015 *Hotspot Patterns* / DV8). Composite `instability_score = revisions × coupled_dependents`, gated on `fan_in ≥ 3` and `revisions ≥ min_revs`.
- **Architecture-graph fusion overlay (SPA dashboard).** The Architecture graph now fuses git history onto the structural import graph: modularity violations render as **dashed amber "temporal-only" edges** (co-change with no import edge) and unstable interfaces render as **enlarged red nodes**, both rolled up to the same module depth as the import edges. The widget that previously drew only an untyped force layout now shows where structure and history agree — and, more usefully, where they disagree.

## [0.9.3] - 2026-06-26

### Added

- **SPA dashboard accessibility pass.** Tablists now support full keyboard navigation (Arrow / Home / End with roving tabindex), completing the WAI-ARIA Tabs pattern they previously had only the `aria-selected` half of. The 10 canvas / ECharts / d3 charts (trends, sankey, chord, arch-graph, treemap, parallel-coords, boxplot, kamei, calendar, sunburst) now expose a `role="img"` + a data-derived `aria-label`, so screen readers announce a summary instead of an unlabeled graphic. The hotspot filter-summary is now an `aria-live="polite"` status region, and a `@media (prefers-reduced-motion: reduce)` block suppresses the CSS-driven motion (transitions + view-transition crossfades) the existing JS guard didn't cover. Closes F179, F180, F181, F182.
- **`codelore explain` covers six more analyses** — revisions, authors, ownership, code-age, soc, abs-churn — each with a formula derived from the analysis's own SQL, plus an anti-drift test asserting every registered analysis either has an `explain` topic or is on an explicit allowlist (so a new analysis can't ship without explain coverage). Closes F190.

### Changed

- **Format / usage errors now exit with a typed code instead of `1`.** An unsupported `analysis × format` combination, an unknown `--format`, an unknown `explain` / `schema` topic, and `--format html` on an unwired analysis now exit `4` (analysis); `--format parquet` / `sqlite` without `--output` exits `5` (output). CI orchestrators dispatching on exit code can now tell a usage mistake from a real failure. Closes F191.
- **CLI argument-conflict errors use a dedicated `InvalidOptions` error variant** (exit code `2`, unchanged) instead of overloading the provenance-manifest variant, so a `--min-coupling > --max-coupling` typo no longer reads as a reproducibility violation. Closes F199.
- **Contributor tooling aligned with CI.** `just test` now runs CI's non-browser scope (`--features test-support,spa`) instead of `--all-features` (which silently skipped browser tests without Chrome); a new `just test-browser` recipe mirrors the CI `spa-browser` job. Release builds reuse CI's sccache, the `deny.toml` dup-version baseline is made explicit via a `skip` allowlist, and the duckdb-rs vendor script now retries + verifies the upstream commit SHA. Closes F186, F187, F188, F189, F195, F196, F197.
- **SPA: the Hotspots and Trends panels each occupy their own full-width row at every breakpoint** (previously they sat side-by-side at xl ≥ 1280 px via `xl:col-span-1`). They now use `md:col-span-2` so the two largest widgets always get the full width they need; no Tailwind rebuild required (the class was already in the bundle).

### Fixed

- **SPA: the file-detail drawer rendered as a blank popup (black in dark mode) on click.** Root cause: the drawer's content container is a DaisyUI `.modal-box`, which ships `opacity: 0` and is only faded to `1` by a `.modal.modal-open` ancestor — but the drawer deliberately drops the `.modal` class (it over-constrains the right-side positioning), so the populated content was always fully *transparent*. Fixed with an explicit `.detail-drawer .modal-box { opacity: 1 }`. A new headless-browser regression test asserts the `.modal-box` is actually opaque when the drawer opens — every prior drawer test checked DOM content, not pixel visibility, which is exactly how this slipped through. The render path was also hardened: the drawer now opens + populates *before* publishing the cross-widget selection (each isolated in try/catch) and falls back to a stable "File details" title, so a malformed row (no `path`/`entity`) can't produce an empty popup either.

### Performance

- **`changes_lineage` is now materialised once per run** instead of once per lineage-opt-in analysis (12+ rebuilds of the recursive rename CTE + full table copy + indexes under `--use-canonical-lineage`). A per-fact-store guard skips the rebuild after the first; the `--group-file` in-place path swap invalidates the guard so the post-grouping rebuild still happens exactly once. Byte-identical output (verified, including a grouping + canonical-lineage combo test). Closes F184, F194.
- **Lower per-function memory + allocation in ingest, and a fixed SPA listener leak.** The clone fingerprint no longer stores the unused per-node `sequence` vector (held for every function across the whole repo), the imports resolver builds its live-path set once instead of cloning a throwaway `&str` set, and the SPA cross-widget selection listeners deduplicate by source so re-rendering trends / parallel-coords no longer leaks a closure over a disposed chart. Byte-identical analysis output. Closes F183, F185, F193.

## [0.9.2] - 2026-06-25

### Fixed

- **SPA: Hotspots and Trends panels now stack on narrow screens and sit side-by-side on wide screens** (the responsive behaviour was inverted). Both carried `xl:col-span-2`, which made them share a row at md/lg widths (768–1279 px) yet each span the full width — stacked — at xl (≥ 1280 px). They're now `md:col-span-2 xl:col-span-1`: full-width and stacked up to lg, side-by-side only once there's room at xl. The Tailwind bundle was rebuilt to emit the new `md:col-span-2` / `xl:col-span-1` classes. Verified in a headless browser via measured layout (wide: two half-width columns; narrow: two stacked full-width rows).
- **`codelore schema <analysis>` no longer rejects three valid analyses.** The supported-row-type list was a hardcoded 29-entry array that had drifted from the 32-variant `AnalysisName` registry, so `delivery-friction`, `main-dev-by-revs`, and `main-dev-by-deletions` were reported as "unknown row type" despite being fully supported. The list and its printed count now derive from `AnalysisName::all()`, so it can't drift again. Closes F166.
- **`stale-code` and `delivery-friction` are now deterministic across runs.** Both anchored their staleness / WIP-age arithmetic to `OffsetDateTime::now_utc()`, so an identical repo + HEAD + cache produced different output second-to-second — breaking reproducibility and quality-gate stability. They now anchor to the newest commit date in the fact store, honouring `--age-time-now` when set, exactly like `code-age` / `knowledge-islands`. Closes F167.
- **`lead-time` rows under `--rows N` are now deterministic.** The final `ORDER BY lead_time_seconds DESC` had no tiebreaker, and lead times are heavily tie-laden (squash / rebase workflows produce bulk zeros), so which rows survived `LIMIT` depended on DuckDB scan order. Added `, rev ASC`. Closes F168.
- **`bus-factor` no longer silently drops repo-root files.** An `AND c.path LIKE '%/%'` filter excluded every file with no `/` (e.g. `README.md`, `Cargo.toml`, `justfile`), skewing or emptying the report on flat repos; root files now aggregate into a `<root>` bucket. bus-factor also opts into the rename-aware lineage view under `--use-canonical-lineage`, consistent with every other path-aggregating analysis. Closes F171.
- **SPA: the file-detail drawer is now accessible.** It had no accessible name and relied on browser-version-specific native non-modal `<dialog>` focus behaviour. It now carries `aria-labelledby`, moves focus into the drawer on open, and restores focus to the trigger row on close (guarding a trigger removed by a re-render). A headless-browser regression test asserts the contract. Closes F175.
- **SPA: the treemap breadcrumb is themed again.** It read an undefined `--bg-elev-1` CSS variable (only `--bg-elev` / `--bg-elev-2` exist), so the breadcrumb fell back to an unthemed default; corrected to `--bg-elev`. Closes F169.
- **SPA: the calendar heatmap no longer risks a `RangeError` on long histories.** It computed min/max via `Math.min.apply(null, counts)`, spreading one argument per active day; a multi-year repo (thousands of days) could overflow the call-stack argument limit and throw, blanking the heatmap. Replaced with a single-pass loop. Closes F172.
- **Dashboard SQL failures now exit 4 (analysis), not 5 (output / I/O).** Six SQL-driven analyses behind the SPA dashboard (`xray`, `clone-summary`, `trends`, `daily-commits`, `kamei-risk`, and the architecture-graph imports query) lived in the output layer and wrapped their query failures as `CodeLoreError::Output`, mislabelling an analysis failure as I/O. They moved into a new `analyses::dashboard` module and now return `CodeLoreError::Analysis`, so CI orchestrators dispatching on exit code see the correct bucket. SPA output byte-identical. Closes F176.

### Security

- **CSV output now guards against spreadsheet formula injection.** A cell whose first character is `=`, `+`, `-`, `@`, or a tab — reachable via attacker-influenceable git strings such as author names and paths — passed through verbatim and would execute as a formula when the CSV is opened in Excel / Google Sheets. Such cells are now force-quoted and prefixed with a `'` literal-text guard, composing with the existing RFC-4180 quoting. Closes F170.

### Changed

- **`codelore profile` derives the schema version instead of hardcoding it.** The profile output carried a literal `schema_v3` that would silently misreport after the next schema migration; it now interpolates `facts::schema::CURRENT_SCHEMA_VERSION`. The cache-invalidation sentinel was also renamed `SCHEMA_VERSION` → `CACHE_EPOCH` (value unchanged, so no cache is invalidated) to name it honestly as a manual cache-buster distinct from the on-disk schema version. Closes F177.
- **`--format spa` no longer requires `--output`.** When omitted, the dashboard is written to `.codelore/spa.html` under the current working directory (the `.codelore/` directory is created if missing); `--output PATH` still overrides it. So `codelore analyze --format spa --repo .` now just works instead of erroring. `--format parquet` and `--format sqlite` still require `--output` — they're binary fact-store dumps with no sensible default filename.

### Performance

- **`run_coupling` is memoized per fact-store, eliminating 2–5× redundant recomputation.** The change-coupling analysis — an O(K²) `filtered_changes` self-join plus a Rust-side Fisher-exact pass — is pure for a given `(fact store, options)` yet was recomputed by every caller: `code-health`, `centrality`, `communities`, `clone-coupling`, and the SPA dashboard builder. A single `--format spa` or multi-analysis run therefore paid the most expensive analysis query several times over. A per-`FactsDb` `RefCell<HashMap<CouplingMemoKey, Rc<Vec<CouplingRow>>>>` memo — keyed on every coupling-affecting option (changeset / revs / coupling-percent bounds, `fisher_significance`, `time_bucket`, `use_canonical_lineage`, `code_maat_compat`), with `--rows N` applied *after* the lookup so a row cap can't poison the shared entry — returns the cached result on repeat calls. `RefCell` + `Rc` matches the `!Send` single-connection ingest model. Output byte-identical across coupling / code-health / centrality / communities / clone-coupling; the gix-vs-cli differential gate stays green. Closes F174.

## [0.9.1] - 2026-06-23

### Fixed

- **Dependabot's weekly Cargo job no longer fails on the vendored `libduckdb-sys` patch.** The workspace `[patch.crates-io] libduckdb-sys` resolves to a vendor tree that is git-ignored and materialised at build time by `scripts/vendor-duckdb-rs.sh`; Dependabot's updater container never runs that script, so `cargo metadata` could not read the patched manifest and every major-bump trigger failed (2026-06-15, -21, -22). A minimal stub — the manifest plus a stub `src/lib.rs` (~10.6 KB) — is now committed so `cargo metadata` resolves the patch path; the full vendor tree still overwrites it with byte-identical content at build time, so builds and `git status` are unaffected. `.gitignore` switches from a blanket `vendor/` to `vendor/*` with leaf negations (a parent-excluded path can't be re-included), and the vendor script now strips the nested `.git/` so the stub files are trackable. Unblocks automated dependency updates.

### Changed

- **CI/release GitHub Actions bumped.** `actions/checkout` 6 → 7 (kept as a floating `@vN` tag, per the policy of pinning only credential-handling actions) and `softprops/action-gh-release` 3.0.0 → 3.0.1 (SHA-pin refreshed to `718ea10b… # v3.0.1`, preserving the `@<40-char-sha> # vN` form). Maintenance bump across `ci.yml`, `bench.yml`, `container.yml`, and `release.yml`; released binaries are unchanged.

## [0.9.0] - 2026-06-23

### Added

- **CI: `dogfood` job runs CodeLore against CodeLore on every PR + main push.** Until now, nothing in the workflow surfaced what the analyzer thought of its own changes — a behavioural-analysis tool with no behavioural signal on its own commits. The new `dogfood` job builds release `codelore-cli --features spa`, then runs `codelore analyze --analysis hotspots --format gha --repo .` so hotspots stream into the PR's Checks panel as inline annotations via the GHA workflow-command emitter (`::warning::` / `::notice::` per the existing `output::gha` bucketing). The same step writes a markdown summary (top hotspots / code-health worst-10 / knowledge islands) into `$GITHUB_STEP_SUMMARY` so reviewers see CodeLore's verdict inline on every PR. PR events additionally run `codelore diff "origin/${{ github.base_ref }}...HEAD" --format markdown` and append the delta. `continue-on-error: true` during the bake-in period so the job surfaces signal without gating merges while thresholds are still calibrating — drop the flag once output stabilises across a few releases. Uses sccache + rust-cache for sub-30s incremental runs. Closes F144.
- **`--format ndjson` extended to `code-health`, `coupling`, and `lead-time`** (alongside the existing hotspots wiring). The 3 most-commonly-piped behavioural analyses now stream as newline-delimited JSON for `jq -c` / LSP / CI log consumption.
- **SPA: treemap semantic-zoom drill-down.** The hotspot treemap widget now supports click-to-drill into directory subtrees via ECharts' `leafDepth: 2` + native breadcrumb. Per-depth `levels[]` styling adds progressive border + gap thickness so directory vs file cells read distinctly. Spec: [Apache ECharts treemap-drill-down example](https://echarts.apache.org/examples/en/editor.html?c=treemap-drill-down). Zero bundle delta — uses the already-pinned ECharts 6.1.0.
- **`--format ndjson`** — newline-delimited JSON output for `codelore analyze`. Each row is emitted as its own line of compact JSON (no enclosing array), so LSP integrations, `jq -c` filters, and CI log pipelines can stream-parse as analyses complete instead of waiting for the closing `]`. Spec: <https://github.com/ndjson/ndjson-spec>. Wired for hotspots (Plan 9 will extend to all analyses); same `HotspotRow` shape as the batch JSON emitter, only the framing changes.
- **`--format gha`** — GitHub Actions workflow-command output for `codelore analyze --analysis hotspots`. Each hotspot becomes one `::error file=...,title=...::msg`, `::warning::`, or `::notice::` line on stdout, bucketed by composite `hotspot_score` (≥ 7 = error, ≥ 4 = warning, otherwise notice). When run inside a GitHub Actions job the runner surfaces each line as an inline annotation on the pull-request diff — same surface Code Scanning uses, but with no SARIF upload, no API call, no `security-events: write` permission required. Property values are escaped per the [official workflow-commands spec](https://docs.github.com/en/actions/reference/workflow-commands-for-github-actions) (`%` → `%25`, `\r` → `%0D`, `\n` → `%0A`, plus `:` and `,` inside property fields).
- **`delivery-friction` analysis (analysis #32).** Composite signal answering "where is technical debt actively slowing us down right now?". Combines `percent_rank(revisions) × percent_rank(median_lead_time) × percent_rank(cognitive)` scaled to `[0, 100]` — a file scoring high requires elevation on all three axes; one dominant signal alone does not light it up. Each row carries the underlying values plus `p95_lead_time_days` (right-tail surfacing) and `wip_age_days` (days since the file's last commit, distinguishing stale-but-still-touched from hot-but-recently-active). Counters CodeScene v7.4's Delivery Analysis surface while staying SQL-driven, CLI-only, and citation-grounded. CSV / JSON / Markdown emitters; SARIF NOT included (not a defect-risk surface).

### Fixed

- **`--format ndjson`/`gha` on an unsupported analysis now errors cleanly instead of panicking.** Both formats pass top-level format validation but are only wired for a handful of analyses (ndjson for hotspots/code-health/coupling/lead-time; gha for hotspots). For every other analysis the per-analysis dispatch fell through to an `unreachable!` and **panicked** (exit 101, "internal error: entered unreachable code") — 22 analysis×format combinations, e.g. `codelore analyze --analysis abs-churn --format gha`. The 11 affected `dispatch_*` functions now bail with a descriptive error listing the formats that analysis supports (`"abs-churn analysis supports csv|json|markdown; got \"gha\""`, exit 1), matching the convention the other dispatch functions already used. A regression test asserts the clean exit code and the absence of a panic in stderr for both formats. Closes F165.
- **`--team-map FILE` unreadable now exits with code 3 (input error), not 5 (output/I/O).** New `CodeLoreError::RepoIo(io::Error)` variant carries read-side input I/O failures into the spec §6.6 exit-3 bucket alongside `Repo` and `BlobNotFound` — same bucket as "the repo path didn't exist". The write-side `CodeLoreError::Io` variant (used by every output emitter via `writeln!(...).map_err(CodeLoreError::Io)`) is unchanged and still exits 5; the new variant is opt-in (no `#[from]`) so generic `?` propagation can't accidentally pull a write-side error into the input bucket. CI orchestrators that dispatch on exit code can now distinguish "fix the input you pointed me at" (3) from "something went wrong on output" (5). Closes F153.
- **SARIF `$schema` URL points at the canonical SchemaStore host.** The constant in `sarif.rs` swapped from `https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0.json` (Microsoft's legacy `azurewebsites.net` origin) to `https://json.schemastore.org/sarif-2.1.0.json` (the canonical SchemaStore CDN). All three SARIF emitters (hotspots, clones, live-clones) inherit the fix through the shared constant. Closes F120 (URL half).
- **Group-level cognitive + MI aggregation under `--group-file`.** Pre-fix,
  `hotspots`, `code-health`, `god-classes`, and `stale-code` silently
  reported `0` cognitive (and NULL MI for `hotspots`) on every grouped
  entity. Root cause: `apply_grouping` rewrote `changes.path` to the
  group name but `complexity_metrics.path` stayed at raw file paths, so
  the `LEFT JOIN file_complexity fc ON fc.path = c.path` never matched.
  `apply_grouping` now also materialises `complexity_metrics_grouped`
  with per-group `MAX(cognitive)` + `MAX(kind='unit' MI)` rolled up,
  and the four analyses route through
  `analyses::grouped_complexity::source_table` to pick the rolled-up
  table when grouping is active. The table is permanent so it survives
  cache replay (`FactsDb::open_read_only` opens a fresh connection that
  can't see the connection-scoped `_grouping_v1` temp table).

### Performance

- **Ingest: producer→consumer channel capacity now sweepable; new `ingest_capacity_sweep` Criterion bench.** `CHANNEL_CAPACITY = 64` was folklore — no empirical measurement existed. The constant is replaced with `DEFAULT_CHANNEL_CAPACITY: usize = 64` plus a process-wide `CHANNEL_CAPACITY_OVERRIDE: AtomicUsize` static and a `pub fn set_channel_capacity_override(n)` write hook. `channel_capacity()` reads override-else-default on each ingest call (single atomic load — negligible against producer/consumer setup cost) and `bounded::<CommitEvent>(channel_capacity())` consumes the runtime value. The new `ingest_capacity_sweep` bench in `benches/end_to_end.rs` uses `BenchmarkId::from_parameter(cap)` to sweep `[16, 64, 256, 1024]` on the medium fixture in one `cargo bench` invocation; the override is reset to `0` (= fall back to default) at sweep end so any later bench in the same invocation isn't surprised. Avoids `unsafe { env::set_var }` (workspace `unsafe_code = "forbid"` blocks that) and avoids expanding the public CLI surface — production dispatch never touches the override. Closes V6.
- **`pair-programming`: integer-interned pair counter eliminates `String::clone` per inner-loop probe.** The pair-counting `HashMap<(String, String), u32>` was replaced with `HashMap<(u32, u32), u32>` backed by a per-run author interner (`HashMap<String, u32>` + `Vec<String>` table). Each author identity is allocated as `String` exactly once at first encounter; all subsequent pair lookups hash pure integer-pair keys. The per-commit `participants: Vec<String>` is also replaced with a reusable `Vec<u32>` scratch buffer (`clear()` keeps the allocation across the commit loop). On repos with heavy pair-programming (~100 commits per pair), the prior shape allocated ~200 redundant `String`s per pair just to discover the pair was already counted; the new shape allocates each author once, period. Canonical lex ordering of output rows (`author_a` < `author_b`) is recovered at output time. New regression test `pair_programming_dedupes_pair_regardless_of_primary_orientation` guards the dedup invariant across primary/co-author orientation swaps. Closes F130.
- **`arch-violations`: stream-validate with early-break on `--rows N`.** Previously the analysis pulled every `imports` row to Rust via an intermediate `Vec<(String, String, String)>`, validated every row against the layer-rules config, then `Vec::truncate`'d to the row-cap after the fact. Now the rows iterator is walked directly without the collect, and the validation loop breaks as soon as `out.len() >= opts.rows_limit`. SQL's `ORDER BY src_path ASC, target_path ASC` is preserved so the first N violations are deterministic and match what the prior shape produced — early-break is byte-identical for any non-zero row cap. On a monorepo with millions of imports and `--rows 50`, validation stops after finding the first 50 violations instead of validating every row. Closes F129.
- **Observability: `#[tracing::instrument]` on all 32 `run_*` analysis entry points.** Each span carries `name="<analysis-name>"`, `skip_all` (drops the `db` and `opts` debug-bloat), and `fields(min_revs = opts.min_revs)` as a structured field — the input gate that explains "I ran hotspots and got zero rows". Operators get per-analysis wall-clock + busy/idle breakdown via `RUST_LOG=codelore_lib::analyses=debug` (or `=info` for span open/close only). Verified end-to-end: `hotspots{min_revs=1}` emits with `time.busy=6.87ms time.idle=2.25µs` on a tiny fixture. Closes F142.
- **Kamei `enrich_diffusion` entropy block rewritten from correlated subquery → 2-pass grouped UPDATE** (reset to 0.0 + single hash-joined UPDATE with window-function `p_i = loc_added / SUM(loc_added) OVER (PARTITION BY rev)`). Mirrors `enrich_history`'s shape: DuckDB walks `changes` exactly once per ingest instead of re-scanning per `commits.rev`. Byte-identical semantics validated via a new `kamei_entropy_per_commit_distribution` regression test with 3 hand-computed cases (single-file commit = 0.0, even 2-way split = log2(2) = 1.0, uneven 3-way reference case = 1.2987949...). Closes F127 — the NS/ND/NF triple was collapsed in PR #64; the entropy remainder is now also closed.
- **SPA: cooperative scheduling for the hotspot table's `Show all` rebuild and the theme-toggle rerender cascade.** Both used to block the main thread for hundreds of ms on large repos (one 5000-row HTML string + one `insertAdjacentHTML` + one `querySelectorAll` over the full table for `Show all`; the full d3.pack re-layout + every ECharts widget's `setOption` running synchronously for theme toggle). `widgets.js` now ships a `yieldToMain()` primitive that prefers `scheduler.yield()` (Chrome 129+, continuation-prioritised) and falls back to a `MessageChannel.postMessage` trick on browsers without it (the postMessage path beats `setTimeout(0)` because it isn't clamped to 4ms and runs at the same priority as input). `renderNextPage` is now `async` and walks in 50-row chunks with `await yieldToMain()` between each; the template's `Alpine.effect` rerenderer loop yields between each registered widget. Closes F134 + F135.

### Security

- **Supply-chain: `fishers_exact` crate removed; Fisher's exact two-tail p-value ported in-tree.** The upstream `fishers_exact` crate (`v1.0.1`, last release 2018-11) was unmaintained for 7+ years — no live CVE in the advisory database, but the longer the gap, the larger the unmonitored attack surface for a dep that runs in every coupling analysis. New `crate::stats::fisher_two_tail_pvalue` module (~150 LOC) computes the hypergeometric tail in log space via `ln_factorial`, summing PMFs of every 2×2 table whose probability is ≤ the observed table's. The single call site in `analyses::coupling::fisher_two_tail` swapped to the new function. Numerical agreement vs. the upstream crate: ≤ 1e-12 relative error across 8 regression cases captured by running both implementations on the same inputs (`[1,2;3,4]` → 1.0, `[8,1;2,5]` → 3.4965e-2, `[1,9;11,3]` → 2.7595e-3, `[10,5;5,10]` → 1.4311e-1, `[0,5;5,0]` → 7.9365e-3, `[100,50;50,100]` → 1.1382e-8, `[1,0;0,1]` → 1.0, `[50,50;50,50]` → 1.0). Plus a 6×6×6×6 exhaustive bounds check (`fisher_pvalue_is_bounded`) and a degenerate-marginal `None`-return test. Closes F121.
- **Supply-chain: Container base images pinned to immutable `@sha256` digests.** Both `Containerfile` `FROM` lines previously pulled mutable tags — `rust:1.96-bookworm` and `gcr.io/distroless/cc-debian12:nonroot` — and both Docker Hub and gcr.io rebuild floating tags on a regular schedule, so an unpinned build today was NOT guaranteed to be byte-identical to the same Containerfile built yesterday. Digest pins now sit INLINE on the `FROM` instructions (not via ARG — Dependabot/Renovate parsers don't resolve ARG substitutions): `rust:1.96-bookworm@sha256:19817ead...` for the builder, `gcr.io/distroless/cc-debian12:nonroot@sha256:b0ae8e98...` for runtime. Digest bumps are tracked by Renovate (Dependabot's docker ecosystem only detects `Dockerfile`/`*.Dockerfile` and skips `Containerfile`) — `renovate.json` gains a `dockerfile.managerFilePatterns: ["/Containerfile/"]` config and a `matchManagers: ["dockerfile"]` package rule grouping all container-base bumps into one weekly PR. Reproducibility, cosign / SLSA provenance attestation, and CVE diffing now work as intended. Closes F115.
- **Supply-chain: SPA build now resilient to jsDelivr availability incidents.** Every vendored SPA asset (`echarts.min.js`, `d3-hierarchy.min.js`, `alpine.min.js`, `alpine-persist.min.js`) gains a `url_fallbacks` mirror in `AssetPin` — the `unpkg.com` equivalent of each jsDelivr URL. `fetch_and_pin` walks primary→fallbacks in declaration order. Both CDNs pull from the same npm registry, so the bytes are identical and the same SHA-256 validates whichever mirror responds. SHA-256 mismatch on ANY URL is a hard fail (not "skip to next mirror") — a tampered mirror can't be silently replaced by the next mirror's clean bytes; the SHA-pin discipline still catches drift everywhere it happens. A jsDelivr DNS outage / regional block / rate-limit no longer breaks every downstream `cargo build --features spa`. Closes F114.
- **Supply-chain: SHA-pinned the 5 credential-handling GitHub Actions across `container.yml` + `release.yml`** — `actions/attest-build-provenance` (issues OIDC token for SLSA provenance signing), `docker/login-action` (consumes `GITHUB_TOKEN` for ghcr.io auth), `docker/build-push-action`, `docker/metadata-action`, `docker/setup-buildx-action`. Each pin uses the canonical `<repo>@<40-char-commit-sha> # vN` shape so Dependabot can still suggest bumps and human reviewers see the human-readable version alongside the immutable anchor — same shape `softprops/action-gh-release` already used. 8 use-sites across both workflows updated. Non-credential actions (`actions/checkout`, `actions/cache`, build-related sccache + rust-cache, etc) deliberately left as `@vN` — pinning them too would balloon Dependabot's weekly bump surface without commensurate attack-surface reduction; F117's "credential-handling subset" framing intentional. Closes F117.

### Changed

- **Task-ID (`F<NN>`) references stripped from all code comments.** Comments across the library and CLI (both `src` and `tests`) referenced internal finding/audit IDs — `F33 fix:`, `(F121)`, `Pre-F29`, `F12 invariant`, `F107/F108` — that are meaningless without the findings report and rot as findings close. Every such reference was removed from `.rs` comments in `codelore-lib/{src,tests}` and `codelore-cli/{src,tests}`, keeping each comment's rationale intact (and, where an ID named a concept, replacing it with the concept — e.g. `F12 invariant` → `rowid-ASC invariant`, `same F16 pattern` → `same live-at-anchor pattern`). Comment-only: no code, string, identifier, or SQL changed. The vendored `codelore-rca` MPL fork, benches, and Markdown docs (CHANGELOG and the findings report, which legitimately track F-IDs) were left untouched. Code comments now describe the current contract directly, per the project convention that history lives only in this changelog. A new test (`comment_hygiene_test.rs`) scans the library and CLI source/test trees and fails the gate if any such token reappears, so the convention can no longer silently regress. Closes F164.
- **CLI dispatch collapsed into per-analysis functions, behind a new `codelore_lib::cli_api` façade.** Two coupled refactors of `codelore-cli`, shipped together and proven output-preserving:
  - The ~1200-LOC `match (format, &analysis)` in `analyze()` — a flat 2-D routing table with no abstraction — is replaced by a 1-D `match &analysis` that delegates to one `dispatch_<analysis>` function per analysis. Each runs its analysis then matches `format` to the right emitter; SARIF's repo-root and HTML's title/repo/generated-at travel in a small `EmitCtx`; the former standalone HTML pre-branch is folded into the same per-analysis functions (sharing one `html_not_wired` error helper). Adding or changing an analysis is now localized to one function instead of threaded through a giant match.
  - `codelore_lib::cli_api` is introduced as the single surface the CLI imports through — it re-exports the modules and root types the binary needs, and every CLI reference now routes through it (`grep 'codelore_lib::' crates/codelore-cli/src | grep -v cli_api` is empty). Internal library modules stay `pub` (the integration-test crate needs deep access), so the façade is additive and non-breaking: the CLI↔library contract is now enumerated in one place.
  - **Output is byte-identical.** Verified across all 228 analysis×format pairs on this repo: exit codes match exactly (including pre-existing behaviour), and the only stdout differences are environmental — clone detection seeing the new dispatch functions in the working tree, the wall-clock-relative `wip_age_days` column, and the per-run SARIF `run` id. Closes F145 and F113.
- **`facts/ingest.rs` (1523 LOC) split into a `facts/ingest/` directory module.** The monolith is now seven topical files: `mod.rs` (the `FactsDb::ingest` entry point, channel-capacity controls, `IngestStats`, the shared `current_head_rev`/`query_live_paths` helpers, and `format_panic_payload`), `complexity_head.rs` / `clones_head.rs` / `imports_head.rs` (the three rayon-then-serial-drain HEAD-time enrichment passes plus import resolution), `consumer.rs` (the connection-owning `ingest_loop` pump and its `append_*` row writers — the half of the producer/consumer split that owns the `!Send` `Connection`), `lineage.rs` (the canonical path-lineage CTEs), and `grouping.rs` (`apply_grouping` plus bucketed/grouped materialisation). Pure code movement, zero behaviour change — verified by an identical 26-function inventory and a normalized content diff showing only `pub(super)` visibility widening (so the `ingest` entry point can still call the methods relocated into child submodules) and path-qualifier adjustments forced by the module-depth change. Every external path contract is preserved: `materialize_changes_lineage` / `materialize_changes_bucketed` / `materialize_path_lineage` / `apply_grouping` are re-exported from `mod.rs`, and `IngestStats` / `set_channel_capacity_override` / `format_panic_payload` stay defined there, so consumers in `analyses`, `kamei`, `repo`, and the benches see no path change. Full test suite (663) passes unchanged. Closes F94.
- **`FactsDb::conn()` tightened to `pub(crate)`; three narrow safe methods added.** Pre-fix, `pub fn conn(&self) -> &Connection` handed the raw `duckdb::Connection` to any external consumer of `codelore-lib`. Because DuckDB's `Connection` mutating methods (`execute`, `execute_batch`) take `&self`, a shared `&Connection` was enough to run arbitrary SQL straight against the store — bypassing the invariants `FactsDb` maintains (schema-version stamp, append-only ingest path, the curated query surface). `conn()` is now `pub(crate)`: the rest of the crate (kamei, quality_gates, `output::spa`, ingest, etc.) still reaches the underlying connection for `Appender` / multi-statement work, but external consumers can't. Three narrow safe methods cover every legitimate external need — `prepare(sql)` for the `prepare → query_map → collect` multi-row pattern, `execute_batch(sql)` for fixture DDL/DML, and `query_row(sql, params, mapper)` for single-row scalar reads — each wrapping the DuckDB error in `CodeLoreError::Analysis` (exit 4) so SQL failures share the analysis-error exit code. All 9 external `.conn()` call sites (every one in `tests/`, across 5 files) migrated to the safe methods; the two multi-query `let conn = db.conn();` bindings in `imports_factsdb_test.rs` expanded to direct `db.query_row(...)` calls. The CLI has zero `.conn()` uses, so production callers see no API change — the finding was API hygiene, not breakage. Closes F111.
- **Schema v4 — `hunks` table now actually populated; tightened to NOT NULL + composite PK + `(rev, path)` index.** Pre-v4, `Repo::diff_hunks` was parsed in `GitCliRepo` but stubbed to `Ok(vec![])` in `GixRepo`; the walker constructed every `FileChange.hunks` as `vec![]`; `append_change` never wrote a single row. The `hunks` table existed in schema, was defensively `DELETE`-cleaned by `apply_grouping` (no-op on an empty table), and dumped by the SQLite emitter — all over an empty table. Now wired through end-to-end: `count_loc` extended to `count_loc_and_hunks` which walks `imara_diff::Diff::hunks()` from the SAME histogram diff already running for `loc_added`/`loc_deleted` (no extra blob read, no second pass); `GixRepo::diff_hunks` resolves before/after blob OIDs via the new `blob_at_path` helper + calls the shared extractor (root-commit-safe via `Option<ObjectId>` empty-side handling); `compute_changed_files` populates `FileChange.hunks` in the Modification arm; `append_change` writes one hunks row per `FileChange.hunks` entry. Schema bump: `CURRENT_SCHEMA_VERSION 2 → 3` (provenance stamp), `types::SCHEMA_VERSION 3 → 4`, `cache::SCHEMA_VERSION schema_v4 → schema_v5` (cache invalidates naturally). Hunk-header conversion matches git's `--unified=0` convention (1-indexed start for non-empty sides, 0-indexed start for empty sides) so the differential test `diff_hunks_match_across_backends` asserts gix == cli hunks byte-for-byte across README/Cargo.toml/CHANGELOG. New regression test `ingest_writes_hunk_rows_to_hunks_table` makes two non-adjacent edits and asserts ≥2 hunks land with zero NULL offsets. ~80 LOC net change (vs the audit's initial M-not-L estimate — recon revealed the gix-diff API exposed `Diff::hunks()` for free). Closes F149.
- **Bump `toml` 0.8 → 1.x** (workspace-wide). The 1.0 release split the `parse` (low-level parser) and `serde` (`from_str` / `Deserialize` glue) features, so the dep declaration now opts into BOTH explicitly. No call-site changes — the high-level `toml::from_str` / `toml::Table` surface is stable across the major. Cargo.lock drops `toml_datetime` 0.6 / `toml_edit` 0.22 / `winnow` 0.7 in favour of their 1.x successors. `Thresholds::parse` and `LayerRules::parse` consumers see no behavioural change. Closes F122.
- **SPA: cooperative widget boot — first paint no longer waits for all 14 widgets.** Pre-fix, the `WIDGETS.forEach` boot loop synchronously rendered every widget (ECharts mount + d3.pack layout + DOM injection — tens of ms per widget on big repos) before the browser could paint anything. First paint was bounded by the total cost of all 14 renders. Now: boot is an `async function bootWidgets()` IIFE that renders each widget, registers its rerenderer, and `await yieldToMain()` between widgets (not after the last — a trailing yield is a wasted task). The first widget in the registry (kpi-tiles) is cheap structural HTML, so by the time we yield after it the browser has already painted the page chrome + KPI cards; the heavier widgets fill in incrementally as the event loop yields. Uses the existing `yieldToMain()` `scheduler.yield()` → `MessageChannel.postMessage` → `Promise.resolve()` fallback ladder shipped for F134/F135. Smaller scope than the audit's "split JSON into per-widget `<script type=application/json>` blocks" — recon showed the JSON parse itself is fast even for big repos; the bottleneck was the synchronous render storm, not the parse. SPA integration + browser smoke tests green. Closes F97.
- **SPA: 2-col layout now kicks in at tablet portrait (≥ 768 px), not desktop (≥ 1280 px).** The dashboard grid container's responsive breakpoint moved from `xl:grid-cols-2` to `md:grid-cols-2`, so viewports between 768 px and 1279 px (tablet portrait / landscape) get a proper two-column layout instead of the uncompressed desktop view. Wide widgets keep `xl:col-span-2` so they only span both columns at desktop; at md/lg they sit in the normal 2-col grid taking one column each. Mobile (< 768 px) stays at single-column. The Tailwind v4 bundle was rebuilt (`tailwindcss -i tailwind-src/input.css -o tailwind.daisyui.min.css --minify`) so `md\:grid-cols-2` is generated alongside the existing `xl\:grid-cols-2` — the v4 `@source` scanner only emits classes it sees as literals in the template, so the rebuild was required after the markup change. SPA integration + browser smoke tests green. Closes F133.
- **SPA: chart palette externalised to CSS custom properties — light theme actually works now.** Four sites in `widgets.js` (coupling-sankey label color, hotspot-treemap leaf label, calendar-heatmap 5-band friction ramp, 15-color author palette used by the knowledge-map mode of the hotspot circle-pack) previously inlined hex literals tuned for dark mode. Light-theme users saw the `#1a4a2c` "low" band of the calendar heatmap disappear into the `#fafafa` card background, and the author palette colors washed out against white. New `--label-on-dark`, `--label-on-saturated`, `--heatmap-{1..5}`, `--chart-palette-{1..15}` tokens are defined in `:root` (dark theme — preserves the existing hex set) and overridden in `[data-theme="light"]` (heatmap "low" band re-tuned to a desaturated mint `#c8e6c9`; author palette re-tuned to deeper saturation so colors don't wash out on white). JS sites swapped to `token('--name')` reads — theme-aware, cache-invalidating via the existing `_tokenCache` + `invalidateTokenCache` pair. Three widget entries (`coupling-sankey` / `hotspot-treemap` / `calendar-heatmap`) upgraded from default `rerender` to `rerender: 'theme'` in the V4 WIDGETS registry so the token cache flushes on theme toggle (was already correct for the hotspot circle-pack). `grep '#[0-9a-fA-F]\{3,6\}'` in widgets.js now returns zero hits. Browser smoke test green. Closes F132.
- **SPA: widget boot is now a single registry-driven loop.** Pre-V4 the §3 Boot section had 14 widgets each declared as two duplicated lines — the `renderXxx(data.field || [])` call AND a `window._codeloreRerenderers.push(() => renderXxx(data.field || []))` line — inviting theme-rerender drift every time a new widget landed (and several Plan-era follow-ups DID drift in exactly that way). The boot section now ships a single `const WIDGETS = [{ name, render, rerender? }, ...]` table where each entry is `name` (id for logging), `render` (a `() => ...` thunk closing over `data`), and an optional `rerender` flag: `false` opts out of any theme rerender (KPI tiles, KI table, hotspot table — pure-DOM widgets that don't read CSS variables), `'theme'` registers via `registerThemeRerender` (token-cache flush before the redraw — used by the hotspot circle-pack for its friction heat ramp / health 3-band / top-quartile overlay), and the default path falls through to `_codeloreRerenderers.push`. A single `WIDGETS.forEach(w => { w.render(); /* dispatch on rerender */ })` loop replaces ~60 LOC of duplicated bootstrap. Adding a widget is now a one-line append. Browser smoke test + integration test green. Closes V4.
- **SPA: per-metric tooltips now surface effective thresholds, not parameter names.** `METRIC_DEFS` formula strings previously read e.g. `gated by min_shared_revs and Fisher exact p < fisher_significance` — the parameter NAMES, not the values actually in force on the run. New `SpaOptionsSnapshot { min_revs, min_shared_revs, min_coupling_pct, max_coupling_pct, max_changeset_size, fisher_significance }` field on `SpaDashboard`, populated from `Options::from_options` at dispatch, is serialised into the SPA's data payload as `data.options`. JS-side `interpolate(formula, opts)` substitutes `${key}` placeholders in METRIC_DEFS strings, so coupling_pairs and coupling_density formulas now read e.g. `min_shared_revs ≥ 5` / `Fisher exact p < 0.05` (or whatever this run's effective thresholds are). Unknown placeholders left as the literal `${key}` token so a stale METRIC_DEFS entry surfaces visibly during review rather than silently filling with `undefined`. `SpaOptionsSnapshot::default()` mirrors the code-maat parity baseline (kept aligned via the existing `default_options_match_code_maat_thresholds` test) so tests + step-summary using `..SpaDashboard::default()` stay green without per-site updates. Closes V5.
- **`output::json::write_json` is now `pub`; 27 trivial `write_*_json` shim functions retired.** The JSON emitter previously carried one wrapper function per analysis (`write_hotspots_json`, `write_code_health_json`, etc.) — each one a 5-line shim that called `write_json(rows, w)`. The generic `write_json<T: Serialize>` is now public and called directly from CLI dispatch via turbofish (`output::json::write_json::<HotspotRow>(&rows, &mut out)`), matching the existing `output::ndjson::write_ndjson` pattern. Two non-trivial emitters preserved: `write_revisions_json` (wraps the `(String, u32)` tuple into a named-field struct for JSON output) and `write_communities_json` (emits the wrapper struct carrying partition-level summary alongside per-file mapping). Net: -137 LOC across emitter + 33 call sites. Closes F146.
- **Schema v3 — `commits` table gains a `committer_date TIMESTAMP NOT NULL` column.** The pre-v3 schema persisted only the author date (in `commits.date`); the `lead-time` analysis emitted zero rows for every commit and silently warned. v3 persists both timestamps; the delta `(committer_date - date)` is the in-flight time that `lead-time` and the new `delivery-friction` analysis surface. `CURRENT_SCHEMA_VERSION` bumped `1 → 2` (provenance stamp), `types::SCHEMA_VERSION` bumped `2 → 3`, `cache::SCHEMA_VERSION` bumped `schema_v3 → schema_v4` (cache invalidates naturally on bump per the design). Both `GixRepo` and `GitCliRepo` populate the new field; the differential test `committer_date_matches_across_walkers` asserts byte-identical agreement across backends. `lead-time` SQL now computes `EXTRACT(EPOCH FROM committer_date) - EXTRACT(EPOCH FROM date)` and the `tracing::warn!` about always-zero output is gone.

### Accessibility / UI polish

- **SPA: tablists now carry `aria-selected` so screen readers announce the selected tab.** Seven tablists across the dashboard (hotspot color modes, trends top-N, module-chord depth, architecture-graph depth, multi-metric top-N, delivery-risk window, change-coupling depth — 30 buttons total) previously toggled only the visual `tab-active` CSS class. Without `aria-selected`, the WAI-ARIA Tabs pattern is incomplete: screen readers see each tab as equally focusable but none as "selected", so a user can't tell which view is currently rendered without inferring it from the chart underneath. JS-driven hotspot color-mode handler (`initHotspotColorToggles` in `widgets.js`) now sets `aria-selected` on every tab inside the toggle loop alongside the existing class toggles. Template ships with `aria-selected="true"` on the default-active hotspot tab and `aria-selected="false"` on the other six so the first paint is correct before the click handler ever fires. The six Alpine-driven tablists each gained `:aria-selected="$store.layout.<key> === <value> ? 'true' : 'false'"` directives next to the existing `:class` binding (applied via a one-shot Python regex pass — 26 of the 30 buttons in a single sweep). Closes F136.
- **SPA: knowledge-islands and hotspot table rows now keyboard-activable.** Both tables wired row click → file-detail drawer; without keyboard equivalent that drill-down was mouse-only — WCAG 2.1.1 Keyboard. New `wireRowKbActivation(rowEl)` helper in `widgets.js` sets `tabindex="0"` + `role="button"` on each row and forwards Enter / Space to the existing click handler (`preventDefault()` on Space so the page doesn't scroll). Applied to both `tr.ki-row` (`renderKnowledgeIslands`) and `tr.hotspot-row` (`renderNextPage`) — the audit only flagged KI but the hotspot table had the same gap; one helper, two call sites. A `tr.hotspot-row:focus-visible, tr.ki-row:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px }` rule paints a visible focus ring so keyboard users can see which row is about to be activated (WCAG 2.4.7). `:focus-visible` (not bare `:focus`) keeps mouse clicks from drawing the ring. Closes F137.
- **SPA: provenance tooltip triggers meet WCAG 2.5.5 Target Size (Minimum).** `.tooltip-trigger` bumped from `14×14 px` to `24×24 px` across every `?` affordance — the glyph stays visually moderate (`font-size: 12px`, `line-height: 22px`) so the trigger doesn't dominate dense table headers, but coarse pointers (touch, stylus, motor-impaired mouse use) now have a reachable target. The trigger sits inline against label text via `vertical-align: -7px` so adjacent text baselines stay aligned with no padding rewrites elsewhere. CSS anchor positioning and `:hover` / `:focus-visible` reveal paths are unchanged. Closes F131.
- **SPA: element-scoped View Transitions (Chrome 147+) for theme toggle, color-mode swap, and `Show all` expansion.** `startViewTransition(updateFn, scope)` now prefers `scope.startViewTransition(...)` when available so the transition runs only on the affected widget — the rest of the dashboard stays interactive during the crossfade. Document-scoped transitions remain the fallback on older browsers.
- **SPA: `view-transition-name: match-element` on `.widget` and `.hotspot-row`.** Hotspot table sort / filter / `Show all` expansion now animate row-by-row rather than as one full-table crossfade; theme + color-mode swaps animate per-widget rather than as a document-wide repaint. `contain: layout style` on `.hotspot-row` caps the per-row cost. Falls through harmlessly on Chrome < 147 (no-op).
- **SPA: CSS anchor positioning for the `?` tooltip popups (Baseline 2026 newly available — Chrome 125+, Firefox 147+, Safari 26).** Each `.tooltip-trigger` declares `anchor-name: --tooltip-trigger`; the popup uses `position-anchor` + `anchor(bottom) + 6px` to pin against it, with `@position-try-fallbacks --tooltip-above, --tooltip-right, --tooltip-left` so the browser auto-flips against viewport edges. The legacy `position: absolute` + `.tooltip-host { position: relative }` path remains as the fallback for browsers below the Baseline cut. Browser handles scroll- and resize-tracking natively, retiring JS reposition math for these triggers.

### Docs

- **MSRV (Minimum Supported Rust Version) Policy** section added to `docs/RELEASING.md`. Documents the deliberate "MSRV tracks toolchain channel" stance — appropriate for the pre-1.0 CLI-binary distribution model where end users install via prebuilt binaries / `cargo binstall` / container rather than building from source. Codifies the post-1.0 reconsideration trigger ("external Rust consumers depend on `codelore-lib`'s API"). Closes F124.

## [0.8.0] - 2026-06-18

### Added

- **F175 — Per-widget tunables on the SPA dashboard.** Module-coupling
  chord, architecture force-graph, and change-coupling sankey each
  gained a depth selector (Auto + 2-6 path segments) that re-paints
  the chart on tab-click. Trends, Multi-metric comparison, and
  Delivery-risk Kamei gained Top-N / window selectors (5/10/20/All,
  10/20/50/All, 10/30/60/All). Backend caps bumped to 50 trends
  paths and 100 Kamei commits so the wider options work without a
  re-analysis. All selections persisted via `Alpine.$persist`.
- **F176 — URL state persistence for shareable views.** Depth
  selectors, Top-N choices, and off-boarding scenario sync to the
  URL hash (`#departed=...&chordDepth=3&trendsTopN=20`). A pasted
  link reproduces the exact dashboard view its creator was looking
  at, overriding the local-storage cache. `history.replaceState`
  drives the writes so back/forward navigation walks through view
  changes without page reloads.
- **F177 — Cross-widget selection bridge.** New
  `Alpine.store('selection')` + listener registry. Clicking a file
  in any path-aware widget (Hotspot table, parallel-coords, KI row,
  keyboard list, treemap) lights the matching polyline on Trends
  and Multi-metric comparison. Drawer-close clears the selection.
- **F178 — Reactive off-boarding propagates through every panel.**
  Picking departed authors now flags affected rows in the Hotspot
  table (red accent + left-border), the keyboard-accessible file
  list (red badge + accent), the coupling-partners list inside the
  detail drawer, and the top-contributors list inside the drawer —
  same Alpine `$store.scenario.departed` signal threaded through
  every surface that shows file ownership.
- **F179 — Detail drawer enrichment.** Drawer now surfaces top
  contributors per file (top-5 authors by `+added/-deleted` LoC
  with knowledge-loss flags), function-level cognitive complexity
  inline from X-Ray (top-8 functions with line numbers), and
  clone-group membership. Coupling-partner list shows each partner's
  primary author and flags departed-author files. Zero-contribution
  authors (rename / revert artefacts) filtered from the contributor
  list.
- **F180 — Per-widget Learn-more disclosures.** Every panel now
  carries a collapsed `<details>` explaining what it measures, how
  to read it, what to watch for, recommended action, and academic
  citation. Hover the small `?` icons on color-mode tabs, depth
  selectors, and Top-N tabs for one-line popups using the existing
  `tooltip-host` convention.
- **F181 — Fullscreen toggle + pan/zoom per widget.** Every widget
  panel gains a fullscreen icon (top-right). Hotspots circle-pack
  and Architecture graph also get a reset-zoom button. Circle-pack
  uses CSS-transform wheel-zoom + drag-pan + double-click reset
  (ECharts `type: 'custom'` doesn't support native roam); arch
  graph uses ECharts' built-in `series.roam`.
- **F182 — Wide-screen layout.** Dashboard cap lifted from 1600 px
  to 2400 px, centred via `mx-auto`. `.widget` gets
  `position: relative` so tooltips appended to the outer section
  anchor against the panel.
- **F183 — Trends vertical-right legend with All / Swap selectors.**
  Horizontal-top legend collided with the y-axis name and
  overflowed at large Top-N; switched to vertical scrollable
  right-side legend with ECharts' built-in `selector` for one-click
  Select-all / invert-selection ("Swap"). Y-axis label rotated
  along the axis so it doesn't compete with the legend.

### Fixed

- **F184 — Hovered bar / line disappeared on Kamei + parallel-coords.**
  ECharts 6 regression on `emphasis: { focus: 'self' }` with
  per-data `itemStyle.color` repainted the hovered element with
  the chart background. Switched both series to
  `emphasis: { disabled: true }`; the tooltip carries the hover
  affordance.
- **F185 — Kamei tooltip occluded the bar it described.** ECharts
  `position: 'top'` falls back to cursor-at-bar placement on
  certain renderer paths (issue #15307). Tooltip is now appended
  to the outer `section.widget` via ECharts' `appendTo` and pinned
  to the panel's top-right corner via object-syntax `position`.
  `.widget { position: relative }` anchors it to the panel.
- **F186 — Knowledge-islands Path column was empty + click-to-drawer
  showed "No additional details".** The KI payload uses `entity`
  (not `path` like the other tables); the renderer + drawer lookup
  now read both. Knowledge-island files without hotspot metrics
  hide the radar instead of rendering an empty-state message —
  ownership / author / days-since-active surface in the Knowledge
  island section directly.
- **F187 — Drawer opened at top-left corner blocking other clicks.**
  Dropped DaisyUI's `.modal` class (its `inset: 0` over-constrained
  the right-side `.detail-drawer`). Switched from `showModal()` to
  non-modal `.show()` so the drawer floats without a backdrop —
  click another row anywhere on the page and the drawer content
  swaps in place. Removed the redundant bottom-left close button
  (was the modal-backdrop close); the × in the header is the sole
  close affordance.
- **F188 — Trends legend overlapped its own entries + y-axis name.**
  Horizontal-top legend with `type: 'scroll'` overflowed once paths
  got long; rotated to vertical right-side and rotated the y-axis
  name along the axis.
- **F189 — Hotspots circle-pack rendered left-aligned in wide
  containers.** d3.pack produces a `[side, side]` square layout;
  on a wider-than-tall canvas that square sat top-left. Centred via
  an `xOffset` / `yOffset` translation applied to every node before
  ECharts consumes the coords, so downstream arc anchors and
  position cache stay in sync.
- **F190 — Calendar heatmap colour legend overlapped month labels.**
  Horizontal-top visualMap with the calendar's month band collided
  visibly ("May 1.0-5.4 Jun 5.4-9.8"). Moved to vertical right-side
  visualMap with calendar `right: 130` to reserve space.

- **F165 — Detail-drawer modal showed as a permanent sidebar.** The
  `<dialog>` element had no `[hidden]` attribute, so its
  `.detail-drawer { position: fixed }` CSS rule painted the closed
  dialog as a visible right-side panel. `Alpine.store('detail').show()`
  now removes `[hidden]` before `showModal()`, `hide()` re-adds it
  after `close()`, and a `close` event listener mirrors that toggle
  for the form-method=dialog × button + Escape-key + backdrop close
  paths (which bypass Alpine).
- **F166 — Temporal Dead Zone on `_colorResolver`.** The `let
  _colorResolver` binding was declared below the boot block but
  reached from `heatRamp()` → `resolveCssColor()` during the
  synchronous Kamei-sparkline render — `let` bindings are not
  hoisted, so the call landed in the TDZ. Declaration moved above
  the §3 Boot section.
- **F167 — Hotspot circle-pack `renderItem` returned NaN under
  ECharts 6.** `api.value('_raw')` / `api.value(2)` regressed in
  ECharts 6 — non-numeric keys produce NaN and object values
  coerce to NaN. Switched to the documented closure-from-data-array
  pattern: pre-build `circlePackData = [...]` at module scope; both
  `series.data` and `renderItem(params)` reference the same array
  via `circlePackData[params.dataIndex]._raw`. Same fix applied to
  the coupling-arc series + `updateCouplingArcs` (in-place mutation
  to preserve closure).
- **F168 — Hotspot circle-pack tooltip always showed "root".** The
  root node's full-canvas hit box intercepted every pointer event
  before it could reach leaf circles. Added `silent: isDirectory`
  to all directory shapes so hits pass through.
- **F169 — PWA manifest data URL truncated at the first `#`.** The
  inline `data:application/json,...` URL embedded
  `theme_color":"#1a1a1a"` — the browser parsed `#1a1a1a"...` as
  the URL fragment, leaving the JSON parser with a truncated
  string that errored at column 150. Percent-encoded `#` to `%23`.
- **F170 — Cognitive-distribution boxplot collapsed to a thin
  band.** Outlier scatter points (cognitive up to 209) shared the
  y-axis with the box (whisker-clamped to ~30); ECharts auto-fit
  stretched the axis to the outliers and the IQR collapsed to ~5%
  of the chart height. Removed the scatter series, clipped
  `yAxis.max` to `upperFence * 1.15`, set `boxWidth: [60, 140]`,
  and added a `graphic` annotation `+N outliers · max V` in the
  top-right.
- **F171 — Hovered line/bar appeared to disappear on the parallel-
  coords and Kamei-sparkline widgets.** ECharts 6's default
  emphasis behaviour leaves the hovered element visually identical
  to its neighbours (or worse, the hovered series re-paints
  underneath the dim batch). Added explicit `emphasis: { focus:
  'self', ...lineStyle/itemStyle: { opacity: 1 } }` + `blur:
  { ...style: { opacity: 0.15-0.25 } }` to both series so hover
  vivid-ifies the target and fades the rest.
- **F172 — Architecture force-graph empty for repos that nest
  everything under a single root.** `topDir(p)` returned the first
  path segment (`app/`), folding every edge into `app→app` and
  dropping it as intra-module — the user got the misleading "stays
  intra-module" message even with thousands of resolved imports.
  Replaced with adaptive `modulePath(p, depth)` that retries
  depth 2→6 until at least one inter-module edge survives. Same
  fix applied to module-chord rollup.
- **F173 — Trends legend overflowed with long file paths.** Top-N
  hotspot paths like `app/services/clients/application/service.py`
  consumed an entire scroll-pager page each. Added abbreviated
  display (`app/…/application/service.py`) + tooltip formatter
  that restores the full path for hover.
- **F174 — Alpine bindings silently inert across the page.** Only
  one `x-data` directive existed in the template (on the theme
  checkbox), so the keyboard-accessible file list, off-boarding
  dropdown, and theme indicators outside that scope were ignored
  by Alpine's directive walk — `$store.dashboard.hotspots.length`
  rendered as `()`, `<template x-for>` left raw template nodes in
  place, `x-show` never hid the "Clear scenario" item. Added empty
  `x-data` to `<body>` so the entire page is a single component
  scope.

## [0.7.0] - 2026-06-16

### Added

- **F143 — Headless-browser smoke test for the SPA dashboard.** New
  `crates/codelore-lib/tests/spa_browser_test.rs` integration test
  (gated on a new `browser-tests` Cargo feature) renders the SPA
  via the differential fixture, opens it in headless Chrome via the
  `headless_chrome` crate, and asserts (a) no `RuntimeExceptionThrown`
  events fire during render, (b) the KPI tiles container has rendered
  content. Companion `spa-browser` job added to `.github/workflows/ci.yml`
  on `ubuntu-latest` only (Chrome dependency). Catches the runtime
  init-order class of defect that v0.5.1's F107/F108 hotfix had to
  fix in production. Caught a real TDZ regression at PR time (the
  `_tokenCache` helpers in `widgets.js`); fix shipped in the same PR.
- **F110 — Differential-test coverage extended to the previously
  un-cross-checked `Repo` trait methods.** Four new tests in
  `crates/codelore-lib/tests/differential_repo_test.rs` —
  `head_sha_matches`, `is_worktree_dirty_matches_on_fresh_clone`,
  `read_blob_at_head_matches_on_tracked_and_untracked_paths`,
  `diff_hunks_gix_is_empty_stub_cli_returns_real_hunks`. Writing
  the third test surfaced a real bug: `GitCliRepo::read_blob_at_head`
  was missing (defaulted to `Ok(None)`) — production code falling
  back to the CLI backend would have silently read empty blobs.
  Backend implementation added via `git show HEAD:<path>`.
- **F112 — Provenance manifest carries reproducibility-critical
  fields.** Five new fields in the `.provenance.json` sidecar:
  `head_sha`, `cache_key_hash`, `rust_version`, `target_triple`,
  `grammars` (BTreeMap of tree-sitter crate name → exact version
  pin). The grammar pins make complexity / clones outputs
  trivially auditable for ABI compatibility.
- **F140 + F141 — Integration-test coverage for the v0.6.0 analyses
  + multi-language import resolver.** Five new `tests/*_test.rs`
  files (`bus_factor_test`, `lead_time_test`, `stale_code_test`,
  `pair_programming_test`, `god_classes_test`, `arch_violations_test`)
  plus a new `ingest_resolves_imports_to_target_paths` assertion in
  `imports_factsdb_test` that exercises the JS/Python/Rust per-language
  resolvers end-to-end.

### Changed

- **Provenance sidecar schema bumped: `MANIFEST_SCHEMA_VERSION = 1`
  → `2`.** Downstream consumers reading `.provenance.json` directly
  see five new top-level fields (`head_sha`, `cache_key_hash`,
  `rust_version`, `target_triple`, `grammars`). Existing fields are
  unchanged. Consumers with `#[serde(deny_unknown_fields)]` opt-in
  need to bump their schema; everyone else passes through. See
  **Upgrade notes** below.
- **F155 — `DiffOutput.{base,head}_median_code_health` JSON shape
  changed.** Both fields are now `Option<f64>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`, instead of
  bare `f64` defaulted to `0.0`. When `--thresholds-file` is absent
  or `[diff]` gates are not configured, the fields are now **omitted
  entirely** from the diff JSON (previously emitted as `0.0`, which
  read as catastrophic on the 0-100 scale and tripped downstream
  dashboard rules). See **Upgrade notes**.
- **F156 — `.codelore-thresholds.toml` now rejects unknown keys.**
  `Thresholds` / `Gates` / `DiffGates` carry `#[serde(deny_unknown_fields)]`,
  so a typo like `cognative_max = 30` (transposed) or
  `disallow_clone_type1 = true` (missing underscore) — previously
  silently parsed as the default — now fails the parse with the
  standard serde "unknown field" error. The gate's value proposition
  is that the repo carries the gate; silent misconfiguration was
  the worst failure mode. See **Upgrade notes**.
- **F157 — `AnalysisName::all()` registry single-source-of-truth.**
  F147's exhaustiveness guard forced new variants to be added to a
  match but did not force them into the actual `&[Self::X, ...]`
  array `all()` returns — meaning a new variant could be added to
  the match (forced) and forgotten from the array (silently). A new
  `registry!` macro now expands ONCE into both the array and the
  guard match from a single token list; the two surfaces cannot
  drift by construction.
- **F158 — SARIF `tool.driver.informationUri` now points at the
  canonical codelore repo.** Five sites previously hardcoded
  `https://github.com/emre/codescene` (wrong project name, wrong
  org). Three `informationUri` constants + two rule `helpUri`
  strings updated. Every SARIF report shipped to GitHub Code
  Scanning previously linked the tool-details panel to a 404 / the
  wrong repo. See **Upgrade notes**.
- **F160 — Kamei strict-prior peer semantics unified.** EXP / REXP
  previously used inclusive `prev.date <= c.date` (same-second peers
  counted as priors); NDEV / NUC / AGE / SEXP used strict `<`. Three
  different "prior commit" definitions inside one canonical 14-feature
  vector made the output paper-unfaithful. EXP / REXP now use strict
  `<` like the rest; the tiny_repo test fixture was updated to span
  explicit per-commit timestamps via a new `run_git_at(path, iso_date,
  args)` helper. See **Upgrade notes**.
- **F154 — `codelore diff <base>..<head>` rejects `base == head`.**
  Previously ran two identical analyses, computed a zero-everywhere
  delta, and emitted an empty SARIF / JSON / markdown diff with no
  signal that the input was vacuous. Now bails at the entry point
  with an actionable error naming the SHA and the range. Hot failure
  mode after a `gh pr checkout` refresh that leaves the local branch
  at the base SHA.
- **F94 / F97 / Branch-merge gate** — these audit findings remain
  Active (file:line + suggested fix) in `docs/reports/deep_analysis_report.md`
  for follow-up. The validation pass in §4½ of that report carries
  source-grepped evidence for every Active finding as of this release.

### Performance

- **F125 — Redundant HEAD-time SQL queries hoisted to compute-once
  per ingest.** `query_live_paths` (a recursive CTE + `arg_max` over
  `commits ⋈ changes`) and `current_head_rev` previously ran four
  times per ingest — once each from the complexity / clones / imports
  / resolver passes. Both now compute once at the top of `ingest()`
  and thread down via `&[String]` + `&str`. Saves 30-300 ms per
  ingest on real-world repos.
- **F126 — Resolver UPDATE rewritten from N round-trips to one
  hash-joined `UPDATE … FROM`.** `resolve_imports_at_head` previously
  issued one prepared `UPDATE imports SET …` per resolved hit —
  O(N × |imports|) because DuckDB has no clustered index on the
  predicate columns. Now bulk-inserts hits into a TEMP TABLE and
  applies a single hash-joined UPDATE. ~100× speed-up on
  import-heavy monorepos with thousands of resolved edges.
- **F127 + F128 — Kamei `enrich_diffusion` + `enrich_size`
  rewritten from correlated subqueries to grouped `UPDATE … FROM`.**
  Three (diffusion: NF/NS/ND) + two (size: LA/LD) correlated subqueries
  per commit re-scanned `changes` for each — O(N × |changes|). Both
  now run a single `GROUP BY rev` and hash-join the aggregates back.
  Same shape the `enrich_history` and `enrich_experience` passes
  already use.

### Fixed

- **F89 → F109 closures from prior batches**, recorded as
  `Fixed-on-branch` in `deep_analysis_report.md` §3 closure log;
  this release marks their landing on main.
- **F118 — `GixRepo::walk_commits` walker thread panic no longer
  silently swallowed.** New `WalkerStream` wrapper owns the
  `JoinHandle` alongside the receiver; on end-of-stream it joins
  the handle and surfaces any panic as a final
  `Err(CodeLoreError::Repo(...))`, mapped to exit code 3 per
  spec §6.6. Two unit tests cover the panic-surfaces and clean-exit
  cases.
- **F127 / F128 / F143 / F150 / F151 / F152 / F154 / F156 / F157 /
  F158 / F159 / F160 / F163** — see above sections.
- **F138 — `startViewTransition` honors `prefers-reduced-motion`.**
  The SPA's wrapper previously called `document.startViewTransition`
  unconditionally; users with the OS-level reduced-motion preference
  still got the crossfade. Now queries `window.matchMedia` and
  applies the update synchronously when reduced motion is preferred.
- **F139 — `[diff]` gates evaluated and wired into `codelore diff`.**
  `Thresholds.diff` was parsed but never evaluated. New
  `evaluate_diff_gate` in `quality_gates` plus CLI wiring in
  `codelore diff` make the `[diff]` section enforce
  `new_hotspot_max` and `delta_code_health_min`. Returns non-zero
  exit on violation.
- **F147 — `AnalysisName::all()` exhaustiveness guard added** (now
  superseded by the F157 macro fix above, but the underlying gap
  is closed end-to-end).
- **F149 / F150 — Schema version validation on `FactsDb::open_read_only`.**
  Operator who hands a stale `.duckdb` to `--cache-dir` directly
  now gets a typed parse-time error instead of cryptic SQL failures
  at analysis time. Literal `"1"` promoted to a `CURRENT_SCHEMA_VERSION`
  constant in `facts/schema.rs` so producer and validator share
  one source of truth.
- **F151 — Leiden community detection seeded deterministically.**
  `LeidenConfig::default()` left `seed = None`; `leiden-rs` fell
  back to wall-clock entropy. Module docstring promised "deterministic
  across runs"; that promise was broken on every cache miss. Fixed
  via `LEIDEN_SEED = 0xC0DE_10E5_AED1_DEED`.
- **F152 — `clone_group_id` deterministic across runs.** The clone
  extractor's `HashMap<[u8;32], _>` bucket was iterated in
  `RandomState` order, swapping group ID assignments across process
  restarts. Switched to `BTreeMap` so iteration is digest-sorted.
- **F159 — SARIF `artifactLocation.uri` percent-encoded per
  RFC 3986.** Paths with spaces, `#`, or non-ASCII characters
  previously shipped as raw bytes; GitHub Code Scanning rejected
  the SARIF upload or silently truncated at `#` so inline annotations
  landed on the wrong file. `percent-encoding` crate added as a
  direct dependency.
- **F163 — SARIF `automationDetails.id` is per-run unique.** SARIF
  2.1.0 §3.17.3 wants `<runGroupName>/<runName>/<correlationGuid>`
  per-run; the three constants in the codebase were static strings,
  so GitHub Code Scanning collapsed runs into a single timeline,
  defeating the partialFingerprints work. New `automation_id_for`
  appends a 16-hex correlation suffix from `SystemTime` nanos +
  `process::id()`.

### Repository hygiene

- **Audit log cleaned up.** `docs/reports/deep_analysis_report.md`
  now carries a §3 closure log (every shipped F-finding with its
  closing commit), a §4 active-findings list (with file:line +
  severity + suggested-fix shape for every future task candidate),
  and a §4½ validation pass with source-grepped evidence per Active
  entry. 20 findings shipped this release; ~30 remain as documented
  future tasks.
- **Local + GitHub state cleaned.** 4 redundant local branches
  deleted, 3 superseded stashes dropped, the v0.5.x UI redesign
  branch stack consolidated to its canonical superset
  (`feat/v0.5x-ui-redesign-pr8-table-controls-a11y`) pushed to
  origin for safekeeping, the 3 intermediate checkpoints preserved
  as `backup/ui-redesign/*` tags on origin.

### Upgrade notes — breaking changes for consumers

This release contains four schema / output-format changes that may
affect downstream consumers. None require code changes if you
consume the documented surface area, but each is called out so
strict-schema parsers can opt in deliberately.

1. **`.provenance.json` schema_version 1 → 2** (F112). Five new
   top-level fields. Backwards-compatible for permissive parsers;
   add the five fields if you use `deny_unknown_fields`.
2. **`codelore diff` JSON shape** (F155). `base_median_code_health`
   and `head_median_code_health` are now omitted when no `[diff]`
   gate is configured; previously emitted as `0.0`. Consumers that
   read these fields without a presence check now see "key absent"
   instead of "0.0" in the no-gate case — semantically more correct,
   but a JSON-shape change.
3. **`.codelore-thresholds.toml` strictness** (F156). Unknown keys
   now fail the parse instead of silently defaulting. If your
   thresholds file has a typo today, the v0.7.0 binary will refuse
   it; previously the typo'd gate was effectively disabled.
4. **SARIF `tool.driver.informationUri`** (F158). Changed from the
   incorrect `https://github.com/emre/codescene` to
   `https://github.com/emrecdr/codelore`. Tooling that hardcodes
   the old URL must update; tooling that follows the URL it sees
   in the report needs no change.
5. **Kamei EXP / REXP same-second peer semantics** (F160). Now
   uses strict `prev.date < c.date` like the rest of the 14-feature
   vector (was inclusive `<=`). Same-author same-second commits no
   longer count as priors for each other. Production repos with
   distinct-second commits see no change; bulk-import / amend-heavy
   histories may see different EXP / REXP values.

## [0.6.0] - 2026-06-15

### Added — v0.6.x maximum-aligned feature sprint (full implementation)

The 27-feature plan in `docs/maximum-feature-plan.md` shipped end-to-end across an 18-day implementation cycle (plus a completion sprint for the deferred quality-gates and multi-language resolver pack). The cycle landed Tier 1 (CodeScene parity + architecture foundation), Tier 2 (analytical surface + modern platform), and Tier 3 (brand-extending) with a single CI-clean clippy gate at every checkpoint.

**Net deltas vs v0.5.1**:
- **+6 new behavioural analyses** — `god-classes` (Brown et al. 1998 AntiPatterns), `architecture-violations` (layered-rule validation via `.codelore-arch-rules.toml`), `stale-code` (untouched + low-cognitive intersection), `pair-programming` (`Co-Authored-By:` trailer aggregation), `lead-time` (DORA in-flight time), `bus-factor` (per-module Filatov 2010)
- **+6 new CLI subcommands** — `codelore explain` (formula + citation for 15 metrics), `codelore check` (quality-gate validation against `.codelore-thresholds.toml`), `codelore profile` (operational telemetry), `codelore docs` (markdown analysis catalogue), `codelore completions <shell>` (bash | zsh | fish | powershell | elvish), `codelore schema <row-type>` (JSON Schema 2020-12)
- **+6 new SPA widgets** — Kamei Delivery-Risk Sparkline (beyond-CodeScene differentiator) · per-file radar in drawer · hotspot treemap · multi-metric parallel coordinates · cognitive boxplot · module chord · architecture force-graph
- **+3 new SPA color modes** on the hotspot circle-pack — Code health (DaisyUI 3-band) · Tech-debt friction (OKLCH continuous heat ramp) · Knowledge loss (offboarding scenario driven)
- **+1 new SPA overlay** — coupling arcs with Fisher p-value-encoded opacity + degree-encoded width (CodeScene-exceeding)
- **+1 new SPA interaction** — Off-boarding scenario picker (DaisyUI multi-select dropdown + `$persist`, runs entirely client-side)
- **5 modern web platform primitives** brought in — View Transitions API · native `<dialog>` · PWA manifest · OKLCH `color-mix()` · WCAG-conformant parallel DOM tree
- **+1 schema migration** — `schema_v3` adds the `imports` table for the architecture import-graph (F-A1), populated via tree-sitter walks across the 6 Tier-1 languages, with per-language path resolver (Rust `crate::`, Python `.`, JS/TS `./`)
- **+2 quality-gates files** — `.codelore-arch-rules.toml` (layered-architecture) + `.codelore-thresholds.toml` (gate thresholds with `$GITHUB_OUTPUT` integration)
- **+602 tests** total (+280 net since v0.5.1), all green, clippy `-D warnings` clean

**Closed F-findings as side effects of feature work**: F71 · F90 · F92 · F97 · F98 (see `docs/reports/deep_analysis_report.md` for the audit trail).

### Fixed — multi-angle validation pass

A high-effort multi-angle code review of the feature cycle surfaced 15 correctness issues; each was confirmed against the running binary on the codelore repo before fixing.

- **stale-code**: replaced `DATE_DIFF('month', TIMESTAMP, DATE)` (which DuckDB 1.10.5 cannot bind) with EXTRACT-based interval-month arithmetic mirroring the existing `code_age.rs` pattern; negative-month rows (future-dated commits) now filtered at SQL.
- **bus-factor**: switched `COUNT(*) FROM changes` to `COUNT(DISTINCT rev)` so multi-file commits aren't multiplied; filter top-level files (`README.md`, `Cargo.toml`) out of the module set; `Option<f64>::unwrap_or` instead of swallowing NULL ratios to a false 0%.
- **arch_rules**: TOML deserialisation now uses an order-preserving `toml::Table` walk; `classify()` honours its documented first-match-by-declaration-order contract deterministically. Test asserts exact result, not `matches!(api|app)`.
- **pair-programming**: primary identity canonicalised to lowercased email (matches lowercased trailer emails); `BotPatterns::from_repo` filter applied to both primary and co-authors so the documented bot-filter promise actually fires.
- **Rust import resolver**: `crate::` now resolves to the importer's containing crate `src/` boundary (workspace-aware), not literal top-level `src/`. Verified `god-classes fan_in` now > 0 on the codelore workspace itself. Bare module paths (`foo::bar`) skipped since Rust 2018+ treats those as extern crate references.
- **Python import extractor**: `from X import Y` stores `X` (was: `X import Y`); `import x.y, z` keeps the first module; `as` aliases stripped on both branches.
- **JS import extractor**: side-effect / dynamic imports (no ` from ` literal) return None so the walker skips them, instead of storing the raw statement as a phantom target that polluted `god-classes fan_out` counts.
- **All path-joining resolvers**: `to_posix()` helper normalises `PathBuf` output to forward-slash so Windows hosts match the POSIX paths gix emits.
- **lead-time**: `tracing::warn!` at run time explaining the schema gap so users see why every row reports 0 seconds.
- **disallow_clone_type_1** gate: was parsed but never enforced; new `evaluate_clone_gate` counts `similarity=1.0` clone groups and surfaces one violation per repo (verified: detects 118 Type-1 groups on the codelore repo).

### Removed

- `codelore notes` subcommand — was a stub that printed markdown explaining itself as a stub then exited 0. Misleading for CI release-notes workflows; defer to a future iteration when the engine is actually wired.
- `--diff <base..head>` flag on `codelore check` — was declared but never read by `run_check_cmd`.

### Docs

- README, CLAUDE.md, advanced-usage.md, codebase_analysis.md, ui-roadmap.md, roadmap-v1.x-and-beyond.md, deep_analysis_report.md refreshed to reflect the 31-analysis surface and current CLI shape.
- Codebase + non-changelog docs scrubbed of every F-ID, Tier/Day marker, sprint label, version anchor (`v0.4.x`, `v0.5.x`, `v0.6.x`), PR-N reference, and "shipped in vX.Y" annotation per the project rule that comments describe current contract; CHANGELOG is the only history surface.
- `docs/roadmap-v1.x-and-beyond.md` + `docs/ui-roadmap.md` restructured from sprint-sequenced release ladders into "Shipped (current state)" + "Planned (next direction)" sections.

## [0.5.1] - 2026-06-14

### Fixed — v0.5.0 SPA runtime errors (F107 + F108 hotfix)

Two production browser-console errors on the v0.5.0 SPA made the dashboard unusable on first paint. Both bugs predated v0.5.0 — they shipped through SPA-touching PRs going back to v0.4.x — and only surfaced when the JS executed in a real browser. The existing `spa_integration_test` greps the rendered HTML for string presence but never runs the JS, so neither bug ever tripped CI. PR #37 ships the fix; PR #38 records the post-mortem in `docs/reports/deep_analysis_report.md` as F107 / F108 plus a methodology note for the audit cycles' shared blind spot.

- **F107** — `METRIC_DEFS` Temporal Dead Zone in `widgets.js` IIFE. `renderKpiTiles(data)` was called at the top of the IIFE and reached `METRIC_DEFS` via `buildTooltipHtml`, but `const METRIC_DEFS = {...}` was declared further down — TDZ tripped on every render with `Uncaught ReferenceError: Cannot access 'METRIC_DEFS' before initialization`. Fix: hoist the `const RESEARCH_FOUNDATIONS_URL` + `const METRIC_DEFS` block to before the `renderXxx(data)` call block.
- **F108** — Alpine inline-script order caused `$store.*` undefined at first paint. Alpine's `cdn.min.js` auto-starts (`Alpine.start()`) immediately when `document.readyState !== 'loading'`, synchronously dispatching `alpine:init` before walking the DOM. The store-init inline `<script>` was placed AFTER `{{ALPINE_JS}}` — by the time our `addEventListener('alpine:init', ...)` ran, the event had already fired and our callback never executed. Every `x-show="$store.theme.isDark"` / `x-show="$store.detail.open"` evaluation then hit `undefined`. Fix: reorder the three inline scripts in `template.html` to `persist plugin → store-init listener → Alpine core`. Persist plugin's `cdn.min.js` registers via `document.addEventListener("alpine:init", () => Alpine.plugin(d))` — safe to load before Alpine core; DOM listeners fire in registration order, so persist's runs before ours; both run before Alpine walks the DOM.

The fix preserves the `Alpine.$persist`-backed store wiring (detail / filter / theme), the prefers-color-scheme first-paint guard, and the v0.4.x → v0.5.0 localStorage migration logic untouched. Template comment block above the scripts now documents the ordering invariant explicitly.

### Docs — F89–F108 audit cycles + cycle-methodology limitation captured

Two read-only audit cycles ran post-v0.5.0 against `main`:

- **F89–F98** (`docs/reports/deep_analysis_report.md` §3) — 5 parallel sub-agents over ingest/threading, SQL analyses, SPA frontend, Rust deps & idioms, CLI & output emitters. 10 Active findings + 3 Improvement opportunities (V4–V6) + 6 Refuted with source-quote evidence. Shipped in PR #36.
- **F99–F106** (§4 second pass) — 3 parallel sub-agents over CI/CD + release pipeline, identity layer + diff PR-mode + provenance manifest, analytical-formula correctness. 8 Active findings + 7 Refuted + 5 already-captured-in-§3 (dropped to avoid double-counting). Shipped in PR #38.

PR #38 also captures the cycle methodology limitation: both passes used static-grep + read-only inspection; neither surfaces *runtime* defects like F107/F108. Open structural follow-up: headless-browser smoke test (chromedp / playwright via cargo) for the SPA emitter.

## [0.5.0] - 2026-06-14

### Added — v0.5.x SPA UI redesign (Tailwind v4 + DaisyUI 5 + Alpine.js)

The interactive dashboard (`--format spa`) moves off the hand-rolled v0.4.x CSS onto a real design-system stack: **Tailwind v4** for utility-first layout, **DaisyUI 5** for themed components, **Alpine.js 3.15** for HTML-attribute reactivity. All three SHA-pinned at build time via `build.rs`; bundle stays self-contained (~1.5 MB rendered SPA, no CDN at runtime).

- Compiled Tailwind v4 + DaisyUI 5 CSS bundle (~78 KB minified) inlined into the SPA at build time.
- Alpine.js core + persist plugin SHA-pinned; localStorage-backed cross-widget filter state (`$store.filter.text`) and detail drawer state (`$store.detail.open`).
- Every widget section converted to DaisyUI primitives (`card bg-base-200 shadow-lg`, `stat / stat-value / stat-desc`, `table table-zebra`, `navbar`, `footer footer-center`, `badge badge-success/warning/error`).
- Detail drawer migrated to Alpine `x-show="$store.detail.open"` with `x-transition.opacity` fade + `@keydown.escape.window` close.
- Hotspot table filter input upgraded to `input input-bordered input-sm` + explicit `aria-label="Filter hotspots by path"` for screen-reader access.

### Added — V2 DaisyUI theme-controller migration (closes F79)

- DaisyUI `themes: light --default, dark --prefersdark` plugin config makes first-paint follow OS `prefers-color-scheme` purely via CSS — no JS frame for the wrong theme to appear in.
- Custom `<button id="theme-toggle">` replaced with DaisyUI `<label class="swap swap-rotate">` + sun/moon SVG pair + `class="theme-controller" value="dark"` checkbox (CSS-only theme swap, defense-in-depth if Alpine fails).
- New `Alpine.store('theme', { isDark: $persist(initialDark).as('codelore_theme_is_dark') })` + `Alpine.effect` bridge reactively mirrors the boolean to `<html data-theme>` AND fires every ECharts re-renderer in `_codeloreRerenderers`.
- Anti-flash-of-wrong-theme inline script in `<head>` reads persisted preference and sets `data-theme` synchronously before first paint. Includes legacy-key migration from old `codelore-theme` string key.
- `widgets.js::initThemeToggle()` (~30 lines) deleted — work distributed across pre-paint script, DaisyUI `:has()` selector, and the Alpine effect.

### Added — F83 clone-detection overlay on hotspot circle-pack

New "Clones" colour-mode toggle on the hotspot circle-pack surfaces structural-duplication signal directly on the file layout users already know — no new widget, no flat table. Files in ≥ 1 clone family render heatmap colour scaled to per-repo max clone-group count; files outside any family render neutral grey.

- New `SpaDashboard.clones` field (`Vec<CloneSummary { path, groups }>`) plumbed through `build_spa_dashboard`.
- New `output::spa::run_clone_summary` SQL helper queries the existing `clones` table with `COUNT(DISTINCT clone_group_id) GROUP BY path`.

### Fixed — F77 clone discovery on bare repositories

`populate_clones_at_head` discovery phase switched from `WalkDir::new(&opts.repo_path)` to `query_live_paths(self)?`. Bare repositories (no working tree checkout) previously returned zero clone candidates because WalkDir found only `.git/` metadata; the fix routes discovery through the gix ODB the same way `ingest_complexity_at_head` already does. New `ingest_populates_clones_on_bare_repository` regression test in `tests/clones_factsdb_test.rs`.

### Fixed — F82 SQLite emitter dumps `clones` table

`output/sqlite.rs::write_full_fact_store_sqlite` was dumping 7 of the 8 base tables in `schema_v1.sql` — silently omitting `clones`. `--format sqlite` exports now include clone-detection data. New table-list regression in `output_sqlite_test.rs` fails if any future schema table is added without updating the emitter.

### Fixed — F86 TSX files parsed with TSX grammar (not plain TypeScript)

`clones/language.rs` gained a `Tsx` variant routed through `tree_sitter_typescript::LANGUAGE_TSX`. Pre-fix, `.tsx` files were parsed with `LANGUAGE_TYPESCRIPT` which errors on JSX tags — every real-world TSX component produced an ERROR node and clone fingerprinting silently bailed. New inline regression test parses `<div>{n}</div>` against the new grammar.

### Fixed — F87 `.jsx` files participate in clone detection

`clones/language.rs::from_path` now maps `"jsx"` to the JavaScript variant (matching what `complexity/language.rs` already did). Pre-fix, JSX files were silently skipped in the clones pass while still being analysed for complexity — an inconsistency in language coverage between two analyses on the same file set.

### Performance — F78 drop redundant `source.to_vec()` in `compute_for_file`

Signature changed from `source: &[u8]` to `source: Vec<u8>`; the rayon caller already owns a fresh `Vec` from `Repo::read_blob_at_head`, so the move is free. Drops one full-source clone per Tier-1 file at HEAD ingest — meaningful on large source trees during HEAD-time complexity scan.

### Performance — F85 `apply_grouping` hunks cleanup uses `NOT EXISTS`

The `--group-file` post-ingest cleanup pass rewrote `DELETE FROM hunks WHERE (rev, path) NOT IN (SELECT …)` to a correlated `DELETE FROM hunks h WHERE NOT EXISTS (… AND c.rev = h.rev AND c.path = h.path)`. DuckDB reliably picks a hash anti-join on the `NOT EXISTS` form, avoiding the per-row subquery scan some planner paths produce for composite-key `NOT IN`. NULL-semantics concern is moot here (both projected columns are `NOT NULL` per schema); swap is purely about plan shape.

### UI — F80 responsive multi-column widget grid on wide screens

Main widget grid was hardcoded single-column regardless of viewport — on 1440p+ displays every widget stretched to full width and stacked vertically, wasting horizontal real estate. `<main>` now carries `max-w-[1600px] mx-auto grid grid-cols-1 xl:grid-cols-2 gap-7 p-7`; six visualization-dense widgets get `xl:col-span-2` so they keep full-width treatment; KPI tiles and knowledge-islands pair on row 1 at xl. Inline `main { ... }` rule deleted — layout primitives now on the markup.

### UI — F81 X-Ray sunburst encodes cognitive complexity as colour

Sunburst leaves were uniformly green (depth-shaded only); a 1-cognitive function and a 100-cognitive function in the same module rendered as the same shade. Per-leaf `itemStyle.color` now drives off `cognitive / maxCognitive`, reusing the existing `heatmapColor(ratio)` helper from the hotspot circle-pack — one visual vocabulary across the dashboard for cognitive complexity.

### Docs — F77–F88 audit pass validation + closeout

`docs/reports/deep_analysis_report.md` validation pass (PR #20) source-verified each finding against `main` HEAD before any fix landed: 3 confirmed-real-already-shipped (F82/F86/F87), 2 refuted with source-quote evidence (F84/F88), 7 confirmed-Active. All 7 Active shipped in this release. Report now collapses to a single "Audit-Pass Closeout" section reflecting the all-resolved state.

## [0.4.6] - 2026-06-13

### Fixed — Windows build unblocked (MSVC 19.40 / duckdb-rs#786)

GitHub Actions Windows runners rolled out MSVC 19.40 (toolchain
14.51) around 2026-06-12, which removed the deprecated
`stdext::checked_array_iterator`. Bundled DuckDB compiles transitively
include the fmt header that consumes it, so every `cargo build` on
`x86_64-pc-windows-msvc` fails inside `libduckdb-sys`. The
[v0.4.5 release](https://github.com/emrecdr/codelore/releases/tag/v0.4.5)
shipped without the Windows artifact for this reason.

Upstream `duckdb/duckdb-rs#786` carries the canonical patch but has
not landed in a published crate. As an interim, CodeLore vendors
`libduckdb-sys` at the released `v1.10503.1` rev plus the patch via
a `[patch.crates-io]` entry. The vendored source is regenerated
deterministically by `scripts/vendor-duckdb-rs.sh` at CI time (and
on demand locally) so source control stays slim. When the upstream
crate ships the fix in a released version, the patch block, script,
and `vendor/` ignore entry will be removed in one commit.

User impact: pre-built Windows binaries are restored. No behaviour
change vs `v0.4.5` on other platforms.

### Improved — SQL hotspot/SoC/churn perf + lockstep complexity joins

A small SQL-shaped perf batch that closes four `deep_analysis_report`
findings without changing any output semantics. Byte-identical baseline
validation across the cache-fixture suite confirms no result drift.

- **F72 / F73 — lockstep `rev` equality on `complexity_metrics × entities`
  joins.** The previous `JOIN entities e ON e.path = cm.path AND
  e.name = cm.name` matched a complexity row to the LATEST entity for
  that (path, name) regardless of when the metric was sampled, so
  long-lived files whose entity tables drifted across many revs paid
  for a redundant cross-product. Adding `AND e.rev_last_seen = cm.rev`
  pins each metric to its sampled rev and cuts the cartesian explosion.
  Applied to both `hotspots.rs::file_mi` and `output/spa.rs::run_xray`.
- **F74 — secondary index on `changes(rename_from)`** materialises the
  lookup the lineage CTE replays at every aggregation. Pure schema-time
  change; the materialised index pays for itself within the first
  rename-aware analysis call.
- **F75 — `soc.rs` filtered-changes CTE.** Mirrors the F67 pattern from
  `change_coupling.rs`: pre-filter `changes` against the pair set before
  the self-implicit double scan, eliminating the
  `Σ(blob_changes_per_file)²` blow-up on monorepos.
- **F76 — pre-aggregate per `rev` to eliminate `COUNT(DISTINCT)` in
  churn.** DuckDB's `COUNT(DISTINCT)` builds a hash set per group; the
  pre-aggregated `commit_churn` CTE produces the same result via two
  cheap scans and a join.

## [0.4.5] - 2026-06-13

### Added — CHM borrows, AI surfacing, step-summary, auditable tooltips

A v0.4.5 batch built end-to-end with empirical validation before
locking thresholds, denominators, output formats, and visual design.
Two false-positive findings (UI-3, F65 carryover) closed honestly with
documented architectural reasoning rather than dead code.

- **File-level Maintainability Index** (Coleman 1994 + SEI 1997) is
  surfaced on hotspots across every emitter (CSV/Markdown/SARIF/JSON/
  SPA). CodeLore has computed `mi_sei()` polyglot since v0.1.0 via the
  vendored Mozilla rust-code-analysis fork; the column was ingested
  into `complexity_metrics.mi` but never queried. The hotspots SQL now
  joins `entities` filtered to `kind='unit'` to pull the file-level
  value (per-function MIs are mathematically unsound to average).

- **Repo-relative MI bands** (Low / Moderate / High by percentile rank
  within the analyzed repo) instead of the literature's absolute
  Coleman/SEI thresholds. Empirically validated on CodeLore's own
  codebase: MI values range `[-137, +104]` with median ≈2.7;
  applying the (≥85 / 65-85 / <65) thresholds verbatim would classify
  100% of well-maintained files as "low maintainability" — useless for
  triage. New `analyses/mi.rs` module exports `MiBand` and `MiRollup`.
  Trade-off (bands aren't comparable across repos) documented in
  `research-foundations.md`.

- **Behavioral coupling graph density** scalar on the SPA KPI tiles —
  `edges / (V·(V-1)/2)` over the Fisher-significant coupling pairs.
  CHM ships density on the static dep graph (JS/TS only); we compute
  it on our richer behavioral graph (Newman 2010 §6.10 formula, same
  algorithm, different signal). Empirical on CodeLore: 0.0275 — modular
  / typical production codebase.

- **Per-file AI attribution percentage** on hotspots. Surfaces the
  share of commits touching each file with `ai-assisted` or
  `ai-authored` attribution (per identity::bots). The SPA's "AI
  Attribution" toggle was a placeholder since v0.4.0 — now wired
  end-to-end with a continuous color ramp (no AI → pale, all AI →
  red). Empirical distribution on CodeLore: median 75%, mean 70.6%.

- **`--format step-summary`** GFM Markdown emitter sized for GitHub
  Actions' `$GITHUB_STEP_SUMMARY` (1 MB cap). Original spec
  (`--format spa --embed`) is structurally impossible because the SPA
  HTML is 1.3 MB dominated by ECharts, GitHub sanitizes `<script>`
  tags, and stripping the chrome saves <10 KB. The redesigned emitter
  produces a 2-15 KB GFM summary with KPI table, top-10 hotspots
  (MI band emoji per file), MI band breakdown (unicode bar chart),
  coupling density line, knowledge-islands `<details>` collapsible.
  Documented workflow snippet in `docs/advanced-usage.md`.

- **Per-metric provenance tooltips** on the SPA dashboard — `?` icons
  next to every KPI tile label and hotspot table column header. On
  hover (or keyboard focus), a popup surfaces the metric's formula in
  plain English plus a link to the matching section in
  `research-foundations.md`. The brand-defining "auditable formulas"
  promise made visual. CSS-only show/hide (no JS event listeners,
  no leak risk per F71 lesson) and theme-inherited via existing CSS
  variables (light/dark mode work automatically). ~14 tooltips at
  runtime.

- **Research foundation citations** in `docs/research-foundations.md`:
  Coleman et al. 1994 (MI formula), SEI 1997 (variant modifier),
  Newman 2006 (modularity foundation), Blondel et al. 2008 (Louvain
  algorithm — forward reference for v0.5.x), Ben Khalfallah 2025
  TOSEM (CHM precedent we adapted from).

- **Hotspot table** gains MI and AI % columns with band emoji
  (🟢 top quartile / 🟡 middle / 🔴 bottom) — these were data
  fields already populated by earlier commits but the SPA table
  hadn't surfaced them.

### Changed — SQL planner work

- **Coupling self-join over pre-filtered CTE.** `build_coupling_sql`
  introduces a `filtered_changes` CTE that pre-filters `changes`
  against `good_commits` ONCE and reuses it in both the `file_revs`
  aggregate AND the `pairs` self-join. DuckDB materializes the CTE
  (5 CTE_SCAN operators in the new plan). On CodeLore the self-join
  cardinality drops from 1412² to 266² (28× reduction). On Linux-
  kernel-scale repos: 1M² → 100K² — an order of magnitude.
  Semantic output proven byte-identical via git-stash baseline.

### Fixed

- **SPA dashboard resize listeners no longer leak.** Every ECharts
  widget render registered an anonymous `window.addEventListener(
  'resize', …)` that captured the chart instance; on re-render
  (color-mode toggle, theme switch) the new listener added without
  removing the old. ResizeObserver per container replaces this —
  disconnects any prior observer before installing a new one.
  **Bonus fix**: ResizeObserver also fires on container-level
  dimension changes (sidebar collapse), which `window.resize` missed.

### Notes

- **UI-3** ("xray entities live-at-HEAD pre-filter") audited and
  marked **Not a bug** — `ingest_complexity_at_head` only populates
  `complexity_metrics` for files in the working tree at HEAD, so the
  xray query is already implicitly live-at-HEAD via the ingest
  invariant. Empirically: 110 paths in current output, 110 paths
  after the proposed filter, 0 complexity_metrics paths absent from
  changes. No code change.

- **F60** (stream `GitCliRepo` log) closed — `GitCliRepo` is only
  used as a differential-test oracle. Production walker (`GixRepo`)
  already streams chunks through a crossbeam-channel.

- **F70** (drop `idx_changes_rev` + `idx_clones_group`) closed as
  **Won't Fix** — schema comment ("rev-prefix scan benefits from a
  dedicated index too") indicates the original author profiled when
  adding these. Dropping blind reverts a measured decision; no
  contrary empirical evidence available.

- **F69** (totals-CTE → window function) deferred to v0.5.x marked
  bench-gated. Need an `EXPLAIN ANALYZE` comparison on a 100k+ commit
  fixture before locking the rewrite — the kind of "is the planner
  actually doing what we think" measurement that this batch
  validated as essential.

- **v0.5.x admin-portal redesign** queued: Alpine.js 3.15.8 + Alpine
  persist + Tailwind v4 + DaisyUI 5 + Plotly basic 3.3.1 spike (the
  user's validated stack from a sibling project). Replaces vanilla
  CSS + ECharts with admin-dashboard-grade component primitives.
  This v0.4.5 commit's hand-rolled tooltips will be re-implemented
  as DaisyUI `tooltip` components during that phase — the rework is
  the accepted cost of shipping the auditable-formulas brand
  promise on v0.4.5 schedule rather than delaying the release for an
  architectural pivot.

## [0.4.4] - 2026-06-11

### Changed — SQL planner simplification + SIMD line counting

- **Hash-aggregation rewrite for "live-at-HEAD" CTE (F63)** — replaces
  the `ROW_NUMBER() OVER (PARTITION BY path ORDER BY date DESC, rowid
  ASC)` + `WHERE rn = 1` pattern with `arg_max(change_type, ROW(date,
  -rowid))` GROUP BY path. DuckDB struct lex-compare reproduces the
  original tiebreak in a single streaming pass — O(K) memory where K
  = distinct paths, vs the prior partition-sorted O(N) materialisation.
  Applied to `query_live_paths`, `entity_churn::live_paths`,
  `knowledge_islands::live_paths`, and `code_age::live_paths_at_anchor`.

- **`arg_max`/`first(... ORDER BY)` for "top author per entity" (F61)**
  — replaces `ROW_NUMBER + JOIN`-style winner selection with single
  grouped aggregates in `authors.rs`, `ownership.rs`, `main_dev.rs`,
  and `knowledge_islands.rs::main_per_path`. Strings can't be inverted
  for `arg_max` struct ordering, so the ASC-author tiebreak uses
  `first(author ORDER BY metric DESC, author ASC)` which collapses
  what was a separate `ranked` / `with_rank` / `last_author_per_path`
  CTE plus self-join into one aggregation step. Deterministic output
  unchanged on every test fixture.

- **SIMD line counting via `bstr::find_iter` (F66)** — `count_lines`
  in `gix_repo.rs` previously did `bytes.iter().filter(|&&b| b ==
  b'\n').count()`. Rewritten as `bytes.find_iter(b"\n").count()` via
  the already-imported `gix::bstr::ByteSlice` trait — internally uses
  `memchr` SIMD scanning. No new dependency; gix re-exports bstr.

### Notes

- F65 ("double `is_worktree_dirty` on cache miss") audited and marked
  **Not a bug** in the deep-analysis report: the two call sites in
  `FactsDb::open_or_ingest_with_cache_root` are in the cache-HIT and
  cache-MISS branches respectively (mutually exclusive). At most one
  fires per invocation. No code change.

- F60 ("stream `GitCliRepo::walk_commits` output") still **Active** —
  `parse_git_log_stream` requires a two-record lookahead to pair
  pretty blocks with name-status pairs, which a streaming reader
  can't do without a parser rewrite. Deferred to a follow-up batch.

## [0.4.3] - 2026-06-11

### Fixed — backend OOM protection + UX defaults + perf polish

A bundled v0.4.3 batch covering one production-blocking OOM, two
"smart defaults" UX wins, and three perf cleanups. All semantic-
equivalent — output values unchanged on every existing fixture.

- **Kamei O(K) memory rewrite (F61)** — `enrich_history` and
  `enrich_experience` previously materialised `LIST(...) OVER w`
  per row, allocating O(K²) memory per partition. On directory-
  skewed monorepos (Vue/JS projects with 26k+ touches under `src/`)
  this OOM'd with 19 GiB exhausted in production. Replaced with
  DISTINCT + hash-grouped COUNT for ndev/nuc/age and ROW_NUMBER
  for sexp — O(K) memory regardless of partition skew. Semantic
  shift to strict-prior `<` (was `<=`); no-op on real repos where
  commits are distinct-second by construction. New regression test
  `dir_skew_does_not_oom_or_timeout` builds a 60-commit fixture
  under a single hot dir and asserts ingest completes in <30s.

- **Auto-`.gitignore` respect (F62)** — CodeLore now respects the
  repo's `.gitignore` + `.git/info/exclude` + `.codeloreignore` by
  default. Vendored deps (node_modules, target, dist), build
  outputs, lockfiles, locales — none show up in hotspots unless
  `--include-ignored` is passed. New `paths_filter` module backed
  by the `ignore` crate (same engine ripgrep / bat use). Applied
  to BOTH the HEAD walk (clones + complexity) and the commit walk
  (changes ingest). Replaces 3 hardcoded `.git|target|node_modules`
  match arms and 2 ad-hoc `.codeloreignore` parsers.

- **SPA emit success message (F68)** — `--format spa` was silent on
  success. CLI now prints output path, size, and a clickable
  `file://` URL after the dashboard is written.

- **Theme re-render registry (F57)** — light/dark toggle now
  repaints every ECharts widget so axis labels, grids, and gradient
  colors pick up the new CSS variable values. ECharts caches
  resolved colors at setOption time, so the prior toggle left
  charts stuck with the previous theme's palette. Implemented as a
  `window._codeloreRerenderers` registry pushed by each widget.

- **Chart instance disposal (F64)** — every `echarts.init(container)`
  without a corresponding `dispose()` leaked the prior instance +
  bound event listeners. Surfaced on repeated color-mode or theme
  toggles. Fix: call `echarts.getInstanceByDom(container)?.dispose()`
  before each re-init. Applied to all 5 widget render fns.

- **Literal-prefix grouping (F58)** — replaced `fancy-regex` (a
  backtracking engine) with the standard `regex` crate for plain-
  text path-prefix grouping rules — the vast majority of
  `--group-file` lines like `src/foo => Engine`. Three-tier
  compilation in `GroupPattern`: `Literal(prefix)` (no regex
  engine), `Std(regex)` (linear-time), `Fancy(fancy_regex)` (only
  for lookaround/backreferences). The `apply_grouping` rayon
  parallelisation already shipped in v0.3.4; this strips the regex
  engine from the inner kernel.

Deferred to v0.4.4: F59 (gix blob reads vs working-tree disk —
needs bare-repo path planning), F60 (streaming git-log — needs
BufReader refactor + parser state-machine update).

## [0.4.2] - 2026-06-11

### Added — `--format spa` widget completeness (v0.4.2)

Five new widgets, the hotspot color-mode toggle, and a light/dark
theme switcher. Builds on v0.4.0's SPA dashboard scaffold without
adding any new build-time dependencies (still ECharts + d3-hierarchy
only).

- **W7 Knowledge map** — toggle on the hotspot circle-pack that
  re-paints leaves by primary author per file (derived from
  `entity_ownership` — max added LoC wins per path) using a stable
  15-color palette.
- **W8 X-Ray sunburst** — function-level cognitive complexity in a
  three-level radial hierarchy (top-level path segment → file →
  function). Top-500 functions by cognitive complexity. New
  `output::spa::run_xray(db, limit)` helper that joins
  `complexity_metrics` with `entities` on `(path, name, rev)` to
  surface the line range alongside each score. CodeScene paywalls
  this surface; CodeLore ships it free.
- **W9 Trends multi-line** — monthly revision counts for the top-10
  hotspot paths. New `output::spa::run_trends(db, &paths)` helper.
- **W10 Calendar heatmap** — per-day commit volume rendered as
  one calendar block per year using ECharts' native calendar coord
  system. New `output::spa::run_daily_commits(db)` helper.
- **W11 AI-attribution toggle** — third color-mode on the hotspot
  circle-pack (placeholder; per-path AI rollup lands in v0.4.3).
- **Light / dark theme toggle** — header button with `localStorage`
  persistence. Light theme defined via `data-theme="light"` on
  `<html>` overriding the `:root` CSS variables; widget bodies
  (including ECharts axis / grid colors) pull from those variables
  via the new `getCssVar(name)` JS helper.

Shape changes:
- `SpaDashboard` gains four fields: `entity_ownership`, `xray`,
  `daily_commits`, `trends`. Each is `skip_serializing_if = "Vec::is_empty"`.
- New row types: `XRayEntry`, `DailyCommit`, `TrendPoint`.

End-to-end smoke (live CodeLore repo): 1.2 MB self-contained HTML
embedding 301 hotspots + 494 xray functions + 7 daily-commit rows
+ 10 trend points + 47 coupling pairs. All 9 widget markers and
color-mode toggles verified present.

## [0.4.1] - 2026-06-11

### Fixed — deep-analysis findings F43-F54 (v0.4.1 perf batch)

Twelve perf findings surfaced by the post-v0.4.0 audit, all validated
against current source and shipped together. None change semantics —
each fix produces output identical to the prior implementation on the
existing test fixtures, with the cost of intermediate string
allocation, distinct-tracking aggregation, or per-node cursor
allocation removed.

**SQL distinct-tracking cleanup (F47-F54, plus a follow-up site)**
— extends the v0.3.4 F42 pattern to 9 more sites where the `DISTINCT`
in `COUNT(DISTINCT col)` was provably redundant given either the
`changes` PK `(rev, path)` or an upstream CTE that already produced
unique rows. Plain `COUNT(col)` is semantically identical and skips
DuckDB's distinct-tracking overhead on hot aggregation paths.

- **F47** `coupling.rs::pairs` — `COUNT(DISTINCT a.rev)` → `COUNT(a.rev)`
- **F48** `churn.rs::entity_churn` — `COUNT(DISTINCT c.rev)` → `COUNT(c.rev)`
- **F49** `code_health.rs::author_revs` — same fix on `(path, author)` group
- **F50** `ownership.rs::author_revs` — same fix
- **F51** `code_age.rs::per_path` — same fix
- **F52** `communication.rs::pairs` — `COUNT(DISTINCT a.path)` → `COUNT(a.path)`
  (upstream `author_files` is `SELECT DISTINCT`)
- **F53** `authors.rs` final select — `COUNT(DISTINCT cls.author)` → `COUNT(cls.author)`,
  same for both CASE WHEN forms (n_humans, n_bots), plus the HAVING clause
- **F54** `soc.rs::rev_sizes` — `COUNT(DISTINCT path)` → `COUNT(path)`

**Non-SQL perf (F43, F44, F45, F46)**:

- **F43** `gix_repo.rs::count_loc::read_blob` — drops the redundant
  `obj.data.clone()` and uses `std::mem::take(&mut obj.data)` to move
  the blob bytes out of `gix::Object` instead of re-allocating +
  memcpy'ing up to `MAX_DIFF_BLOB_BYTES` (1 MiB) per changed-file per
  commit. `gix::Object` implements `Drop`, so a direct partial move
  isn't permitted — `mem::take` swaps in `Vec::default()` (no
  allocation) and returns the original.
- **F44** `gix_repo.rs::count_loc` — short-circuits the histogram
  diff for pure additions (`old_oid.is_none()`) and pure deletions
  (`new_oid.is_none()`). Counts newline-terminated lines in the
  non-empty side directly via a single byte scan, skipping the
  `InternedInput` tokenisation + `Algorithm::Histogram` slider pass.
  New private helper `count_lines`.
- **F45** `clones/fingerprint.rs::walk_preorder_internal` and
  `clones/extractor.rs::visit` — both rewritten as iterative
  pre-order traversals that allocate a SINGLE `TreeCursor` per
  invocation regardless of subtree size. Previous recursive forms
  called `node.walk()` at every AST node, allocating one cursor per
  node on a hot path (deep ASTs → tens of thousands of cursor allocs
  + drops). Pre-order semantics preserved exactly; nested-function
  emission behaviour preserved.
- **F46** `output/html.rs` + `output/spa.rs` — both replace the
  chained `String::replace(...).replace(...).replace(...)` pattern
  with a single-pass templating helper at
  `output::template::substitute`. The chained form allocated a fresh
  `String` per call, copying the growing intermediate buffer each
  time — for the SPA emitter (which embeds the ~1.1 MB
  `echarts.min.js` blob plus widget glue plus the per-analysis JSON
  data block), the chained form copied that multi-megabyte
  intermediate 7 times per emit. The single-pass form allocates one
  output `String` pre-sized from template length + replacement value
  lengths and writes substitutions in one scan. New unit-test
  coverage in `output::template::tests`.

## [0.4.0] - 2026-06-11

### Added — `--format spa` interactive dashboard emitter (v0.4.0 first slice)

A single self-contained HTML dashboard that mirrors the
CodeScene-equivalent surface end-to-end, opt-in via the `spa`
Cargo feature so default builds (`cargo install codelore`) remain
offline-clean with zero JS dependencies.

- **`build.rs`** fetches Apache ECharts 6.1.0 + d3-hierarchy 3.1.2
  from jsDelivr at SHA-pinned URLs the first time the `spa` feature
  is enabled, caches them in `OUT_DIR`, and embeds them via
  `include_str!`. If jsDelivr ever serves bytes that don't match the
  pin, the build fails loud — no silent supply-chain swaps. Adds
  `ureq` (HTTP) as a build-dep; runtime deps unchanged.
- **`--format spa -o codelore.html`** wires through the existing
  CLI dispatch, mirrors the `--format sqlite` precedent (bypasses
  `--analysis` and runs the analyses the dashboard needs:
  `hotspots`, `summary`, `code_health`, `coupling`,
  `knowledge_islands`). Coupling + knowledge-islands degrade
  gracefully to empty on tiny fixtures.
- **6 widgets** ship in v0.4.0:
  - **KPI tiles** — at-a-glance metrics: files analyzed, commits,
    distinct authors, median code health, cognitive p95, knowledge
    island count, coupling pair count.
  - **Hotspot circle-pack map** (the signature CodeScene view) —
    files sized by churn, colored by complexity, nested by
    filesystem hierarchy. Implemented as an ECharts `custom`
    series fed by `d3-hierarchy.pack()`.
  - **Hotspot table** — sortable, filterable drill-down with the
    existing `output/html.rs`-style 500-row pagination + 80 ms
    debounced filter.
  - **Change-coupling sankey** — top-30 coupling pairs by
    combined score via the native ECharts `sankey` series.
  - **Knowledge islands** (CodeLore differentiator) — ranked table
    of files where the primary author has departed and no
    substantial other owner exists. Auto-detected, no manual
    ex-developer marking. Surfaced with a "CodeLore differentiator"
    badge.
  - **File detail drawer** — side panel that opens on click of any
    circle / table row / sankey node and aggregates hotspot,
    knowledge-island, code-health, and coupling-partner data for
    one path. ESC or × closes.
- **Vendored JS attribution**: ECharts (Apache-2.0) and d3-hierarchy
  (ISC) — both already in `deny.toml`'s license allow-list. Pin
  table in `crates/codelore-lib/build.rs::ASSETS` IS the supply-chain
  manifest; reviewers can re-fetch and hand-verify.
- **Output size**: ~1.2 MB self-contained HTML for a 300-hotspot
  repo (most of which is the ECharts payload). Gzipped over the
  wire: ~400 KB.

Validation:
- 3 unit tests + 1 end-to-end integration test cover widget
  markers, embedded JSON shape, ECharts/d3-hierarchy payload
  presence, and the XSS-escape on `</script>` terminators.
- Smoke-tested on the live CodeLore repo: emits a real dashboard
  with all 6 widgets populated. Opens in any browser, runs
  offline, requires no server.

### Fixed — F40 follow-up: entity-name disambiguation for PK constraint

Hotfix to the v0.3.4 F40 fix surfaced by the SPA smoke run on
real repos. `dedup_entities` now suffixes every entity name with
its line range (`"foo@start-end"` or `"<anonymous>@start-end"`).
Before this, the F40 fix correctly surfaced anonymous /
overloaded entities into Rust memory but the
`(path, name, rev)` PK on `entities` / `complexity_metrics`
rejected the second row with a UNIQUE-constraint violation,
aborting ingest on any closures-heavy or overload-heavy codebase
(JS/TS, Python, Rust async blocks, C++ overloads). The
line-range suffix is the stable identity for unnamed entities
and the unique key the PK needs without a schema migration.

## [0.3.4] - 2026-06-10

### Fixed — deep-analysis findings F38 + F40 + F41 + F42

- **F38 (perf, correctness-preserving)** — Kamei enrichment
  (`ndev`, `nuc`, `age`, `sexp`) replaces the path-self-join shape
  (which was `O(K²)` per hot path) with per-path / per-(dir,author)
  running aggregations via DuckDB `LIST(...) OVER (... RANGE BETWEEN
  UNBOUNDED PRECEDING AND CURRENT ROW EXCLUDE CURRENT ROW)`. The
  RANGE frame preserves Kamei's same-day inclusion semantic exactly.
  Per-commit DISTINCT counts come from `LIST_DISTINCT(FLATTEN(LIST(
  ...)))` across the commit's paths. Hot files (lockfiles, top-level
  manifests, vendored config) no longer dominate ingest wall-clock.
  Regression test in `kamei_test.rs` validates the windowed
  semantic.

- **F40 (correctness — silent data loss)** — `dedup_entities` keys
  by `(name, start_line, end_line)` instead of `name` alone. Tree-
  sitter walkers report multiple anonymous functions per file with
  identical name (`<anonymous>` or empty for closures, lambdas,
  generator expressions). The old name-only dedup silently dropped
  every anonymous entity after the first, leaving zero
  `complexity_metrics` rows for closures-heavy files (JS/TS,
  Python, Rust async blocks). The line-range tuple is the closest
  thing to a stable identity for unnamed entities.

- **F41 (perf)** — `apply_grouping` now matches paths against the
  group map's regex set in parallel via rayon. The regex set is
  immutable (`Send + Sync`) and shares freely across workers; the
  serial INSERT into the `_grouping_v1` temp table happens after
  the parallel collect. Pre-fix the loop was single-threaded on
  the main thread — for monorepos with `paths × rules` in the
  millions, this dominated `apply_grouping` wall-clock.

- **F42 (perf, cleanup)** — drops the redundant `DISTINCT` in six
  `COUNT(DISTINCT rev)` sites (`revisions`, `hotspots`,
  `code_health`, `coupling`, `main_dev`, `communication`). The
  `changes` table has `PRIMARY KEY (rev, path)`, so per
  `GROUP BY path` the `rev` column is already unique within each
  group — `COUNT(rev)` equals `COUNT(DISTINCT rev)` equals
  `COUNT(*)`. Plain `COUNT` skips DuckDB's distinct-tracking
  overhead. Same logic applies to the `commits` table where `rev`
  is its primary key.

## [0.3.3] - 2026-06-10

### Fixed — deep-analysis findings F35-F37 + F39

- **F35 (correctness)** — `GitCliRepo`'s `parse_numstat_with_key` now
  handles git's brace-collapsed rename syntax. Git emits renames as
  either whole-path arrow (`old/path => new/path`) or with a common
  prefix/suffix collapsed into braces (`src/{old => new}/file.rs`,
  `src/{old.rs => new.rs}`, `a/{ => sub}/b.rs`). Pre-fix, only the
  whole-path form was handled — brace forms produced keys like
  `new}/file.rs` that never matched the raw stream's destination
  path, so the numstat-to-raw HashMap join fell through and reported
  `(0, 0)` line counts for every directory rename and shared-prefix
  rename. New helper `expand_rename_path_destination`; 7 unit tests
  cover all observed git brace shapes plus the legacy whole-path
  arrow form and the non-rename pass-through.

- **F36 (crash)** — `entity_effort.rs`'s `explain_if_requested` call
  now passes `params![row_limit]` (1 element), matching the SQL's
  single `LIMIT ?` placeholder. Pre-fix it passed
  `params![opts.min_revs, row_limit]` (2 elements), crashing
  `codelore analyze --analysis entity-effort --explain` with
  DuckDB's `Got 2, needed 1`.

- **F37 (crash)** — `clone_coupling.rs`'s `explain_if_requested`
  call now passes
  `params![opts.min_clone_node_count, opts.clone_similarity_floor]`
  (2 elements), matching `CLONE_PAIRS_SQL`'s two `?` placeholders.
  Pre-fix it passed `[]`, crashing
  `codelore analyze --analysis clone-coupling --explain` with
  `Got 0, needed 2`.

- **F39 (correctness)** — `gix_repo::changed_files_for_commit` now
  returns an empty `Vec` for merge commits (parents > 1), matching
  `git log --name-status`'s default merge-suppression that
  `GitCliRepo` inherits for free. Pre-fix the gix backend computed
  a first-parent diff for every merge while the CLI backend
  reported empty — divergent event streams under `--include-merges`,
  breaking the differential parity gate and inflating churn /
  hotspot / coupling metrics whenever merges were included. New
  regression test in `differential_repo_test.rs` exercises both
  backends' merge handling.

## [0.3.2] - 2026-06-10

### Fixed — deep-analysis findings F29-F34

- **F29 (correctness)** — under `--time-bucket`, `max_changeset_size`
  filter now counts files per PHYSICAL commit (not per collapsed
  bucket). Pre-fix, `coupling`/`soc`/`clone-coupling` SQL applied the
  filter to `changes_bucketed`, treating the day/week/month aggregate
  size as if it were a single-commit size — silently dropping every
  active period whose total distinct-files exceeded the threshold.
  New helper `analyses::coupling::good_commits_cte(bucket, use_lineage)`
  emits the bucketing-aware CTE: under bucketing a bucket survives iff
  `MAX(files per commit) <= max_changeset_size`. Two regression tests
  in `time_bucket_test.rs` lock the semantic.

- **F30 (robustness)** — clones walk + ingest now evaluate the
  vendored-dir skip list (`.git`, `target`, `node_modules`) against
  the **repo-relative** path components, NOT the absolute path. A repo
  located at `/Users/joe/target/my-repo` used to have `target` in
  every absolute path's components and silently skipped 100% of
  candidate files. Same `path.components().any(...)` predicate, but
  reads the post-`strip_prefix` relative path so legitimate user
  directories that happen to share a name with a skip token aren't
  collateral damage.

- **F31 (correctness)** — three `LEFT JOIN author_aliases aa ON
  aa.canonical = ...` sites (`knowledge_islands.rs:155`,
  `authors.rs:116`, `top_committers.rs:80`) replaced with a
  deduplicating subquery: `LEFT JOIN (SELECT canonical, BOOL_OR(is_bot)
  AS is_bot FROM author_aliases GROUP BY canonical) aa`. The
  `author_aliases` schema has `raw_email TEXT PRIMARY KEY`, so
  `canonical` is N:1 for multi-email authors — the original join
  multiplied every joined row by N, inflating `SUM(loc)`,
  `COUNT(DISTINCT commits)`, and burning the `LIMIT N` row budget on
  duplicates.

- **F32 (correctness)** — `codelore diff --base-cache <path>` now
  validates `cached.sha == base_sha` before reusing the cache. On
  mismatch (e.g. `main` advanced between PR runs, or a shared CI
  cache path reused across branches): warn, recompute via
  `analyze_at_rev`, overwrite. Pre-fix the cached `RevAnalyses` was
  consumed directly — silently poisoning every hotspot delta,
  coupling absence, and clones delta with an out-of-date base.

- **F33 (robustness)** — `cache::cache_path_with_root` now
  canonicalises `repo_path` before hashing the per-repo subdirectory
  name, matching the canonicalisation that `cache_key` already
  applies. Pre-fix, `codelore analyze .` and `codelore analyze $PWD`
  computed the same key but landed in different subdirectories,
  forcing a redundant ingest on every alternation.

- **F34 (perf + correctness)** — `gix_repo::count_loc` now returns
  `(0, 0)` for blobs that are either oversized (>1 MiB, matching
  Git's `core.bigFileThreshold` default) or binary (NUL byte in the
  first 8000 bytes, matching Git's own heuristic). Pre-fix the
  function loaded raw blob bytes for any OID and ran imara-diff
  unconditionally: a commit touching a 50 MiB SQLite database
  allocated 100 MiB of `Vec<u8>` per worker thread and produced
  nonsense `loc_added`/`loc_deleted` from random newline bytes,
  polluting hotspots / churn / code-health. `GitCliRepo` doesn't
  have this problem because `git log --numstat` reports `- -` for
  binary files — the gix backend now converges with that behaviour.

### Fixed — deep-analysis findings F26-F28

- **F26 (usability)** — `parse_rev_range` now accepts implied-HEAD
  shortcuts matching `git log`/`git diff` semantics: `main..` resolves
  to `main..HEAD`, `..main` to `HEAD..main`, and `..` to `HEAD..HEAD`.
  Same treatment for the three-dot form. Previously every implied-HEAD
  input failed with `malformed two-dot rev range`, forcing users to
  type `HEAD` explicitly — a needless break from standard Git CLI
  ergonomics. Three regression tests in `diff::prune_tests` cover the
  two-dot omitted-base, two-dot omitted-head, and three-dot
  omitted-head paths.

- **F27 (performance)** — `walk_commits` no longer parses every
  reachable commit on the main thread. The merge filter + date-range
  filter ran inside a `filter_map` that called `repo.find_commit(oid)`
  once per OID before chunked-rayon then called `find_commit` AGAIN
  on the worker for each surviving commit — two object-store lookups
  per surviving commit with the first one fully serialised. Filtering
  is now folded into `process_commit_oid` (returning
  `Result<Option<CommitEvent>>`), so the OID gather is pure index
  iteration and filtering parallelises across workers. F12's
  `commits.rowid ASC` invariant is preserved: the OID vec retains
  walk order, `par_iter().collect()` preserves per-chunk order, and
  the driver thread drains `None`s without inserting — so rowid still
  tracks walk order on the surviving subset. Validated by the
  differential repo test suite (GixRepo ↔ GitCliRepo event-stream
  equality across the 50-commit fixture).

- **F28 (robustness)** — `prune_stale_worktrees` order swapped:
  directory sweep runs FIRST, `git worktree prune` runs SECOND.
  Previously the prune ran before the sweep, so directories deleted
  in this run's sweep didn't have their `.git/worktrees/<name>/`
  administrative metadata cleaned up until the next invocation —
  single-shot users left orphan metadata indefinitely.

## [0.3.1] - 2026-06-10

### Fixed — deep-analysis re-audit findings (F22-F25)

- **F22 — `path_lineage` CTE now traverses same-second rename chains.**
  The recursive step used strict `co.date > l.current_date` to extend
  the chain, terminating prematurely when two sequential renames
  (`A → B` then `B → C`) landed in commits sharing the exact same
  second. Carry `commits.rowid` through the CTE and break date-ties
  via `co.rowid < l.current_rowid` — gix walks reverse-chronologically
  so newer commits receive smaller rowids, and the next step in a
  forward rename chain must come from a newer commit (hence smaller
  rowid). Regression test asserts `a.txt → b.txt → c.txt` merges
  under `c.txt` with 3 revs accumulated when both rename commits
  share the same second.

- **F23 — cache writes now use PID-suffixed `.tmp.<pid>` paths.**
  The fixed-path `cache_p.with_extension("duckdb.tmp")` allowed two
  concurrent runs on the same cache key (parallel CI jobs, multiple
  terminals) to clobber each other's in-flight writes. Each ingest
  now writes to a process-unique path; the proactive `remove_file`
  at write-start only removes the current PID's leftovers (PIDs are
  not recycled while their owner is alive).

- **F24 — global cache walk no longer aborts on a single bad entry.**
  `collect_duckdb_files_inner` propagated `entry.metadata()?` errors
  to the caller, so one broken symlink or permission-denied subdir
  anywhere under the global cache root would abort the entire walk →
  `prune_global_cache` would return without pruning → cache grew
  unbounded. Errors on individual entries now log-and-skip instead,
  per-subdirectory and per-entry.

- **F25 — pruners now sweep `.duckdb.wal` companions and stale
  `.tmp.<pid>` artifacts.** Both pruners filtered on
  `extension == "duckdb"` only, leaving orphan `.wal` files (from
  forced kills mid-write) and `.tmp.<pid>` artifacts (from crashed
  ingests) growing the cache disk usage indefinitely. New helpers:
  `delete_duckdb_with_companion` removes the `.wal` alongside the
  database; `cleanup_stale_tmp_files` age-gates `.tmp.<pid>`
  artifacts at 1 hour (longer than any realistic ingest, short
  enough to bound disk leak from frequently-crashing runs).
  Regression tests cover both behaviours.

### Fixed — T8-T12 follow-up findings (F18-F21)

- **F18 — `knowledge-islands` back-testing now applies the
  `--age-time-now` anchor inside the data CTEs.** The anchor was
  previously only filtered at the outer `DATE_DIFF` and `WHERE`, so
  `author_last_commit`, `live_paths`, and `per_path_author` still saw
  post-anchor history — yielding negative `days_since_main_active`
  values, post-anchor LoC sums, and incorrect "live" file
  classifications. Anchor now filters all three CTEs; bind site has 8
  placeholders. Back-testing produces a temporally-isolated view as
  intended.

- **F19 — `clone-coupling` now passes `with_no_row_limit()` to the
  inner `run_knowledge_islands` call.** A user-supplied `--rows 10`
  on the outer clone-coupling previously also capped the inner
  knowledge-islands lookup, mis-flagging any clone-coupling pair
  whose partner sat in island rank 11+ as `at_risk = false`. Inner
  call is now uncapped (matches the F2 pattern for the inner
  coupling sub-analysis).

- **F20 — HTML exporter paginates by 500 rows.** Previously the
  emitter rendered every row synchronously, freezing the browser
  ("Page Unresponsive") on 30k-row outputs. Added incremental
  `renderNextPage()` via `insertAdjacentHTML` plus "Show next 500"
  and "Show all (slow)" controls — each batch stays under the 100 ms
  UI-freeze perception threshold. Default page size: 500 rows.

- **F21 — `codelore-action` three robustness fixes.**
  - `v`-prefix auto-normalisation — inputs like `0.3.0` (no `v`)
    are now prepended to `v0.3.0` before constructing the release
    URL. Previously a missing `v` would 404.
  - Authenticated GitHub API call — release-lookup `curl` now sends
    `Authorization: Bearer $GH_TOKEN` (token from `github.token`)
    to avoid shared-runner rate-limit failures.
  - Portable absolute-path resolution — pure-bash `if [[ "$OUTPUT"
    = /* ]]; then ABS_OUTPUT="$OUTPUT"; else
    ABS_OUTPUT="$PWD/$OUTPUT"; fi` replaces the GNU-only
    `readlink -f` + python3 fallback chain (broke on plain
    macOS-without-Homebrew runners).

## [0.3.0] - 2026-06-10

### Added — strategic differentiators

- **T8 — `knowledge-islands` analysis (automatic bus-factor / knowledge-loss
  detection).** New behavioural signal — per-file risk indicator surfacing
  files where the primary author (by LoC) has effectively departed
  (`--departed-threshold-days`, default 90) AND no other contributor owns
  a substantial share (≥ 10% LoC). The first tool in the
  behavioural-code-analysis category to detect departures *automatically*
  from commit-date falloff — `CodeScene` requires manual marking of
  "Ex-Developers"; codelore composes mailmap canonicalisation + `author_
  aliases.is_bot` + second-precision `TIMESTAMP` to skip the ops labour.
  Output columns: `entity, main_author, ownership_pct,
  days_since_main_active, last_main_author_commit, n_substantial_others`.
  Sort: `ownership_pct DESC, days_since_main_active DESC, entity ASC` —
  highest-concentration-then-longest-departed first. Filters out
  bot-dominated files and binary/lockfile cases pre-emptively. 3
  regression tests + `docs/research-foundations.md` entry citing Bird et
  al. (FSE 2011), Avelino et al. (SANER 2016), Cosentino et al. (CHASE
  2015). New CLI flag `--departed-threshold-days N`.

- **T9 — `clone-coupling` × knowledge-loss intersection (`at_risk` field).**
  Every `clone-coupling` row now carries an `at_risk: bool` set true when
  either file in the pair is a knowledge-island. Sort flips at-risk rows
  to the top of the output. The literal-strictly-novel codelore signal:
  no other tool composes clone × co-change × knowledge-loss
  automatically. Graceful degradation — knowledge-islands sub-analysis
  failures fall back to `at_risk = false` for all rows with a tracing
  debug log. CSV gains a 19th column `at-risk`; Markdown shows `⚠`
  markers; JSON serialises automatically via serde.

- **T11 — `--format html` static-report emitter.** New single-file HTML
  output: embedded CSS + vanilla JS + JSON data, no external CDN, no
  framework. 9–10 KB typical output — well under the 200 KB soft cap
  for clean GitHub Actions artifact attachment. Light/dark mode via
  `prefers-color-scheme`. Sortable columns (click headers), free-text
  filter, print-friendly via `@media print`. Security: HTML metadata
  escaped (`& < > " '`); JSON data has `</` → `<\/` to prevent
  script-block breakout. 3 unit tests lock both behaviours. 8 analyses
  wired (`hotspots`, `code-health`, `knowledge-islands`,
  `clone-coupling`, `summary`, `revisions`, `authors`, `top-committers`);
  generic over `Serialize` so adding more is one match arm.

- **T12 — `codelore-action@v1` reusable GitHub Action.** New `action.yml`
  at repo root. Composite action — no Docker pull (~90s saved per run),
  no Node bootstrap, ~3s startup. Detects runner OS+arch, downloads the
  matching binary archive from Releases, runs `codelore analyze` with
  user-supplied flags, exposes `result-path` + `version-used` outputs.
  Supports all 5 GitHub-hosted runner families (Ubuntu x64/arm, macOS
  Intel/Apple Silicon, Windows). New `docs/github-action.md` documents
  6 common patterns: PR-mode SARIF, weekly knowledge-loss report,
  live-clones SARIF with at-risk priority, multi-analysis matrix
  strategy, code-maat-compat for dashboard migration, version pinning.

### Performance

- **T10 — CI speedup (`CARGO_INCREMENTAL=0`).** The CI workflow already
  had `Swatinem/rust-cache@v2`, `mozilla-actions/sccache-action@v0.0.10`,
  `cargo-nextest`, `paths-ignore` for docs, and `cancel-in-progress` for
  superseded runs. Adding `CARGO_INCREMENTAL: "0"` to the workflow env
  block closes the last missing best-practice — incremental compile
  artifacts in cached CI environments get written + cached but
  invalidated next run, wasting cache hit rate. Setting to 0 (per the
  Rust CI consensus — cargo docs, Swatinem/rust-cache README,
  ripgrep/cargo/clippy workflows) saves ~30–60s per cold job AND
  improves cache hit rate.

## [0.2.2] - 2026-06-10

### Fixed — correctness

- **F12 — `current_head_rev` and `query_live_paths` now use `commits.rowid ASC`
  as the same-second tiebreak** instead of the previous `c.rev DESC` (SHA-1
  lex). SHA-1 lex is arbitrary and could pick the parent commit as HEAD on
  same-second pairs (common with automated/scripted commits). Since gix
  walks reverse-chronologically (children before parents), `rowid ASC`
  for same-second pairs correctly selects the child. Deterministic across
  runs. F13's chunked walker preserves insertion order so this tiebreak
  remains valid under parallel processing.
- **F14 + F15 — `--time-bucket` now rejected at CLI boundary for analyses
  that don't support bucketing.** Previously 10 of 14 analyses either
  crashed with `Catalog Error: Table changes_bucketed does not exist`
  (F14 — used `materialize_if_needed` which is a no-op on the bucketed
  branch) OR silently returned 0 rows because the JOIN `c.rev =
  commits.rev` matched date-string-keyed bucketed revs against SHA-1
  hashes (F15 — `code-health` was the worst offender). New
  `AnalysisName::supports_time_bucket()` classifies — only `coupling`,
  `soc`, `hotspots`, `code-health` are bucket-compatible; others reject
  at `main.rs` with a descriptive error.
- **F16 — `code-age` and `entity-churn` now filter to live files only.**
  Previously deleted files cluttered output (a file deleted two years ago
  showed up with `age_months=24`). `code-age` uses an anchor-aware
  live-paths CTE (correct for back-test mode — `live as of anchor`);
  `entity-churn` uses a live-at-HEAD CTE.

### Fixed — performance

- **F13 — `GixRepo::walk_commits` now streams events through a bounded
  channel instead of eagerly collecting into a Vec.** F9 (v0.2.1)
  parallelised commit processing but called `par_iter().collect::<Vec<_>>`
  — gigabytes peak on 100k+ commit repos with rich changes per commit.
  New chunked rayon pattern: 1000-OID batches, each parallelised with
  `par_iter().map().collect()` (order-preserving within batch), then
  drained serially through a 256-slot `crossbeam_channel::bounded`.
  Peak memory now bounded by (chunk size + channel) regardless of repo
  size. Critically, order preservation across chunks is what allows
  F12's `commits.rowid ASC` tiebreak to remain correct under parallel
  processing.
- **F17 — standalone `run_clones` analysis now parallelises across
  cores.** F17 was the dual of F9: `populate_clones_at_head` (ingest
  path) was Rayon-parallelised but the standalone `--analysis clones`
  path still walked + tree-sitter-fingerprinted on the calling thread.
  Refactored to the same two-phase pattern: serial `WalkDir` + globset
  filter gathers candidates, then `into_par_iter().map().collect()`
  reads + fingerprints each.

## [0.2.1] - 2026-06-09

### Fixed — correctness

- **F8 — `GitCliRepo` raw/numstat zip corrupted line counts on submodule
  or binary-mismatch commits.** The previous implementation paired
  `--raw` and `--numstat` stream entries by positional index, but
  submodules emit raw lines without matching numstat lines (and
  vice versa for binary-filtered files). After a length mismatch,
  every subsequent file got its line counts paired with the WRONG
  path. Replaced with a `HashMap`-by-destination-path join, where the
  destination path is extracted from both streams (numstat's `old =>
  new` syntax becomes `new`; raw's `R`/`C` statuses use `path2`).
  Six new unit tests in `git_cli_repo::tests::f8_*` lock the path-key
  semantics and verify the unequal-stream regression.

### Fixed — correctness (compat-mode bit-exactness)

These changes ONLY affect `--code-maat-compat` output. The modern
default CSV remains unchanged. Each item closes a divergence found in
the feature-by-feature deep dive vs code-maat's Clojure implementation.

- **DEEP-1, DEEP-2, DEEP-3 — `coupling` CSV under
  `--code-maat-compat` now matches code-maat's verbose 7-column shape
  exactly.** Column names: `entity,coupled,degree,average-revs,first-
  entity-revisions,second-entity-revisions,shared-revisions` (drops
  `fisher-p` which has no code-maat equivalent). `degree` truncated to
  integer matching code-maat's `(int coupling)`. `average-revs` uses
  CEIL `((a+b)/2.0)` matching code-maat's `(math/ceil average-revs)` —
  previously the modern default's integer-floor differed by 1 from
  code-maat for any odd-sum pair.
- **DEEP-4 — `soc` threshold uses strict `>` under compat** matching
  code-maat's `(> n min-revs)`. Modern default keeps `>=` (more
  intuitive "SoC of at least N").
- **DEEP-11, DEEP-12 — `communication` `average` uses CEIL under
  compat** matching code-maat's `(math/ceil (m/average …))`; `strength`
  truncated to integer matching code-maat's
  `(int (m/as-percentage …))`. Modern default keeps floor + float for
  precision.
- **DEEP-15 — `summary` row STATISTIC NAMES under compat are
  hyphenated** `number-of-commits`, `number-of-entities`,
  `number-of-entities-changed`, `number-of-authors`. Previously PAR-5
  fixed only the column header (`statistic,value`); the row values
  still said `commits` etc., breaking downstream tools that filter on
  `if statistic == "number-of-commits"`.

### Fixed — robustness

- **F11 — `GixRepo::is_worktree_dirty` now includes untracked files.**
  The previous `into_index_worktree_iter` skipped the dirwalk entirely,
  reporting untracked-only repos as clean — diverging from
  `GitCliRepo` (which uses `git status --porcelain`, reporting
  untracked). Switched to the full `into_iter()` which unifies
  index-vs-worktree differences AND the dirwalk for untracked-file
  detection.
- **F10 — tree-sitter AST parsing now has a 2 MB file-size cap.**
  Previously oversized files (minified JS bundles, protobuf
  `.pb.cc` generated code, vendored single-file libraries like
  `sqlite3.c`) could exhaust stack or heap during tree-sitter parse.
  Three insertion points: `ingest_complexity_at_head`,
  `populate_clones_at_head`, and `analyses::clones::run_clones`. Skip
  is silent at `debug` log level — warning per-file would drown the
  console on JS-heavy repos.

### Added — performance

- **F7 (refined) — persistent cache now serves Parquet exports.**
  Previously `--format parquet` and `--format sqlite` bypassed the
  cache entirely via `needs_writable_db`. Parquet's `COPY ... TO`
  is built into core DuckDB and works fine on read-only connections;
  the bypass was over-cautious. SQLite output still bypasses (its
  `INSTALL sqlite; LOAD sqlite;` writes the extension registry on the
  source connection — read-only blocks that). Net: re-running
  `--format parquet` on a clean cache is now sub-second instead of
  full-history-walk slow.
- **F9 — `GixRepo::walk_commits` now parallelises commit processing
  across cores via Rayon.** The OID iteration uses
  `par_iter().map(...).collect::<Result<Vec<_>>>()` — order-preserving,
  so consumers see events in commit order. Each worker constructs its
  own thread-local `gix::Repository` from the `Send`-able
  `ThreadSafeRepository` clone. Memory cost: ~20 MB peak for a
  100k-commit repo (acceptable; the downstream `FactsDb` ingest pulls
  every event anyway). Speedup: roughly N × on N-core boxes for
  CPU-bound diff calculation, capped by gix object-resolution overhead.

### Added — migration UX

- **NEW-B — `codelore -a identity` now emits a helpful redirect**
  pointing migrating users at `--format sqlite -o facts.db` (the
  CodeLore equivalent of code-maat's raw-data dump, strictly richer
  with 8 fact-store tables vs code-maat's parsed-log seq). Previously
  the command returned a confusing 22-name enum error. The `identity`
  name is NOT registered in the canonical `AnalysisName` enum (it's a
  code-maat debug artifact, not a real analysis); the special-case
  redirect lives in `FromStr` ahead of the generic lookup.

### Removed

- **NEW-C — Deleted `Options.verbose_results` dead-code field.**
  The field had been a "soft gap" leftover since v0.1.x — declared but
  never read by any analysis, not bound to any CLI flag, and
  explicitly dropped from cache-key serialisation. Four references
  removed; behaviour unchanged.

## [0.2.0] - 2026-06-09

### Changed (BREAKING)

- **`authors` analysis now answers the per-entity Bird et al. (FSE 2011)
  risk-indicator question instead of the per-author commit leaderboard
  (PAR-1).** Output columns evolve from `[author, commits]` to
  `[entity, n_authors, n_humans, n_bots, n_revs, last_author, last_modified]`
  by default. Under `--code-maat-compat`, the CSV writer emits code-maat's
  legacy `[entity, n-authors, n-revs]` columns so scripts targeting that
  contract keep working. The previous per-author leaderboard behaviour
  moves to a new first-class analysis `top-committers`, enriched with
  LoC added/deleted, first/last commit dates, and `is_bot` flag from the
  identity layer. Migration: scripts that ran `codelore -a authors`
  expecting per-author output should switch to `codelore -a
  top-committers`; scripts expecting code-maat's per-entity output get
  it back via `codelore -a authors --code-maat-compat`.

### Fixed — correctness

- **`commits.date` promoted from `DATE` to `TIMESTAMP` so HEAD resolution
  and same-day chronology are precise (F1).** Schema v2 (cache key bumped
  from `schema_v1` → `schema_v2`, naturally invalidating existing caches).
  Previously, two commits sharing a calendar day forced a lexicographical
  rev tiebreak in `query_live_paths` and `current_head_rev`, silently
  picking the wrong HEAD on the final day and stamping `complexity_metrics`
  and `clones` rows with the wrong rev. `CommitEvent.date` carries a full
  `time::OffsetDateTime` (UTC-normalised; tz offset is currently discarded
  at the schema boundary — see schema comment for the tz-preservation
  roadmap). `code-age` and `abs-churn` use explicit `CAST(... AS DATE)` /
  `date_trunc('day', ...)` where day-grain aggregation is the intent.

- **`code-age` no longer returns negative ages on back-tests (PAR-2).**
  The SQL now filters commits with `commits.date <= anchor` before
  computing age, matching code-maat's `changes-within-time-span`. The
  anchor defaults to `now_utc()` (current instant, second precision);
  with `--age-time-now <date>` it becomes end-of-day of that date so
  "as of June 1" includes June 1's commits. The analysis also emits two
  new columns — `age_days` (whole-day precision; sort tie-break against
  `age_months`) and `last_modified` (calendar-date context column for
  triage) — exploiting the schema v2 timestamp precision.

- **`SoC` (Sum of Coupling) now pre-filters changesets by
  `max_changeset_size` (F5).** A single massive sweep (lockfile bump,
  monorepo-wide rename, vendored dependency import) no longer dominates
  the score of every file it touched. Mirrors the `good_commits` CTE
  pattern used in `coupling.rs`.

- **`clone-coupling` no longer silently drops clone pairs with shared
  revs between `min-clone-shared-revs` and `min-shared-revs` (F2).** The
  inner `run_coupling` call now uses
  `Options::for_clone_coupling_inner_coupling`, which lowers
  `min_shared_revs` to `min(min_shared_revs, min_clone_shared_revs)`. A
  clone pair co-changing exactly 3 or 4 times previously vanished from
  the candidate pool before clone-coupling's own filter ever saw it; it
  now surfaces correctly.

### Fixed — robustness

- **Persistent cache now skips WRITE when the working tree is dirty
  (F3 strengthen).** The read-time `tracing::warn!` already existed;
  the corresponding write-time skip did not, so a first run on a dirty
  tree would still poison the cache under the clean `head_sha` key. The
  fix falls back to an in-memory `FactsDb` on cache miss + dirty tree,
  with a `tracing::warn!` explaining `--no-cache` and "commit changes"
  as the two ways to suppress the notice.

- **`prune_stale_worktrees` resolves the cache root through
  `default_cache_root()` instead of bare `/tmp` (F4).** The earlier
  hardcoded `/tmp` collided across users on shared hosts and missed
  namespaced worktrees of the current user; routing through the same
  helper `add_worktree` already used keeps the two paths in sync.

- **`add_worktree` no longer leaks an empty directory under the cache
  root when `git worktree add` fails (F6).** `tmp.keep()` now runs only
  after the git command returns success — if git errors out (invalid
  rev, local corruption, lock error) `tmp` Drops cleanly and the
  tempdir disappears. Regression test
  `add_worktree_does_not_leak_tempdir_on_git_failure` locks the
  contract.

### Added — migration ergonomics

- **`--code-maat-compat` now flips CSV column headers for the four
  remaining parity-affected analyses (PAR-5).**
  - `summary`: `metric,value` → `statistic,value` under compat.
  - `code-age`: `entity,age_months,age_days,last_modified` →
    `entity,age-months` under compat (drops the extra precision /
    triage columns for legacy-tooling compatibility).
  - `communication`: `author-a,author-b,…` → `author,peer,…` under
    compat.
  - `ownership`: `entity,main-author,total-revs,fractal-value` →
    `entity,fractal-value,total-revs` under compat (drops the
    `main_author` triage column; matches code-maat's column order).
  - 8 lock-down regression tests in `par5_csv_compat_test.rs` cover
    both modes for each writer.

- **Migration section of `README.md` rewritten to document the
  code-maat → CodeLore divergences (PAR-7).** A "Modern defaults vs
  code-maat compatibility" matrix lists the eight surfaces where the
  default and the `--code-maat-compat` behaviour differ; a
  "Short-flag migration map" walks through the 10-flag long-form
  rewrite. The default `-a` divergence (CodeLore: `revisions`,
  code-maat: `authors`) is documented; CodeLore does not flip its
  default under `--code-maat-compat` (the explicit `-a authors` is the
  one-character rewrite migrating users need).

### Fixed — correctness (Phase 3 polish)

- **`code-age` `age_months` now uses interval-month semantics, not
  month-boundary-crossing (PAR-4).** Previously DuckDB's `DATE_DIFF(
  'month', a, b)` counted month-component differences, giving
  Mar 15 → Apr 1 = 1 month. The reference `joda-time` (and code-maat)
  semantic counts whole calendar months elapsed: Mar 15 → Apr 1 = 0;
  Mar 15 → Apr 16 = 1; Mar 31 → Apr 30 = 0 (one day short of a full
  month). Implemented inline in SQL via `12 * (year - year) + (month -
  month) - (1 if day_anchor < day_commit else 0)`. Five-case regression
  table covers boundary semantics.

- **`coupling` `min_revs` pivot now respects `--code-maat-compat`
  (PAR-6).** Default behaviour (per-file gate) unchanged: a pair where
  one file has 4 revs and the other has 20 is dropped under `--min-revs
  5` because the 4-rev file is filtered before pairing — the stricter,
  more defensible semantic. Under `--code-maat-compat`, the threshold
  moves to the pair-average level (`(revs_a + revs_b) / 2 >= 5`),
  matching code-maat's `coupling-algos.clj` `within-threshold?` logic.
  Both modes share a single SQL builder with stable placeholder
  positional binding; `min_revs` binds twice, only one branch's gate is
  live (the other is a `? IS NOT NULL` tautology — small redundancy for
  a non-branching caller).

### Added — documentation (Phase 3 polish)

- **New `docs/research-foundations.md` curated reference (PAR-9).**
  Single-source-of-truth doc mapping every behavioural analysis to its
  primary citation, the question it answers in plain English,
  practitioner heuristics for "what good values look like", and the
  source-file location. Each of the 15 parity + ★ analyses in
  `crates/codelore-lib/src/analyses/` now carries a one-line
  `Research basis: see docs/research-foundations.md entry "<name>"`
  rustdoc cross-link so the academic provenance is one hop from any
  source file. Replaces code-maat's scatter-across-13-Clojure-files
  citation pattern with a curated reference that doubles as a
  marketing/education asset.

## [0.1.4] - 2026-06-09

### Fixed — correctness

- **`complexity_metrics.loc` now records physical lines (ploc), not duplicate
  source lines (F-LOC).** `crates/codelore-lib/src/complexity/mod.rs` mapped
  both the `loc` and `sloc` columns to `m.loc.sloc()` — the `loc` column
  was silently a copy of `sloc`, and the actual physical-LOC count (which
  includes comments + blanks) was discarded for every ingested file.
  `rust-code-analysis` already exposed `ploc()` — it just wasn't called.
  One-character fix: `m.loc.sloc()` → `m.loc.ploc()` on the `loc` mapping
  only. **Behavioural impact:** the `loc` value in the `complexity_metrics`
  table changes for every record going forward; cached fact-stores keyed
  off the prior `loc==sloc` invariant become invalid (the schema-version
  field in the cache key already invalidates them naturally).

- **`GitCliRepo` invocations pass `-c core.quotepath=false` so non-ASCII
  paths match `GixRepo` (F-QUOTEPATH).** Git's default behaviour wraps
  paths with spaces or non-ASCII characters in `"…"` and octal-escapes
  the non-ASCII bytes (e.g. `café.rs` → `"caf\303\251.rs"`). Gix reads
  raw bytes directly from the object database and returns `café.rs`. So
  before this fix, the two backends silently split per-file aggregations
  (hotspots, churn, ownership) across two `path` values on any repo
  containing non-ASCII filenames. The injection happens at three sites —
  the `open()` rev-parse, the central `run_git()` helper, and the
  `check-mailmap` call inside `resolve_alias()` — covering every git
  subprocess `GitCliRepo` spawns.

- **`/tmp` fallback for the persistent cache + diff-mode worktrees is now
  user-namespaced (F-TEMP).** When `dirs::cache_dir()` returns `None`
  (rare — containers/sandboxes with stripped env vars), the cache root
  used to fall back to a bare `/tmp` and the per-repo join produced
  `/tmp/codelore/…` shared across all users. Subsequent users hit
  `EPERM` on directories owned by whoever ran codelore first. New
  `fallback_tmp_root()` reads `$USER` / `$LOGNAME` / `$USERNAME` (with
  a `pid<N>` last-resort suffix) and produces
  `/tmp/codelore-fallback-<id>`. `diff.rs::add_worktree` routes through
  the same `default_cache_root()` helper so both fallback paths share
  one source of truth.

- **`analyses::lineage::rewrite` handles lowercase SQL (NEW-C).** The
  alias-vs-keyword disambiguator used to test the next character's case
  (`is_uppercase` → keyword, lowercase → alias). It would silently
  misclassify `from changes group by …` — treating `group` as an alias
  and producing `FROM changes_lineage group BY …` (parse error). Fixed
  by switching the regex to case-insensitive (`(?i)`) and replacing the
  case heuristic with an explicit SQL-keyword whitelist (WHERE, GROUP,
  ORDER, LIMIT, JOIN, INNER, LEFT, RIGHT, OUTER, CROSS, NATURAL, ON,
  USING, UNION, INTERSECT, EXCEPT, WINDOW, QUALIFY, FETCH, SAMPLE,
  TABLESAMPLE, AS, WITH, ANTI, SEMI, ASOF). Two new regression tests
  in `analyses::lineage::tests` exercise both lowercase-keyword and
  lowercase-alias variants.

### Changed — internal

- **`scripts/cut-release.sh` local gate now matches CI's clippy invocation
  exactly.** Earlier the script ran a narrower `cargo build --release
  -p codelore-cli` as its sanity check, which missed
  `clippy::useless_conversion` in the NEW-3 code at v0.1.3 cut time and
  left CI to surface the failure. Local gate now runs
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  + `cargo fmt --all --check` before pushing, matching CI byte-for-byte.

- **`scripts/cut-release.sh` no longer trusts `gh run watch
  --exit-status`.** That command exits 0 for both `success` AND
  `cancelled` runs — and concurrency-cancelled runs were misread as
  green CI during v0.1.2 and v0.1.3 cuts, masking real failures.
  Script now explicitly fetches `gh run view --json conclusion --jq
  .conclusion` after the watcher returns and aborts unless the
  conclusion is literally `"success"`. Affected callers see the
  precise conclusion in the abort message.

- **`docs/codebase_analysis.md`** trait-signature reference updated:
  `resolve_alias` now shows `(name, email)` (post-NEW-A v0.1.3
  signature), and `is_worktree_dirty` is added to the listed trait
  methods (post-F3 v0.1.3).

## [0.1.3] - 2026-06-09

### Fixed — additional correctness findings surfaced post-v0.1.2

- **`Repo::resolve_alias` now accepts `(name, email)` so `.mailmap` name+email
  rules resolve in both walkers (NEW-A).** `.mailmap` supports two rule
  shapes: 3-token `Canonical Name <canonical@email> <old@email>` (email-only
  match) and 4-token `Canonical Name <canonical@email> Old Name <old@email>`
  (name+email match). Earlier, the trait method took only `email` — both
  `GixRepo` (passed `name: b""` to gix's SignatureRef) and `GitCliRepo`
  (passed `<email>` to `git check-mailmap`) silently missed all 4-token
  rules. The bug was invisible because `GixRepo::walk_commits` had its OWN
  inline mailmap resolution that already passed the actual author name —
  so `GixRepo::walk_commits` and `GitCliRepo::walk_commits` diverged on any
  real `.mailmap` using the 4-token form. Differential test fixtures didn't
  exercise 4-token rules, so the divergence shipped quietly. Trait signature
  is now `fn resolve_alias(&self, name: &str, email: &str) -> String`;
  callers pass `event.author_name` alongside the email. `GitCliRepo`'s
  per-event mailmap cache key extended to `(name, email)` to avoid silently
  sharing one resolution across distinct identities that happen to share
  an email. New test `mailmap_name_plus_email_rule_resolves` exercises both
  forms; the existing `differential_repo_test::resolve_alias_matches` now
  iterates over `(name, email)` pairs and asserts parity for both
  empty-name and paired-name probes.

- **Clone-extractor recursion descends into function bodies so nested
  helpers get their own fingerprints (NEW-B).** The tree-sitter walker in
  `clones/extractor.rs::visit` used to `return;` immediately after
  emitting a function's fingerprint — a comment claimed "nested functions
  become separate entries via the outer-loop walk that follows", but no
  such outer walk existed. The early return silently dropped every nested
  function/closure/IIFE from clone detection. For Python (`def` inside
  `def`), JavaScript (closures everywhere), and Rust (`fn outer() { fn
  helper() { ... } }`), this missed real clone families. The fix removes
  the `return;` so recursion descends into the function body and extracts
  nested fingerprints additively — the outer function's own fingerprint
  is unchanged (still computed via the same `walk_preorder_internal`
  pre-traversal that captures nested structure as part of its sequence),
  so existing clone-pair detection at the outer level is not regressed.

### Fixed — correctness + performance (closes the v0.1.2 deferred findings)

- **Cache hits on dirty working trees now emit a warning (F3).** The persistent
  cache key is hashed from `(canonical_repo_path, head_sha, opts, version,
  schema)` — explicitly NOT from worktree state. That's correct for the
  majority of analyses (`revisions`, `coupling`, `ownership`, `churn`,
  `messages`, etc. all read only committed history), but `hotspots`-style
  HEAD-time metrics computed by `ingest_complexity_at_head` and
  `populate_clones_at_head` read files from disk at ingest time and can
  silently mismatch the current worktree on a cache hit. Now: a
  `tracing::warn!` fires whenever a cache hit lands on a working tree with
  uncommitted modifications or untracked Tier-1 source files, telling the
  user to pass `--no-cache` if the dirty state matters. Detection uses gix's
  `Repository::status` API for `GixRepo` and `git status --porcelain` for
  `GitCliRepo`. New `Repo::is_worktree_dirty(&self) -> bool` trait method
  with a `false` default so future backends can opt in without breaking the
  existing API contract. Errors during detection are deliberately swallowed
  (return `false`) — a missed warning is strictly preferable to a hard
  analyze failure on an edge case like an unusual submodule layout.

- **`--after` date-range walk uses gix's `ByCommitTimeCutoff` for graph
  pruning (NEW-3 perf).** Previously the walker eagerly traversed the
  entire reachable commit graph via `repo.rev_walk([head]).all()`, then
  dropped commits in-memory via `.filter_map`. On large histories
  (linux-kernel size, 100k+ commits) this turned a "last 7 days" analysis
  into the same work as a full-history walk. Now: when `opts.after` is
  set, the walk runs with `Sorting::ByCommitTimeCutoff { seconds, order:
  NewestFirst }` so gix stops traversing the moment it crosses below the
  cutoff. The cutoff uses committer time (gix's primitive); the in-memory
  filter still enforces author-time semantics. Unusual rebases that move
  commit time far earlier than author time can now be dropped by the
  cutoff — but git's own `--after` flag has identical behaviour, so this
  CONVERGES `GixRepo` and `GitCliRepo` rather than diverging them. The 10
  differential cross-walker tests (which assert event-stream parity) all
  still pass on the existing fixtures. `--before` falls through to the
  existing in-memory filter unchanged — gix has no symmetric primitive
  for "walk forward until newer than" and `--before`-only is the rarer
  workflow.

### Added — internal

- **`scripts/cut-release.sh`** — codifies the release-cut procedure into a single idempotent script. Validates preconditions (clean tree, on main, in sync with origin, no existing tag, non-empty `[Unreleased]`), bumps the workspace version, flips the CHANGELOG section, pushes the release commit, waits for CI green, runs the `disable-ruleset → tag → push → restore` dance (auto-restoring the ruleset via `trap EXIT` so the repo is never left unprotected on interrupt or failure), and prints the release-monitor links. Captures the v0.1.2 lesson that ruleset `required_status_checks` does not reliably consume Check Runs even with `integration_id`. Supports `--dry-run` for safe preview and `--skip-ci-wait` for re-attempts after the commit has already gone green.

- **Dependabot ignore for `dtolnay/rust-toolchain`** in `.github/dependabot.yml`. The action's `@<version>` tag names the Rust toolchain to install, not the action's own release version — Dependabot was offering bumps to versions that don't yet ship as stable (closed PR #1 was 1.96.0 → 1.100.0 while 1.96.0 was still current). Coordinated Rust bumps happen deliberately at release-cut time via `scripts/cut-release.sh`, not weekly via Dependabot.

## [0.1.2] - 2026-06-09

### Fixed — correctness

- **Pre-flight banner failures now exit with the correct typed-error code.**
  The new `analyze` pre-flight gate (path-missing / not-a-repo / empty-repo /
  output-not-writable) was emitting `anyhow::bail!` without a wrapping
  `CodeLoreError`, so the chain-walk in `main()` fell through to the default
  exit code `1` instead of the spec §6.6 codes (`3` for repo errors, `5` for
  output errors). Replaced each `bail!` with `Err(CodeLoreError::Repo(...))`
  / `Err(CodeLoreError::Output(...))`. Closes the regression in
  `invalid_repo_exits_with_code_3` that I introduced in the banner commit
  earlier in this release.

- **Rename-lineage CTE now respects chronological ordering.**
  `materialize_path_lineage` walked the rename graph by joining
  `c.rename_from = l.current` recursively, with no chronological constraint.
  If a filename was renamed away and a DIFFERENT file later took the same
  name, the CTE would walk through the old rename and produce a spurious
  lineage edge — e.g. `A → B` in commit 1 plus `C → A` in commit 10 produced
  a fictitious `C → A → B` trace that merged two unrelated files' history.
  Fixed: the CTE now joins `commits.date` in both the seed and recursive
  step, and the recursive step adds `WHERE co.date > l.current_date` so
  only chronologically forward renames extend a chain.

- **`coupling` analysis no longer silently truncates significant pairs.**
  The SQL query had `LIMIT ?` as its final clause, applied BEFORE the Rust-side
  Fisher exact significance filter — so when `--rows N` was set, DuckDB
  returned the top-N pairs by degree, then Fisher dropped the non-significant
  ones, leaving the user with fewer than N rows even when more significant
  pairs sat just below the truncation point. Fix: remove `LIMIT` from the
  SQL builder, collect ALL candidates, apply the Fisher filter, then
  `Vec::truncate(rows_limit)` in Rust. The composite call sites
  (`code_health::materialize_centrality`, `clone_coupling::run_clone_coupling`)
  were already passing `Options::with_no_row_limit()`, so they're unaffected
  by the change. Memory cost: the in-memory `Vec` grows from `O(rows_limit)`
  to `O(passing-pairs)` before truncation — bounded by the SQL `WHERE`
  clauses (`min_revs`, `min_shared_revs`, `min_coupling_pct`,
  `max_coupling_pct`) so it stays modest in practice.

- **Hotspot score divisor `/10.0` → `/4.0` closes the documented-range gap.**
  The module-level docstring promised `hotspot_score ∈ [0, 10]` but the math
  capped output at `4.0`: `code_health` is computed inline as
  `100 × (1 − 0.40 × normalize(cognitive))`, bounded `[60, 100]` because the
  `0.40` weight limits the deduction; that makes `(100 − code_health) ∈ [0, 40]`;
  multiplied by two percent ranks (each `[0, 1]`) the unscaled max is `40`;
  dividing by 10 capped scores at `4.0`. Dividing by 4 lands them in the
  documented `[0, 10]` band, matching the CodeScene convention that
  `≈10 ⇒ "on fire"`. **Behavioural impact:** existing reports with a top
  hotspot of `3.78` will now report `≈9.45` for the same file — the RANKING
  is unchanged (only the absolute scale shifts), but anyone with thresholds
  (e.g. SARIF rule cutoffs, code-review alerts) calibrated against the old
  `[0, 4]` range needs to rescale them by ×2.5.

- **`clones` analysis short-circuit now covers all 4 supported formats.**
  Previously the `format == "csv"` guard on the early-exit branch meant
  `--analysis clones --format json|markdown|sarif` fell through to the
  full git-ingest path even though clones extraction is a HEAD-only
  filesystem walk — making non-CSV clones runs 10–100× slower and
  breaking them outright in non-git directories. Expanded the guard to
  `matches!(format, "csv" | "json" | "markdown" | "sarif")` and added
  per-format dispatch inside the short-circuit. Parquet/SQLite for
  clones remain unsupported (no clones emitter for those formats).

- **`communication` analysis is now rename-aware.** Conway's-law shared-
  work output was the last path-aggregating analysis missing canonical
  lineage rewrite: two authors who co-edited the same logical file across
  a rename were counted as if they edited two different files, under-
  counting team coupling on every history with renames. Added the
  standard `materialize_if_needed` + `lineage::rewrite` pair, mirroring
  the pattern in `entity_effort`, `messages`, `ownership`, etc.

### Added — distribution

- **Homebrew tap actually publishes a working formula.** The `emrecdr/homebrew-codelore` tap was created during v0.1.0 setup but stayed empty through v0.1.1 because the release workflow never produced a `.rb` formula — `[workspace.metadata.dist]` in `Cargo.toml` declared `installers = [..., "homebrew"]` and `publish-jobs = ["homebrew"]` but `.github/workflows/release.yml` is hand-rolled and never invokes `cargo dist`. Now wired end-to-end:
  - New `homebrew-publish` job in `release.yml` (`needs: [plan, build, release]`) downloads the build artifacts from the workflow's `upload-artifact` storage (bit-identical to the eventually-published Release assets, no CDN propagation race), computes SHA256 of the four macOS/Linux archives, renders `Formula/codelore.rb` from a heredoc template, checks out `emrecdr/homebrew-codelore` via SSH using the `HOMEBREW_TAP_DEPLOY_KEY` deploy-key secret, and pushes the regenerated formula. Idempotent (`git diff --cached --quiet && exit 0` skips no-op pushes).
  - Formula uses the modern nested `on_macos`/`on_linux` × `on_arm`/`on_intel` pattern (per current Homebrew Formula DSL docs); pinned 4-platform: macOS aarch64+x86_64, Linux aarch64+x86_64-gnu. Single `bin.install "codelore"` since the release archive has the binary at the top level. `test do` asserts `codelore --version` matches the formula version.
  - Cross-repo auth via SSH deploy key (no expiry, min-privilege, single-repo scoped) instead of fine-grained PAT — eliminates the yearly-renewal silent-failure mode.
  - v0.1.1 formula was backfilled manually so `brew install emrecdr/codelore/codelore` works against the existing release; v0.1.2+ get the formula auto-regenerated by the workflow.
- **README install section now leads with Homebrew** alongside `cargo binstall` and `cargo install --git`, replacing the "once a published release lands" placeholder copy from the v0.1.0 README.

### Removed — dead configuration

- **`[workspace.metadata.dist]` block removed from `Cargo.toml`.** The 35-line block had been inert since the release workflow was rewritten (`890537e: fix(release): replace broken SLSA generator with attest-build-provenance`) — `cargo dist` is not invoked anywhere, so the metadata block was misleading any reader who assumed cargo-dist was driving the release. Replaced with a short comment explaining the historical context and pointing at the actual `release.yml` workflow.

### Changed — documentation

- **`docs/RELEASING.md` rewritten to describe the actual release pipeline.** Previously claimed cargo-dist runs the build and handles pre-release detection; now documents the real `release.yml` job graph (`plan` → `build × 5` → `release` → `homebrew-publish`), the `protect-release-tags` ruleset gating tag creation on green CI, the `cargo binstall` auto-scan path, and the manual step needed for pre-release tag handling (since `softprops/action-gh-release` doesn't auto-flag pre-releases).
- **`docs/codebase_analysis.md` §8 release-pipeline line** now lists the actual workflow components (hand-rolled cargo build matrix, `attest-build-provenance`, deploy-key Homebrew publish) instead of "cargo-dist (6 target binaries)".
- **`docs/advanced-usage.md` workspace tree comment** updated: `release.yml` is "cargo-build matrix + SLSA L3 + Homebrew (on tag push)" instead of "cargo-dist + SLSA L3".
- **`docs/roadmap-v1.x-and-beyond.md` musl entry** rewritten: was framed around cargo-dist's `github-custom-runners` field; now describes the two real routes for adding a musl target to the current hand-rolled `release.yml` (musl-cross Docker image or musl-cross-make toolchain install).
- **README "Status" line** drops the `cargo-dist` mention; now lists the actual pipeline outputs (SLSA L3 binaries, distroless container, Homebrew tap, `cargo binstall` manifest).

## [0.1.1] - 2026-06-09

### Fixed — correctness defects discovered post-v0.1.0

- **`.codelorebots` extension hook wired into production (previously dead code).** `BotPatterns::from_repo()` existed and was unit-tested, but the production ingest pipeline called the free `identity::is_bot()` / `identity::ai_attribution()` functions that only consulted `DEFAULT_BOT_PATTERNS`. User-configured patterns from `<repo>/.codelorebots` were silently ignored — internal/custom bots were classified as `human` instead of `ai-authored`, polluting author counts and contribution metrics. `ingest()` now loads `BotPatterns::from_repo(&opts.repo_path)` once and passes it into `ingest_loop`, which routes both the bot flag (used by `author_aliases`) AND ai_attribution through the user-extensible patterns. Added `ai_attribution_with(patterns, …)` as the variant that honors `BotPatterns`.

- **Kamei history features (`ndev`, `nuc`, `age`, `sexp`) routed through canonical lineage.** `enrich_history` and `enrich_experience` joined `changes pchg` against `changes cchg` on literal `path`; a renamed file lost all pre-rename history under both the old and new path. `enrich(db, use_lineage)` now materializes `changes_lineage` first and routes the path joins through it when canonical lineage is on, so rename ancestry merges into the canonical post-rename name. Kamei JIT-SDP predictions for renamed files reflect their full history.

- **GitCliRepo now resolves `.mailmap` at walk time.** Previously the differential-test oracle set `canonical_author: None` with a stale comment claiming "matches GixRepo behaviour" — but GixRepo had been resolving via `gix-mailmap` at walk time since v0.1.0. Two walkers, two `canonical_author` columns, breaking the cross-walker parity invariant the differential tests assert. `walk_commits` now post-processes events with a per-unique-email cache that calls the existing `resolve_alias` helper, so a `git check-mailmap` subprocess runs once per author (not once per commit) and `canonical_author` matches gix's output bit-for-bit.

- **Parquet emitters route through canonical lineage.** `write_hotspots_parquet` and `write_revisions_parquet` had `FROM changes` hardcoded, so Parquet output split renamed files even when `--use-canonical-lineage` was on (default). Now both writers materialize `changes_lineage` and pull from the lineage-resolved source via the shared `analyses::lineage` helper, matching the CSV/JSON/Markdown emitters.

- **Canonical lineage wired into the remaining 9 path-aggregating analyses.** B.1 shipped lineage for `revisions`, `hotspots`, `coupling` as the canonical pattern. New shared `crates/codelore-lib/src/analyses/lineage.rs` module exposes `materialize_if_needed(db, opts)`, `source_table(opts)`, and `rewrite(sql, opts)`. The rewriter regex-handles both SQL conventions (`FROM changes` with qualified `changes.col` refs → `FROM changes_lineage AS changes`; `FROM changes c` with per-query alias `c.col` → `FROM changes_lineage c`). Applied to `churn` (3 functions), `code_age`, `entity_ownership`, `main_dev` (3 metrics), `ownership`, `entity_effort`, `messages`, and `soc`. All 12 path-aggregating analyses now honor `--use-canonical-lineage`. `revisions` + `hotspots`'s prior per-file copies of this pattern were collapsed onto the shared helper.

- **`.mailmap` and `.codeloreignore` edits now invalidate the cache.** The mutable-config content-hashing pass covered `--team-map-file`, `--group-file`, `.codelorebots`, and (after the first simplify pass) auto-discovered `.codelore-teams`. But two other repo-root files were missed: `.mailmap` (consumed by gix at walk time to canonicalize author identities; edits change every author-bearing analysis silently) and `.codeloreignore` (parsed by `build_clones_exclude_set` to filter clone-extraction targets; edits change clone families silently). Both are now hashed into the cache key via `canonical_json()`. Two regression tests in `options::tests`: edit-in-place mailmap and edit-in-place codeloreignore both invalidate.

- **`changes_lineage` temp table now has `path` and `rev` indexes** matching the base `changes` table's covering indexes. Without them, downstream `GROUP BY path` and `JOIN changes ... ON rev` aggregations against `changes_lineage` fell to full table scans, regressing query performance whenever canonical lineage was on (which is the default).

- **`gix_repo::count_loc` skipped for bit-identical Rewrite blobs.** When gix's rewrite tracker emits `diff: None` for a perfect 100% rename (gix's documented contract: source_id == id), the code was still reading both blobs and running a histogram diff — wasted I/O whose result was always `(0, 0)`. Now returns `(100u8, 0, 0)` directly.

- **Rename-aware aggregation via a canonical-lineage CTE (silent file-history split).** Every analysis that aggregated on `changes.path` was treating a renamed file's pre- and post-rename history as two separate entities. The `rename_from` column was captured at ingest by both `GixRepo` (after A.4 enabled rewrite tracking) and `GitCliRepo` — but ZERO analyses queried it. A file renamed once showed split revision counts; coupling pairs across the rename boundary went missing; hotspots ranked under-counted refactored files. New `facts::ingest::materialize_path_lineage` builds a `(old_path, canonical_path)` lookup via a recursive `WITH RECURSIVE` CTE that walks `rename_from` chains (depth cap = 50, deterministic latest-name-wins resolution); `materialize_changes_lineage` then projects `changes` with `path` replaced by the canonical lineage path. Wired into `revisions`, `hotspots`, and `coupling` as the canonical examples — the other 9 path-aggregating analyses gain `changes_lineage` support in `v0.1.2` via the same `build_sql(src)` pattern. New `Options::use_canonical_lineage: bool` (default `true`) gates the behavior; `--no-canonical-lineage` opts out, `--code-maat-compat` also flips it off (code-maat uses `--no-renames` and has never canonicalized). Cache key auto-tracks via `canonical_json()`. Empirical regression test against the `differential_repo` fixture's `old_name.rs → new_name.rs` chain: with lineage ON, `revisions` reports ONE entity with merged counts; with lineage OFF, the historical split returns for code-maat parity.

- **Real `loc_added` / `loc_deleted` in both repo walkers (SHOWSTOPPER).** Both `GixRepo` and `GitCliRepo` were emitting `loc_added: 0, loc_deleted: 0` on every change event — a `// Plan 1 stub` left from the walking-skeleton phase that was never wired in. Every churn-driven analysis in v0.1.0 silently returned zero: `abs-churn`, `author-churn`, `entity-churn`, `main-dev`-by-added-lines (every author tied at 0, so ranking was arbitrary), `entity-ownership`'s added/deleted columns, the `code-health` churn term (20% of the composite score), and Kamei `la`/`ld`/diffusion features. `GitCliRepo` now drives `git log` with `--raw --numstat` (replacing `--name-status`) and pairs raw lines with numstat lines by index; `GixRepo` uses `gix::diff::blob::diff_with_slider_heuristics` (Git's default histogram algorithm with slider postprocessing) to count added/removed lines between blob OIDs, and reuses the `DiffLineStats.insertions`/`removals` the rewrite tracker already computed for `Rewrite` events. New differential `tests/differential_repo_test.rs::line_counts_are_non_zero_and_match_across_walkers` asserts non-zero aggregate churn across the 50-commit fixture AND that the two walkers' totals match within 5% (small drift expected from root-commit edge cases).

- **`GixRepo` now detects renames at parity with `GitCliRepo`.** The pure-Rust walker had `diff_opts.track_rewrites(None)` set, so every rename surfaced as a `Deleted` event on the old path plus an `Added` event on the new path. `GitCliRepo` (via `git log --name-status`'s default `-M` detection) correctly emitted a single `Renamed { from, .. }` entry, so the two backends produced divergent change-type streams on the SAME commit history — silent file-history splits in every downstream analysis depending on which walker was selected. Now passes `gix::diff::Rewrites::default()` (50% similarity, copies off, 1000-file fuzzy limit — matches Git's `-M` defaults). New differential invariant `tests/differential_repo_test.rs::rename_commit_change_type_matches_across_walkers` asserts both walkers emit `Renamed` for the same commit and that no orphan `Deleted` event remains. Copies stay off because Git's default `git log` doesn't pass `-C`; preserves walker bit-equivalence.

- **`max_coupling_pct` was silently ignored.** Surfaced by the post-v0.1.0 deep-analysis pass. The field was wired through `Options` (default `100`) but `build_coupling_sql` only bound the lower threshold; pairs with degree above `max_coupling_pct` were returned regardless. Users who set `--max-coupling 80` to filter file-split / copy-rename artifacts got the unfiltered output. SQL gains an `AND degree <= ?` clause; bind list grows from 5 to 6. Regression test in `tests/coupling_test.rs::coupling_respects_max_coupling_pct` builds against `differential_repo`, observes the max coupling degree, then asserts that capping below half drops the top pair.

- **`rows_limit` propagation into composite analyses poisoned scores AND the cache.** `code_health::materialize_centrality` and `clone_coupling::run_clone_coupling` both invoke `coupling::run_coupling` to materialize the full coupling graph as a sub-computation. They were passing the parent `opts` straight through, so `--rows N` flowed into the SQL `LIMIT ?` and the inner result truncated to N pairs — centrality scores were then computed over an arbitrary 10-pair sliver, and clone-coupling silently dropped any clone whose partner sat outside the top N. Worse: `Options::canonical_json()` deliberately drops `rows_limit` (cosmetic output truncation; cache should hit regardless), so the corrupted result was *cached* under the no-row-limit key and poisoned subsequent runs that didn't pass `--rows`. New `Options::with_no_row_limit()` helper used at both nested call sites. Regression invariant in `tests/code_health_test.rs::code_health_score_invariant_under_rows_limit`: paths that survive truncation must carry the same score as in the unlimited baseline.

### Added — code-maat parity completion

- **`--team-map-file PATH` flag aliases author identities to team names** at ingest time. Mirrors code-maat's `-p / --team-map-file`. Two-column CSV (`author,team` with a required header row), applied as the LAST identity projection after mailmap normalization and bot filtering — the team-map sees the already-canonical author email and replaces it with the team name in `commits.canonical_author`. Every author-bearing analysis (`authors`, `author-churn`, `entity-ownership`, `main-dev`, `communication`, etc.) downstream sees the team name. Unmatched authors pass through unchanged (matches code-maat's `(get team-lookup author author)` fallback).
- **`.codelore-teams` auto-discovery.** If `--team-map-file` is omitted, CodeLore checks `<repo>/.codelore-teams` and loads it transparently. Mirrors the `.codelorebots` repo-root convention so a team-map can be checked into the repo without every contributor needing to set the flag.
- **New `crates/codelore-lib/src/identity/team_map.rs` module** with `load(path)`, `apply(map, author)`, and `discover(repo_root)` helpers. Strict parser: empty header, missing comma, blank fields, and duplicate authors all raise structured errors with the offending line number. Round-trip tested.
- **Cache key now hashes mutable-config FILE CONTENT, not just paths.** The team-map / group-file / `.codelorebots` paths are stripped from `Options::canonical_json()` and replaced with their sha-256 digests. Without this, a user editing the team-map CSV in place (same path) would silently see stale cached results because the cache key remained byte-equal across the edit — a known footgun in code-maat that bit users for years. Two regression tests in `options::tests`: (a) editing the team-map content invalidates the cache key, (b) the path is stripped so two machines with identical content hit the same cache entry.

- **`fragmentation` and `code-ownership` aliases for the `ownership` analysis.** code-maat exposes the Herfindahl-Hirschman fractal value under the analysis name `fragmentation`; CodeLore already computed the same value (`analyses/ownership.rs:64`) and emitted it alongside the main-author column, but `--analysis fragmentation` returned `UnknownAnalysisError`. Resolved via the same `FromStr` alias pattern as `refactoring-main-dev → main-dev-by-deletions`. Surfaced during validation: the canonical analysis name is `ownership`, but the user-facing docs uniformly used `code-ownership` (4 references across `advanced-usage.md` and `codebase_analysis.md`), so users typing what the docs said also failed; `code-ownership` is now an alias too. Both share a single regression test in `tests/ownership_test.rs`.

### Internal hygiene

- **`--explain` flag prints the DuckDB optimizer plan** for the analysis's underlying SQL to stderr before running the query. Useful for debugging performance ("which join was the dominator?") and for verifying index use. New `FactsDb::explain_sql(sql, params) -> Result<String>` helper runs `EXPLAIN <sql>` against the connection and returns the plan rows joined by newlines. Initially wired into `run_hotspots` only; deep-analysis follow-up surfaced the gap (20 analyses silently ignored the flag), now closed: shared `analyses::query::explain_if_requested(db, sql, params, label, opts)` helper added to the existing query module, wired across all 21 analyses with a single call before each `prepare(sql)` / `query_map_collect(...)` site. Single-source-of-truth helper means future analyses get `--explain` by adding one line. Doc note in the CLI flag's help text + regression `explain_sql_returns_non_empty_plan` test asserts the plan carries DuckDB operator markers (PROJECTION / JOIN / ORDER).

- **`Options::validate()` rejects pathological flag combinations at the CLI boundary.** Four cross-field invariants used to silently produce empty results: `min_coupling_pct > max_coupling_pct`, `clone_similarity_floor` outside `[0, 1]`, `fisher_significance` outside `[0, 1]`, and `after > before`. `Options::validate(&self) -> Result<()>` now checks all four with descriptive error messages; `codelore-cli::main` calls it immediately after constructing `Options`. The simpler approach was chosen over a full `OptionsBuilder` pattern (which would have forced every callsite to migrate to a new construction path) — one method, zero callsite churn, same coverage. 4 unit tests in `options::tests`.

- **`gix_repo::walk_commits` hoists `.mailmap` parsing out of the per-commit closure.** The mapped iterator step was calling `repo_local.open_mailmap()` on every commit event (re-reading and re-parsing `.mailmap` from disk per commit) AND calling `inner_clone.to_thread_local()` twice per step (once for the diff walk, once redundantly for the mailmap-open). `gix_mailmap::Snapshot` is owned bytes (`Send + Sync`), so it now lives outside the closure and moves cleanly into the mapped iterator; the redundant `to_thread_local()` was removed. On a 10k-commit walk this drops `.mailmap` disk I/O from ~10k reads to 1.

- **`populate_clones_at_head` parallelizes the per-file fingerprint pass via rayon.** HEAD-time complexity extraction was already parallel (`ingest_complexity_at_head` uses `into_par_iter().map_init`), but fingerprint extraction for clone detection stayed sequential — every Tier-1 file was read and fed through tree-sitter on the calling thread. Split into a serial walk phase (cheap `WalkDir` + exclude-globset filter, captures `(absolute path, POSIX rel, lang)`) feeding a `rayon::into_par_iter()` phase that reads + fingerprints in parallel. Fail-fast error semantics preserved via `collect::<Result<Vec<_>>>` (first extract failure short-circuits the pass); unreadable files silent-skip as before. Mirrors the established complexity-pass pattern.

- **`output/csv.rs::quote_if_needed` now triggers on `\r` for RFC 4180 §2.5 compliance.** The hand-rolled quoting helper triggered on `,`, `"`, and `\n` but missed bare carriage return — an author name or commit metadata field carrying `\r` would split a CSV row in two. Added `\r` to the trigger predicate; three unit tests cover the CR / comma+quote-doubling / plain-string paths. Migration to the `csv` crate was considered and explicitly rejected: would regenerate 14+ golden snapshots for zero correctness gain (CSV injection — `=CMD|...` style — is a downstream-Excel concern that neither approach addresses; defense belongs in consumer tooling, not the emitter).

- **Dependency-version drift test** (`crates/codelore-lib/tests/dep_versions_drift_test.rs`) asserts that the hardcoded provenance strings (`gix_version`, `duckdb_version`, `ARROW_RUNTIME_VERSION`) match the corresponding `[[package]]` entries in `Cargo.lock`. `include_str!("../../../Cargo.lock")` + simple string matching — 3 tests, no new deps. Future `cargo update` runs that don't sync the provenance strings now fail in CI rather than shipping desync'd provenance.

- **Per-stage timing via `tracing` spans** — `RUST_LOG=codelore::bench=info` now prints elapsed time for the three load-bearing stages of `analyze` (repo open, cache lookup / ingest, analysis + emit). No new `--bench` flag — the existing `tracing` + `tracing-subscriber` infrastructure (already wired in `init_logging`) carries the timing, which composes with `RUST_LOG=codelore=debug` for per-analysis sub-timings. Default WARN-level filtering suppresses the bench spans entirely (zero overhead for normal runs). Documented in `advanced-usage.md` §11.5.

- **Three narrow typed-error variants** added to `CodeLoreError` at the highest-leverage sites: `MalformedTeamMap { path, line, reason }`, `UnknownAnalysisName { name, supported }`, `BlobNotFound { oid }`. The audit found that only ONE call site (`codelore-cli::main::exit_code`) pattern-matches `CodeLoreError` today, so a wholesale "every failure mode gets a typed variant" migration would be ceremony without payoff. These three are the failure modes where downstream consumers can meaningfully act: surface a "fix `<path>` line N" hint, list accepted analysis names, signal repo corruption distinctly from generic repo errors. `UnknownAnalysisError` (analysis.rs) now `From`-converts to the new variant. Existing `Provenance(String)` / `Repo(String)` / `Analysis(String)` / `Output(String)` / `Io(io::Error)` stay as-is because no consumer differentiates among them today.

- **Default-value constants extracted to `codelore_lib::constants`** as the single source of truth. The pre-extraction state had `min_revs = 5` (and 9 other knobs) literal-encoded in both `Options::default()` (`options.rs:178+`) and `clap` `default_value_t` annotations in `args.rs`; same number in two places, zero compile-time check that they stayed in sync. The post-v0.1.0 audit confirmed they did match, but the silent-drift hazard is real. New module exports 10 `pub const`s — `DEFAULT_MIN_REVS`, `DEFAULT_MIN_SHARED_REVS`, `DEFAULT_MIN_COUPLING_PCT`, `DEFAULT_MAX_COUPLING_PCT`, `DEFAULT_MAX_CHANGESET_SIZE`, `DEFAULT_FISHER_SIGNIFICANCE`, `DEFAULT_MIN_CLONE_NODE_COUNT`, `DEFAULT_MIN_CLONE_SHARED_REVS`, `DEFAULT_CLONE_SIMILARITY_FLOOR`, `DEFAULT_CLONE_SKIP_SAME_DIR` — each with docstring explaining the semantics and the rationale for the chosen value. `Options::default()` and every matching `#[arg(default_value_t = …)]` annotation in `codelore-cli` now reference the constants. Regression test `defaults_use_the_constants_so_drift_is_caught_at_compile_time` asserts the round-trip; future drift compile-errors.

### Docs

- **`--format sqlite` documented as the code-maat `identity` equivalent.** code-maat's `identity` analysis dumps the raw parsed dataset for developer debugging; CodeLore's `--format sqlite` exports a strictly richer fact-store (all 8 tables — `commits`, `changes`, `complexity`, `clones`, `mailmap`, `provenance`, plus identity normalization metadata — vs code-maat's commits-and-changes-only view). Documented in `advanced-usage.md` line 73 so migrating users find the equivalent surface.

## [0.1.0] - 2026-06-08

### 2026-06-08 — Three-sprint deliverable: bugfix + modernization + code-maat parity

**Headline:** 14 → 21 analyses (every published code-maat analysis is now supported), 41 new regression tests (349 → 390), 29 atomic commits.

#### Added — Code-maat parity sprint (PAR-1 through PAR-7)

- **7 new analyses** closing the code-maat feature gap:
  - **`soc` (Sum of Coupling)** — per-entity total of `(commit-size − 1)` across every commit the entity appears in. New `--min-soc N` flag (replaces code-maat's overloaded-`--min-revs` semantic in this one analysis).
  - **`messages`** — commit-message regex matcher driven by `--expression-to-match REGEX` / `-e`. Validated eagerly via the `regex` crate; matching server-side via DuckDB `regexp_matches`.
  - **`main-dev`** — top author per file by lines added. Honest column headers `entity,main-dev,added,total-added,ownership`.
  - **`main-dev-by-revs`** — top author per file by revision count. Honest headers `entity,main-dev,revisions,total-revisions,ownership` (code-maat lied and used `added`/`total-added`).
  - **`main-dev-by-deletions`** (canonical name) / **`refactoring-main-dev`** (alias) — top author by lines removed. Same query, accepted under both names via `FromStr`.
  - **`entity-effort`** — per-(file, author) revision counts with file total alongside.
  - **`entity-ownership`** — per-(file, author) added/deleted churn breakdown.
- **Architectural grouping** via `--group-file FILE` (PAR-7). Plain-text and regex rules in code-maat's `<lhs> => <name>` format. **Full lookaround support** via the `fancy-regex` crate (code-maat's own test fixtures rely on it). First-match-wins, GROUP BY rev + new-path collapses multiple paths in one commit to the same group. Non-strict by default (CodeLore safety improvement: unmapped paths keep raw names; `--strict-grouping` opts back into code-maat's silent-drop behavior).
- **7 wired CLI flags** (PAR-6) — every Options field that had been pre-allocated finally gets a CLI surface: `--min-shared-revs`, `--min-coupling`, `--max-coupling`, `--max-changeset-size`, `--age-time-now YYYY-MM-DD`, `--expression-to-match REGEX` (alias `-e`), `--min-soc N`.
- **3 new Options fields** (PAR-0): `min_soc`, `time_bucket: Option<TimeBucket>`, `code_maat_compat: bool`. All participate automatically in `Options::canonical_json()` so they propagate to both the cache key and the provenance manifest.

#### Added — Modernization sprint (10 of 11; E.3 deferred)

- **Hot-path schema indexes** on `changes(path)`, `changes(rev)`, `commits(canonical_author)`, `commits(date)`.
- **SARIF `CODELORE-MISSING-COCHANGE` rule** — the CodeScene-signature "absent change pattern" signal now reaches Code Scanning (was computed correctly but silently dropped from SARIF).
- **`Options::canonical_json()`** — single helper that serializes the full struct via serde for cache-key + provenance manifest. Eliminates the drift bug where new fields silently disappeared from both surfaces.
- **`BotPatterns` extension hook** via project-level `.codelorebots` file. User additions are purely additive — defaults can never be turned off.
- **AI-attribution patterns extended** to cover 2024-2026 AI coders: Cursor, Aider, Cody, Continue, Codeium, Windsurf, Devin, Tabnine, Amazon Q. Case-insensitive throughout (fixes `Dependabot[Bot]` mixed-case detection bug).
- **Worktree prune on `codelore diff` startup** — SIGKILL / OOM no longer leaves orphan worktrees accumulating.
- **`code-health` centrality uses Fisher-filtered coupling** (eliminates spurious refactor-sweep noise).
- **Diff CLI typed enums** — `--analysis`, `--format`, `--fail-on` upgraded to `clap::ValueEnum`. Typos now caught at parse time.
- **`--absence-min-shared` + `--absence-fisher-p` flags** make coupling-absence thresholds tunable.
- **Hotspot SARIF `level` derives from security-severity bands** (matches the live-clone rule pattern).
- **`change_type` CHECK constraint** on the `changes` table.
- **11 analyses migrated to `params!` bind parameters** — SQL extracted to top-level `const` strings.

#### Fixed — Bugfix sprint (all 7 shipped)

- **`clone_coupling` p-value=0.0 hard-coded** — every live-clone row was shipping a fake p_value since launch. Now carries real Fisher exact p-value.
- **Empty `name` column** in hotspots + code-health output dropped.
- **`author_churn` + `abs_churn` non-deterministic ordering** — tertiary sorts added.
- **Cache key collision** — six clone-detection Options were silently omitted from the cache key (fixed via canonical_json).
- **SARIF coverage gap for coupling absences** — fixed via CODELORE-MISSING-COCHANGE.
- **Outdated AI-attribution patterns** — extended.
- **Worktree leak on SIGKILL** — fixed via startup prune.

#### Changed (BREAKING)

- **CSV header `main-dev` → `main-author`** in `ownership` output (matches struct field). Code-maat-compat surface restoring the legacy header is queued for the `--code-maat-compat` flag.
- **Hotspots + code-health output schema:** dropped the always-empty `name` column. Downstream consumers parsing the CSV verbatim see one fewer column.
- **Manifest JSON schema:** added nested `options: { ... }` field carrying the full canonical Options snapshot. 18 existing flat fields unchanged.
- **CI workflow:** `fetch-depth: 0` set on test job (the `gix_repo_walks_self_repo` test needs ancestry).

#### Docs

- New: `docs/RELEASING.md` (SemVer policy + full release procedure), `docs/github-topics.md` (18-topic recommended set + `gh repo edit` command).
- New plan files in `docs/superpowers/plans/`: `2026-06-08-codelore-bugfix-sprint.md`, `2026-06-08-codelore-modernization-sprint.md`, `2026-06-08-codelore-code-maat-parity.md`.
- README rewritten for clarity (5-minute pitch + 21-analysis table + `--group-file` example + migration story + acknowledgments).
- Cargo.toml: 5 keywords + 2 categories added. README badges expanded from 5 → 13.

#### Added — Code-maat parity completion (PAR-8 + PAR-9, post-wrap-up)

- **PAR-8 `--time-bucket DAY|WEEK|MONTH`** — modern replacement for code-maat's sliding-window `--temporal-period`. Materializes a `changes_bucketed` temp table via `date_trunc(<unit>, commit.date)` that the coupling-family analyses (coupling, clone-coupling indirectly, soc) query when active. Clean non-overlapping buckets — no commit-duplication artifact like code-maat's sliding window had. Implemented via a tiny `format!()` injection of the closed-enum-derived table-name identifier (`"changes"` vs `"changes_bucketed"`); MOD-A1's bind-parameterized threshold values are preserved.
- **PAR-9 `--code-maat-compat`** + **`--strict-grouping`** flags wired on the CLI. `--code-maat-compat` flips three internal defaults: `main-dev-by-revs` CSV emits lying `added`/`total-added` headers; `soc` falls back to `--min-revs` for the threshold; `--strict-grouping` is auto-implied (`strict_grouping: args.strict_grouping || args.code_maat_compat`).

#### Deferred (not deletions — queued for future work)

- **MOD-E3 table-driven dispatch** — refactor the 45-arm `match (format, analysis)` ladder. Current code works + is type-checked; refactor adds marginal value. Revisit if dispatch reaches ~80+ arms.
- **Legacy `--temporal-period N`** under `--code-maat-compat` — the literal sliding-window-with-duplication semantic code-maat shipped. The modern `--time-bucket` covers the actual user need; the `--temporal-period` flag itself rejects as unknown today. Future-work if migration users hit this.

---

### Added (Plan 8 §3 — persistent fact-store cache)
- **`FactsDb::open_or_ingest`** — content-addressed cache wrapper around `FactsDb::ingest`. Cache key is `SHA256(canonical_repo_path || head_sha || codelore_version || options_hash || schema_v1)`. Storage at `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`. Repeat invocations on the same `(repo, HEAD, options)` open the cached DuckDB read-only in ~10 ms — **100×+ speedup** on the dev inner loop.
- **Atomic writes** via `.tmp` → `fsync` → atomic rename (macOS APFS gotcha handled per the research brief).
- **LRU eviction**: 5 entries per repo + 2 GB global cap. Pruning runs after every successful miss-and-write.
- **`Repo::head_sha()` trait method** added; implemented on both `GixRepo` (via `gix::head_id`) and `GitCliRepo` (shell-out to `git rev-parse HEAD`).
- **CLI flags `--no-cache` + `--cache-dir PATH`** — skip the cache entirely (fresh in-memory ingest) or override the XDG cache root (useful in CI with per-job caches on shared runners).
- **Parquet + SQLite formats bypass the cache** by design — both require write access to the DuckDB connection. Documented as a deliberate carve-out.

### Added (Plan 8 §4 — FactsDb clones integration)
- **`clones` table populated during `FactsDb::ingest`** (closes validation report Finding S3). Walks the working tree at HEAD via `walkdir`, fingerprints every function in every Tier-1 file via `clones::extractor`, groups by AST structural digest, and INSERTs one row per family member via DuckDB Appender. Honors `opts.min_clone_node_count` (default 30) and `opts.exclude_patterns` (from `--exclude` + `.codeloreignore`).
- **`IngestStats.clones_ingested`** field added for observability.

### Added (Plan 8 §5 — parallel complexity extraction)
- **`ingest_complexity_at_head` runs via `rayon::par_iter().map_init`** over the working-tree file list. Each Rayon worker independently parses files via tree-sitter; results are collected into a `Vec` and drained serially into the DuckDB Appender on the connection-owning thread (Appender is `!Send + !Sync` per the research brief).
- `tree-sitter::Parser` is `Send + Sync` in 0.25.x — no thread-local pool needed.
- Per-file errors are logged via `tracing::warn!` but do not abort the parallel pass.
- **New bench targets** `complexity_extraction/parallel_default_threads` and `complexity_extraction/serial_1_thread` for measuring the speedup. The serial variant uses a 1-thread `rayon::ThreadPool` per-iteration via `pool.install(|| ...)` (you cannot reset the global pool mid-process).

### Added (Plan 8 §6 — clone-coupling intersection — THE differentiator)
- **`codelore analyze --analysis clone-coupling`** — surfaces "live clones": clone families whose members ALSO co-change at Fisher-significant rates. CodeScene calls this "X-Ray"; we ship the same analytical pattern with our published-formula transparency.
- **Algorithm**: any-pair intersection. JOIN the `clones` table (self-joined on `clone_group_id`, `path_a < path_b`) against `change-coupling` output (Fisher exact `p < 0.05`). Each surviving pair becomes one `CloneCouplingRow` with 18 fields including a `combined_score = similarity × degree_pct × (1 − p_value)` ranking.
- **5 false-positive mitigations** per the SourcererCC research brief, all `Options`-tunable:
  - `min_clone_node_count` (default 30 — drops trivial getters/setters)
  - `min_clone_shared_revs` (default 3 — below this Fisher's exact is unreliable)
  - `clone_similarity_floor` (default 0.70 — SourcererCC BCB benchmark optimum)
  - `--exclude` / `.codeloreignore` (already shipped in §2 Task 8)
  - `clone_skip_same_dir` (default true — drops intentional structural mirroring like `foo_test.rs` ↔ `foo.rs`)
- **Performance**: `O(n·k²)` where n=clone families, k=avg family size (typically ≤ 10). HashMap-based probe table built from coupling results.
- **All 4 output formats** wired (CSV, JSON, Markdown, SARIF).
- **`CODELORE-LIVE-CLONE` SARIF 2.1.0 rule** — one result per `(clone_group_id, file_a, file_b)` pair. `locations[0]` = higher-`support_a` partner (the primary for GitHub Code Scanning inline rendering); `locations[1]` = the lower partner. `partialFingerprints.cloneGroupFingerprint/v1` (AST digest) + `partialFingerprints.filePairHash/v1` (sorted sha256) for stable cross-run identity. `security-severity = combined_score * 10` clamped [0, 10]. Live clones get higher severity than the bare `CODELORE-CLONE` rule because the co-change signal proves real debt, not lookalike noise.

### Added (Plan 8 §7 — `codelore diff` PR-mode subcommand)
- **`codelore diff <base>..<head>`** (two-dot) **and `codelore diff <base>...<head>`** (three-dot, resolves via `git merge-base`). The form users actually deploy in CI.
- **Non-destructive `git worktree`** strategy: each rev checks out into a tempdir under `$XDG_CACHE_HOME/codelore/diff-worktrees/`, analysis runs there, the worktree auto-cleans on Drop via `git worktree remove --force`. The user's working tree is never touched.
- **`--base-cache PATH`** flag serializes the base rev's `RevAnalyses` as JSON. Next PR run with the same base SHA loads from the file instead of recomputing — cuts dual-analysis cost in half for the common case of many PRs against the same base.
- **Per-analysis delta semantics** (CodeScene Delta Analysis-style + research brief a18122f9ec6886ddf):
  - **Hotspots**: `rank_entrants` (new in top-N), `score_increased` (>= threshold), `pr_touched_existing` (info-only)
  - **Coupling**: `coupling_absences` — the CodeScene signature "you should have also changed X" signal. Fires when a historically-strong pair (`shared >= 5 AND fisher_p < 0.05`) has exactly one member in the PR's changed set.
  - **Clones**: `new_families` (introduced by the PR), `pr_touched_existing` (PR modified an existing family member)
- **Four output formats**: `text` (default, human-friendly terminal), `json` (full `DiffOutput` via serde), `markdown` (GFM tables for `$GITHUB_STEP_SUMMARY`), `sarif` (reuses `CODELORE-HOTSPOT` + `CODELORE-CLONE` rules with `properties.codelore/diff-classification` tagging).
- **`--fail-on` quality gate**: `none` (default), `rank-entrant`, `score-increase`, `any`. Exits 4 (analysis-failure per spec §6.6) when condition fires.
- **Example GitHub Actions workflow** at `examples/.github/workflows/codelore-pr.yml` shows the full deployment pattern with the critical gotchas documented (`fetch-depth: 0`, three-dot merge-base, SARIF upload permissions).

### Fixed (Plan 8 §2 follow-up + §6 architectural finding)
- **Mailmap canonicalization for Name+Email entries.** `GixRepo`'s inline mailmap lookup during ingest now passes `event.author_name` to `gix::SignatureRef`, so `.mailmap` entries of the form `Canonical Name <c@x> Original Name <o@x>` resolve correctly. Before: only the email-only form worked, which silently left Alice/Carol-style aliases un-canonicalized.

### Added (Plan 8 §1 — pre-tag hardening)
- **`--analysis` enumeration in error messages.** `UnknownAnalysisError::Display` now lists every supported analysis (12 today) so typos surface a complete menu instead of just "unknown analysis: bogus".
- **`write_clones_csv` snapshot test** locks the 9-column CSV shape against silent header drift.
- **Spec §8 status** updated: clone-coupling row marked `PARTIAL — clone detection ships in Plan 7; intersection lands in Plan 8 §6`.
- **README + perf-evidence-v1.md** corrected for accurate counts and warm/cold timing distinction (the previous 4× "drift" finding was a cold-cache first-run artifact, not a real regression).

### Added (Plan 8 §2 — spec-gap closures)
- **`--analysis authors` standalone.** Closes spec §1.1 gap. CSV header `name,n-commits` matches code-maat; sorted desc by commit count, tiebreak by name. Available across csv/json/markdown formats.
- **`-g` / `--group-file` flag** parsed and forwarded to `Options.group_file`. Aggregation logic deferred to Plan 9; today the flag warns "recognized but no effect yet".
- **`--exclude PATTERN` + `.codeloreignore`** for path-filter. Repeatable flag; the file uses `.gitignore`-style line conventions (blank + `#` comments ignored). Honored by `clones` today; other analyses join in Plan 9. Globset 0.4 dep added.
- **Clones JSON + Markdown emitters.** `--analysis clones --format json` and `--format markdown` now work (CSV was the only output in Plan 7).
- **CODELORE-CLONE SARIF 2.1.0 rule.** New rule for clone-family findings (live-clones in Plan 8 §6 get the higher-severity `CODELORE-LIVE-CLONE` variant). One SARIF result per family with multiple `locations[]` (one per member with line range); `partialFingerprints.cloneGroupFingerprint/v1` keys the family for stable cross-run identity.

### Fixed (Plan 8 §2 follow-up)
- **Mailmap canonicalization for Name+Email entries.** `GixRepo`'s inline mailmap lookup during ingest now passes `event.author_name` to `gix::SignatureRef`, so `.mailmap` entries of the form `Canonical Name <c@x> Original Name <o@x>` resolve correctly. Before: only the email-only form worked, which silently left Alice/Carol-style aliases un-canonicalized.

### Added (Plan 7: Clone Detection — Type 1 + Type 2)
- **`codelore analyze --analysis clones`** — surfaces clone families across the working tree at HEAD.
  - **Algorithm**: AST structural hashing on tree-sitter parses. Pre-order walk emits `(node_kind_id, child_count)` pairs while skipping identifier + literal nodes; SHA-256 over the byte stream is the 256-bit `Fingerprint::digest`. Identical digests = clone family.
  - **Catches**: Type 1 (exact, ignoring whitespace) and Type 2 (renamed/parameterized — names, types, literals normalized away).
  - **Languages**: Rust, Python, Java, JavaScript, TypeScript — same Tier-1 set as the complexity module.
  - **CSV output** (9 columns): `clone-group, fingerprint, entity, function, start-line, end-line, node-count, similarity, family-size`. Similarity is always `1.0000` for T1+T2; Type 3 near-miss (MinHash + LSH at Jaccard < 1.0) is deferred to v1.x.
  - **Validation**: ran against gitoxide @ 10k commits — surfaces ~760 real clone families (URL-parser test pairs, error-formatter duplications, etc.).
  - **HEAD-only**: no git ancestry needed; works on shallow clones, untracked trees, and non-git working dirs (the CLI short-circuits the ingest pipeline for this analysis).
  - **Tuning knob**: `Options::min_clone_node_count` (default 30 ≈ 5-8 statements) drops trivial getters/setters from results.
- **Modules added**: `codelore_lib::clones::{fingerprint, extractor, language}` + `codelore_lib::analyses::clones`. Direct deps added: `tree-sitter 0.25.3`, the 5 Tier-1 grammar crates (exact-pinned to match `codelore-rca`'s ABI), `walkdir 2`.
- **Schema**: `clones` table added to `FactsDb` v1 schema for future ingestion-time storage (clone-coupling intersection lands in v1.x).

### Deferred to v1.x
- **Type 3 near-miss clones** via MinHash + LSH (Plan 7 §2 Task 4) — design + algorithm captured in `docs/superpowers/plans/2026-06-07-codelore-plan-7-clone-detection.md`.
- **`clone-coupling` analysis** — the CodeScene X-Ray pattern: clones × Fisher-significant co-change. The differentiator. Pure SQL JOIN over the existing `clones` + `coupling` tables once T03b (FactsDb integration) lands.
- **SARIF rules `CODELORE-CLONE` + `CODELORE-LIVE-CLONE`** for the GitHub Code Scanning UI path.
- **JSON / Markdown output formats** for clones — currently CSV only.

### Changed (project rename)
- **Project rebranded `bca` → `CodeLore`.** Cargo crates `bca-lib` / `bca-cli` / `bca-rca` renamed to `codelore-lib` / `codelore-cli` / `codelore-rca`; binary `bca` → `codelore`; `BcaError` → `CodeLoreError`; SARIF rule `BCA-HOTSPOT` → `CODELORE-HOTSPOT`; SARIF tool name `bca` → `codelore`; SARIF property keys `bca/*` → `codelore/*`; provenance field `bca_version` → `codelore_version`; markdown headings `# bca <analysis>` → `# CodeLore <analysis>`; spec + plan filenames `2026-06-06-bca-*` → `2026-06-06-codelore-*`. Workspace dir kept as `codescene` to avoid breaking local path bookmarks. No semantic change — only naming.

### Added (Plan 6 in progress: Differential testing + Perf + Release infra)
- **`GitCliRepo`** — shell-out impl of the `Repo` trait, treats C git as ground truth. 11 integration tests cover open, walk, mailmap resolution, `changed_files`, hunk extraction, and `commit_metadata`.
- **`differential_repo` fixture** — 50-commit generated test repo with `.mailmap` (3 alias mappings), 3 authors + 1 bot, 1 rename (`src/old_name.rs` → `src/new_name.rs`), 1 `--no-ff` merge, deterministic per-hour-offset commit dates. **All 8 differential property tests pass**: walk identity (rev-set equality), per-commit field equivalence, `resolve_alias`, `changed_files`, `commit_metadata`, bot-commit visibility, rename visibility, merge-commit count.
- **`GitCliRepo` parser fix** — closed the bug surfaced by the differential test: `parse_git_log_stream` was dropping the commit immediately after a `--no-ff` merge with empty name-status. New `starts_with_pretty_block` helper detects chunks that begin directly with a pretty line (no preceding name-status block); `split_off_name_status_prefix` and `extract_name_status_prefix` fast-path that case. Reproducer added as a `#[cfg(test)]` unit test.
- **`criterion` bench harness** — `crates/codelore-lib/benches/end_to_end.rs` with 3 targets: `ingest_tiny` (5-commit sanity), `ingest/medium_500_commits` (500-commit fixture, CI baseline), `ingest_kernel/linux_kernel_snapshot` (only runs when `CODELORE_BENCH_LINUX_KERNEL_PATH` is set; validates spec §1.1 release blockers). `medium_repo` fixture in `test_support` (3 authors, 25 files, deterministic dates, `gc.auto=0` to avoid loose-blob races during 500-commit init).
- **Weekly CI bench job** at `.github/workflows/bench.yml` — Monday 06:00 UTC, cached cargo + Linux kernel snapshot, `benchmark-action/github-action-benchmark` regression tracking (>10% threshold).
- **`cargo-dist` config** — `[workspace.metadata.dist]` pins cargo-dist 0.28 with 6 release targets (Linux gnu/musl x86_64, Linux gnu aarch64, macOS aarch64+x86_64, Windows MSVC) and 4 installer kinds (shell, powershell, MSI, Homebrew via `emrecdr/homebrew-codelore` tap).
- **`.github/workflows/release.yml`** — multi-platform binary build + SLSA L3 build provenance via `slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml@v2.1.0`. Triggers on tag `v*` push. Per-target archive sha256s are forwarded as SLSA subjects. Aggregates artifacts + `.intoto.jsonl` provenance into the GitHub Release.
- **Distroless container image** — `Containerfile` (3-stage: chef → planner → builder → distroless/cc-debian12:nonroot, ~25-30 MB compressed) + `.github/workflows/container.yml` (linux/amd64 + linux/arm64 via QEMU, SBOM, attested build provenance, GHA layer cache). Image at `ghcr.io/emrecdr/codelore`.
- **PGO scaffolding** — `[profile.release-pgo]` + `scripts/pgo.sh` 3-stage `cargo-pgo` campaign script. v1.0 ships the standard release profile; the PGO campaign is deferred to v1.1 per spec §6.5.

### SLSA provenance rewrite (root-cause fix)

- **Replaced `slsa-framework/slsa-github-generator` reusable workflow with per-matrix-job `actions/attest-build-provenance@v4`.** The reusable workflow aggregates `outputs.hashes` across the build matrix into a single `base64-subjects` input — but GitHub Actions only preserves the LAST matrix job's outputs, silently dropping 4 of 5 subject hashes. (Also, our per-job hash computation emitted only `<hex>` while the reusable workflow expects `sha256sum`-format `<hex>  <filename>\n` lines.) v0.1.0's first publish attempt failed at `provenance / final` (exit code 27 — `SUCCESS=false` because the subject list didn't match the uploaded asset count) and skipped the `release` job that would have attached binaries to the release page.
- **New attestation flow:** each matrix job (`actions/attest-build-provenance@v4`) signs its OWN artifact with the runner's OIDC token, publishes to sigstore's Rekor transparency log, and anchors the attestation to the artifact's content hash. No matrix aggregation, no subjects-format coupling. SLSA v1.0 spec (the reusable workflow used SLSA v0.2). Consumers verify with `gh attestation verify <archive> --owner emrecdr`.
- **Removed the separate `provenance:` job** — attestation is now an inline step in `build` jobs, before the `upload-artifact` step. `release` job's `needs:` list shrinks from `[plan, build, provenance]` to `[plan, build]`.

### Runner version bumps

- **macOS release builds `macos-14` → `macos-15`** in `release.yml`. macos-15 (Sequoia) has been the GitHub-default macOS runner since mid-2025; macos-14 still works but is the prior stable. Both `aarch64-apple-darwin` and the cross-compiled `x86_64-apple-darwin` build on the same `macos-15` host via Apple's bundled cross-target SDK.

### CI action versions + Dependabot

- **Every GitHub Action in `.github/workflows/*.yml` bumped to latest major.** v4 → v6 (`actions/checkout`), v4 → v5 (`actions/cache`), v4 → v7 (`actions/upload-artifact`), v4 → v8 (`actions/download-artifact`), v2 → v4 (`actions/attest-build-provenance`), v3 → v4 (`docker/setup-buildx-action`, `docker/login-action`), v5 → v6 (`docker/metadata-action`), v6 → v7 (`docker/build-push-action`), v2 → v3 (`softprops/action-gh-release`), v0.0.6 → v0.0.10 (`mozilla-actions/sccache-action`). Resolves the "Node.js 20 actions are deprecated" warning we hit on the v0.1.0 release pipeline (Node 20 sunsetting Sep 2026; latest majors all use Node 24).
- **`timeout-minutes:` set on every job** in `release.yml` + `container.yml`. Without these, the GitHub default is 6 hours, which is how we ended up with a queued-for-2.5-hours macos-13 build sitting open during the v0.1.0 debug cycle.
- **`.github/dependabot.yml` added** with weekly grouped PRs for both `github-actions` and `cargo` ecosystems. Single batched PR per ecosystem per week keeps action versions current without per-bump review churn.

### CI hardening (Tier 2 roadmap items shipped)

- **`cargo-nextest`** replaces `cargo test` for unit/integration runs in `ci.yml`. Smarter scheduling, faster process spawning, better failure aggregation; ~20-30% wall-time win on the workspace's test phase. Doc tests still run via `cargo test --doc` since nextest doesn't support them.
- **Path filters** — docs-only pushes (`docs/**`, `**/*.md`, `LICENSE`, `.gitignore`) now skip the full CI matrix. Limited to `push` events so PRs always run CI (avoids the required-check stuck-pending pitfall).
- **`concurrency:` blocks** added to `ci.yml` and `bench.yml` (matching the `release.yml` + `container.yml` pattern). CI cancels superseded runs for the same ref; bench keeps superseded runs so the timeline data isn't dropped.
- **`timeout-minutes:` + `fail-fast: false`** on the test matrix — one OS failing no longer cancels the other two, and stuck jobs fail fast instead of consuming the full 6-hour GitHub default.
- **`sccache --show-stats`** diagnostic step on test + clippy jobs. Gives actual hit-rate numbers per OS per run; needed to design the next round of cache-key fixes for the 0%-hit-rate-on-Windows issue tracked in the roadmap.

### Pre-tag release-pipeline hardening

- **`x86_64-apple-darwin` cross-compiled from `macos-14`** instead of the deprecated `macos-13` runner. GitHub's macos-13 pool has been capacity-constrained since early-2026 (jobs queue for hours or never pick up); macos-14 (aarch64 Sonoma) carries the same Apple cross-SDK and builds the x86_64 target via the existing `--target` flag with no source changes. Intel-Mac binary distribution preserved.
- **Container build split per-arch + manifest merge.** Previously a single `ubuntu-latest` job built `linux/amd64,linux/arm64` together using `setup-qemu-action` to emulate arm64. The emulated build was pathologically slow on Rust workspaces (~2.5 hours when it didn't hang outright) and tripped rustup's toolchain-extract step with `EFAULT (Bad address)` errors. Rewrote `container.yml` as two parallel native-runner jobs (`ubuntu-latest` for amd64, `ubuntu-24.04-arm` for arm64), each pushing by digest only; a final `merge` job assembles the multi-arch manifest via `docker buildx imagetools create` and attests provenance on the manifest digest. No QEMU, ~7 min per arch.
- **`concurrency:` blocks added to both `release.yml` and `container.yml`** keyed on `${{ github.workflow }}-${{ github.ref }}` with `cancel-in-progress: true`. Without this, retags during debugging produced zombie queued runs that competed with the live run for scarce runner slots — observed firsthand during the v0.1.0 push cycle (five tag attempts left three queued release zombies behind that had to be cancelled manually).

### Pre-tag bug-free pass + dep refresh

- **6 verified correctness bugs fixed** before first stable tag:
  - **R1** — Hotspot score formula was producing `[-9, 1]` values because the SQL used `(10 − code_health) / 10` while `code_health` is on the `[0, 100]` scale. Corrected to `(100 − code_health) / 10` so the emitted score range matches the documented `[0, 10]` interpretation; ranking direction unchanged. Spec doc, README, and advanced-usage updated to the new formula.
  - **R12** — `write_hotspots_parquet` was emitting only `entity, revs, cognitive`, silently dropping the `code_health` and `score` columns CSV/JSON/Markdown/SARIF show. Parquet now emits the full 5-column schema using the same SQL as `run_hotspots`.
  - **R2** — `GixRepo::walk_commits` was ignoring `Options.after` / `Options.before` (parameter was prefixed with `_` and unused). Now resolves each commit's author-time date and applies the bounds during OID collection. GitCliRepo already honored these via `git log --after/--before`.
  - **R3** — `--after YYYY-MM-DD` / `--before YYYY-MM-DD` / `--include-merges` CLI flags added to `AnalyzeArgs`. Date flags reuse the `parse_date` helper from `--age-time-now`.
  - **R4** — `GixRepo` now honors `Options.include_merges` (filter at OID-collection time when `parent_count > 1` and the flag is off). Mirrors GitCliRepo's existing `--no-merges` behavior so both backends produce identical event streams. `authors_test::authors_against_differential_fixture` updated from asserting 5 authors (the old leaky GixRepo behavior) to 4 authors (spec-correct default-filtered); new sibling test `authors_against_differential_fixture_with_merges_included` covers the `include_merges = true` path.
  - **R6** — Kamei `enrich_history` and `enrich_experience` rewritten from per-commit correlated subqueries (O(N²) on commit count) to single hash-joined `UPDATE … FROM (subquery)` passes (plus leading zero-out to preserve `COALESCE(…, 0)` semantics for history-less commits). Orders of magnitude faster on large repos; output bit-identical on existing fixtures.

- **Flaky-fixture race fixed**: `differential_repo::build()` was emitting intermittent `error: invalid object … Error building trees` when run alongside other parallel test binaries. Added `git config gc.auto 0` (matches the proven `medium_repo` fix) plus a module-level `BUILD_MUTEX` to serialize concurrent fixture builds across test threads. Each test still gets its own tempdir; only the fixture-build's git-invocation storm is serialized. Stress-tested locally with 10× consecutive parallel runs (10/10 pass after fix; ~1/5 fail rate before).

- **Doc reorganization**: `docs/codebase_analysis_report.md` was mixing two audiences (descriptive architecture + forward-looking improvement backlog). Split into `docs/codebase_analysis.md` (architecture overview — workspace shape, pipeline data flow, threading model, 21-analysis taxonomy, identity layers, Kamei vector, quality posture) and merged the 4 open improvement items into `docs/roadmap-v1.x-and-beyond.md` Tier 3 / Tier 4. Also deleted three dev-process snapshot reports (`deep_analysis_report.md`, `validated_findings_report.md`, `validation-report-2026-06-07.md`) that served earlier sprints but have no role post-`v0.1.0`. Four user-facing references in README + advanced-usage updated.

- **Toolchain + dep refresh to latest stable**:
  - **Rust 1.89.0 → 1.96.0** in `rust-toolchain.toml` and `.github/workflows/ci.yml` (`dtolnay/rust-toolchain@1.96.0` × 3 jobs). MSRV in Cargo.toml workspace metadata bumped to match (was `1.87`).
  - **`sha2` 0.10 → 0.11**, **`fancy-regex` 0.14 → 0.18**, **`dirs` 5 → 6** — all direct deps, API-compatible at usage sites.
  - **`duckdb`** stays at `=1.10503.1` — cargo's resolver confirmed this IS the latest crates.io version (the duckdb-rs project uses an unusual minor-version scheme; `1.5.x`-style versions do not exist as crate releases). The exact pin remains intentional for cache-binary-format determinism.
  - **`tree-sitter` family** stays exact-pinned — deliberate ABI compatibility constraint with the vendored MPL-2.0 `codelore-rca` crate (parser ABI breaks if grammar versions drift).
  - **Two new Rust 1.96 clippy lints addressed**: `clippy::collapsible_match` in vendored `codelore-rca` (crate-level `#![allow]` so upstream code stays unmodified) and `clippy::map_unwrap_or` in `differential_repo_test.rs` (`.map(…).unwrap_or(false)` → `.is_ok_and(…)`).

### Pre-tag stabilization (post-version-bump, pre-`v0.1.0` push)

- **fix(facts): Windows `sync_all` Access-Denied** — `FactsDb`'s cache `.tmp` file was being opened with `std::fs::File::open` (read-only handle), then `sync_all()` was called. On Unix that's a no-op-on-mode `fsync`, but on Windows `sync_all` lowers to `FlushFileBuffers` which requires `GENERIC_WRITE` on the handle and returns `ERROR_ACCESS_DENIED` (os error 5). Surfaced as 17 of 28 Windows CI tests failing across the entire `--analysis` matrix. Fixed by opening the temp file with `OpenOptions::new().read(true).write(true)` so Windows accepts the handle.
- **ci: pin toolchain + drop nightly-only rustfmt options** — `rustfmt.toml` had `imports_granularity = "Module"` and `group_imports = "StdExternalCrate"` (both nightly-only). Stable rustfmt warned-and-ignored them, which let formatting drift accumulate across 17 source files. Dropped both options, ran `cargo fmt --all` to normalize the workspace, and pinned `.github/workflows/ci.yml` to `dtolnay/rust-toolchain@1.89.0` as defense-in-depth.
- **chore: resolve `<owner>` placeholders to `emrecdr`** — `Cargo.toml` `repository` field + `cargo-dist` Homebrew tap + `ghcr.io` image references in CHANGELOG, RELEASING.md, and the validation report.

### Deferred (out of scope for `v0.1.0`)
- **Code-maat golden parity tests** — requires Leiningen/Clojure runtime to invoke code-maat against the fixture repos. Owner runs the capture script and commits the resulting `fixtures/golden/code-maat/*.csv` files. Tracked for `0.1.x` patch.
- **`v1` performance evidence** — requires a Linux kernel snapshot to run the kernel bench and capture wall-clock + peak RSS into `docs/perf-evidence-v1.md` validating the spec §1.1 release blockers (<10 min, <4 GB). Tracked for `0.1.x` patch.
- **Legacy `--temporal-period N` under `--code-maat-compat`** — code-maat's sliding-window-with-duplication semantic. The modern `--time-bucket` covers the actual user need. Revisit only if migration users hit this.

### Added (Plan 5: Output formats + Provenance)
- **5 new output formats** alongside CSV (all 11 analyses unless noted):
  - `--format json` — structured JSON (pretty-printed, serde-derived)
  - `--format sarif` — **SARIF 2.1.0 with `CODELORE-HOTSPOT` rule** (hotspots only — the published Behavioral SARIF differentiator). Security severity proxy: `(100 − code_health) / 10`; stable `partialFingerprints` via `sha256(repo_root|path)`
  - `--format markdown` — GitHub-Flavored Markdown pipe tables, designed for `$GITHUB_STEP_SUMMARY`
  - `--format parquet --output FILE` — DuckDB-native `COPY ... TO ... (FORMAT PARQUET)`. Plan 5 ships hotspots, revisions, summary; binary format requires `--output`
  - `--format sqlite --output FILE` — full 7-table fact-store dump via `ATTACH '...' (TYPE SQLITE)`. Provenance table is included inside the DB; no sidecar needed. Requires explicit `INSTALL sqlite; LOAD sqlite;` (DuckDB bundled build doesn't auto-load)
- **Provenance manifest sidecar** (`codelore_lib::provenance::Manifest`) — 18-field reproducibility receipt:
  - `codelore_version`, `gix_version`, `arrow_version`, `duckdb_version`, `run_started_at` (UTC RFC3339)
  - `repo_path`, `analysis`, `after_date`, `before_date`, `age_time_now`, `merge_handling`, `include_merges`
  - All threshold knobs: `min_revs`, `min_shared_revs`, `min_coupling_pct`, `max_changeset_size`, `fisher_significance`, `complexity_sample`
  - Emitted as `{output}.provenance.json` next to every file output **except** `--format sqlite` (where the provenance table lives inside the DB) and stdout output (where no path exists). Addresses Spadoni 2025's 500% inter-tool disagreement problem
- CLI: `codelore analyze --format {csv | json | sarif | markdown | parquet | sqlite}` — 2-level (format × analysis) dispatch with validated constraints:
  - `parquet`/`sqlite` require `--output PATH` (binary; can't stream stdout)
  - `sarif` requires `--analysis hotspots` (Plan 5 scope; coupling SARIF lands in Plan 6)

### Added (Plan 4: Analyses + Identity + Kamei + Code Health completion)
- **Identity resolution**:
  - `.mailmap` lookup via gix mailmap API in `GixRepo::resolve_alias`
  - `bots.toml` default-deny bot list (dependabot, github-actions, copilot, claude-code, renovate, pre-commit-ci)
  - AI attribution stub (`ai-authored` / `ai-assisted` / `human`) on every commit
- **Kamei 14-feature change vector** (Kamei et al. JIT-SDP canonical) populated via SQL UPDATE pass after ingest:
  - Diffusion: NS, ND, NF, entropy
  - Size: LA, LD, LT (LT stubbed to 0 — Plan 5 may improve)
  - Purpose: FIX (regex on bug/fix/defect keywords)
  - History: NDEV, AGE, NUC
  - Experience: EXP, REXP, SEXP
- **8 new analyses**:
  - `code-age` — months since last modification per file (spec §1.1)
  - `abs-churn` — date-grouped lines added/deleted/commits
  - `author-churn` — canonical-author-grouped churn (uses .mailmap resolution)
  - `entity-churn` — file-grouped churn
  - `communication` — Conway's law author-pair shared-work + strength
  - `code-ownership` — Fractal Value (1−HHI complement) + main developer per file
  - `change-coupling` — per spec §3.2.1 correctness invariants (max-changeset-size pre-filter, mirrored pair dedup, Fisher exact significance at p<0.05 default)
  - `summary` — 4-row repo overview (commits/changes/entities/authors)
- **Code Health composite** now uses all 4 inputs from spec §4.6 (cognitive 0.40 + churn 0.25 + fragmentation 0.15 + coupling 0.20). Verified: src/main.rs (4 commits) now ranks lower than src/lib.rs (1 commit) in Code Health.
- CLI: `codelore analyze --analysis NAME --format csv` works for all 11 analyses.
- `--complexity-sample {head|adaptive|full}` flag (Plan 4 ships head only; adaptive/full land in Plan 5)

### Added (Plan 3: Complexity Integration + Hotspots + Code Health)
- `codelore-lib::complexity` module — wraps `codelore-rca` for Tier-1 languages (Rust, TS/JS, Python, Java)
- Path-based language dispatch (`Tier1Language::from_path`) maps file extensions to codelore-rca parsers
- Function-level entity extraction via `codelore-rca::FuncSpace` traversal (file + function + class scopes)
- `FactsDb::ingest()` now populates `entities` and `complexity_metrics` tables at HEAD by reading working-tree files
- `hotspots` analysis (`codelore_lib::analyses::hotspots::run_hotspots`) per spec §1.1 published formula:
  `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (10 − code_health) / 10`
- `code-health` composite analysis (`codelore_lib::analyses::code_health::run_code_health`) per spec §4.6
  - Plan 3 wires cognitive input only; churn/fragmentation/coupling inputs land in Plan 4
  - Reduced formula: `100 × (1 − 0.40 × normalize(cognitive))`
  - Range: [0, 100], higher = healthier
- CLI: `codelore analyze --analysis hotspots --format csv` and `codelore analyze --analysis code-health --format csv`
- New CSV emitters: `write_hotspots_csv`, `write_code_health_csv` with shared `quote_if_needed` helper

### Added (Plan 2: RCA Vendor)
- `crates/codelore-rca/` — vendored fork of mozilla/rust-code-analysis
  - SPDX: `MPL-2.0 AND GPL-3.0-only`
  - Dropped `-web`, mozjs grammar, ABC/WMC/NPA/NPM impls
  - Mozjs fully excised (Option B from UPSTREAM.md); standard `tree-sitter-javascript` covers everything we need
  - Mozcpp retained (language_cpp.rs is generated from mozcpp grammar; standard tree-sitter-cpp would silently break C++ metrics)
  - Per-language tree-sitter grammars exact-pinned for ABI compatibility with our generated `language_*.rs` enums
  - `metrics-experimental` feature flag for JS/TS Halstead+MI (RCA bugs #528 #1183)
  - 199 upstream RCA unit tests preserved and passing
  - 5 Tier-1 language smoke tests (Rust/Python/Java/TS/JS) + 1 conditional for metrics-experimental

### Fixed (Plan 1 carry-over)
- `CodeLoreError::exit_code()` now wired into `codelore` CLI per spec §6.6 (Plan 1 always exited 1)
- `FactsDb::query_one_value` gated behind `test-support` feature (no longer in production builds)
- `gix_repo.rs` "Plan 11" comment typo → "Plan 4"
- Added file-backed `FactsDb::open()` roundtrip test (was untested in Plan 1)

### Added (Plan 1: Phase 0 + Walking Skeleton)
- 3-crate Cargo workspace (`codelore-lib`, `codelore-cli`, future `codelore-rca`)
- Core types: `CommitEvent`, `FileChange`, `Hunk`, `ChangeType`, `KameiFeatures`
- `AnalysisName` enum and `Options` struct with code-maat parity defaults
- `arrow_facade` module — single re-export point for `arrow-rs`
- `Repo` trait + `GixRepo` impl (read .git via gix 0.84)
- Fixture builder (`test_support::tiny_repo`) for reproducible 5-commit test repos
- `FactsDb` — DuckDB-backed fact store with v1 schema (7 tables)
- Commit ingestion pipeline (gix → crossbeam channel → DuckDB Appender)
- `revisions` analysis (SQL view + Rust orchestrator)
- CSV output emitter (code-maat header parity)
- `codelore analyze --analysis revisions --format csv` CLI
- GitHub Actions CI (fmt, clippy, test on 3 OSes, cargo-deny)
- Justfile, deny.toml, renovate.json, rust-toolchain.toml

### Pending (subsequent plans)
- Plan 2: RCA vendor (Mozilla rust-code-analysis fork) + Go support ✅
- Plan 3: complexity integration + hotspots + Code Health composite ✅
- Plan 4: 8 new analyses + Kamei vector + identity resolution + full Code Health composite ✅
- Plan 5: SARIF + Markdown + Parquet + SQLite + provenance manifest ✅
- Plan 6: differential testing harness + perf benchmarks + release infra
