# CodeLore — Deep Codebase Analysis Report

Read-only audit log. Findings are immutable F-IDs; status field tracks state.
Shipped findings are pruned from this report once their release ships (full history in `CHANGELOG.md`); refuted findings stay documented to prevent rediscovery.

---

## 1. Architectural Overview & Pipeline Data Flow

CodeLore is a multi-crate Rust workspace:
*   [codelore-rca](file:///Users/emrec/Projects/playground/codelore/crates/codelore-rca): Vendored fork of Mozilla `rust-code-analysis` — structural syntax hashing + complexity metrics.
*   [codelore-lib](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib): Core engine — repository walk abstraction, identity resolution, fact-store management, analyses, caching, output emitters.
*   [codelore-cli](file:///Users/emrec/Projects/playground/codelore/crates/codelore-cli): Argument parsing, option consolidation, output routing.

### Data Ingest Flow

```mermaid
graph TD
    A[GixRepo / GitCliRepo] -->|walk_commits → CommitEvent stream| B[Bounded crossbeam channel]
    B -->|producer → consumer| C[FactsDb ingest]
    C -->|DuckDB Appender bulk-insert| D[(DuckDB fact store)]
    E[HEAD-time blob walk @ HEAD] -->|tree-sitter parsing via rayon| F[Complexity + clones extraction]
    F -->|HEAD-time metrics| D
    D -->|SQL views / parameterized queries| G[23 behavioral analyses]
    G -->|emitters| H[CSV · JSON · SARIF 2.1.0 · Markdown · Parquet · SQLite · HTML · SPA]
```

1.  **Repository Traversal**:
    *   [GixRepo](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/repo/gix_repo.rs) — pure-Rust `gitoxide`. Hot path.
    *   [GitCliRepo](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/repo/git_cli_repo.rs) — shells out to `git`; differential-testing oracle.
2.  **Event Ingestion**: `duckdb::Connection` is `!Send + !Sync`. Producer-consumer: background thread walks commits → bounded `crossbeam-channel` → connection-owning thread runs DuckDB Appender ([ingest_loop](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/facts/ingest.rs)).
3.  **HEAD-time work**: [ingest_complexity_at_head](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/facts/ingest.rs) + [populate_clones_at_head](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/facts/ingest.rs) read blobs from the gix ODB (works on bare repos), parse via tree-sitter on a rayon pool, drain serially into the DuckDB Appender.
4.  **SQL-Driven Analyses**: 23 behavioral analyses run as parameterised DuckDB queries (e.g. [hotspots.rs](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/analyses/hotspots.rs), [coupling.rs](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/analyses/coupling.rs)). Path-aggregating analyses opt into rename-aware aggregation via the `changes_lineage` CTE rewriter.

---

## 2. Historical Findings (F1–F87) — Shipped

All prior findings (F1–F87) have shipped and were validated against `main` HEAD. Per-finding evidence is preserved in `CHANGELOG.md`. Audit-trail summary:

| Batch | Scope | Outcome |
|---|---|---|
| **F1–F17** (v0.2.x) | Schema timestamps, chunked walker, rename-aware aggregation, CLI-boundary validation | Shipped |
| **PAR-1–PAR-9, DEEP-1–DEEP-15** (v0.2.x) | Code-maat parity sweep | Shipped |
| **F18–F28** (v0.3.x) | Back-test isolation, HTML pagination, cache-write concurrency, parallel filtering | Shipped |
| **F29–F34** (v0.3.2) | Time-bucket changeset semantics, path-relative skip checks, binary diff guards | Shipped |
| **F35–F42** (v0.3.3 → v0.4.0) | Numstat brace expansion, explain-mode params, quadratic Kamei rewrite | Shipped |
| **F43–F56** (v0.4.1 → v0.4.2) | Blob clone elision, single-pass templating, COUNT(DISTINCT) elimination, SPA X-Ray join | Shipped |
| **F57–F67** (v0.4.4) | ECharts theme reload, prefix matching, ODB blob reads, hash aggregation sweep, SIMD line counting | Shipped |
| **F68–F76** (v0.4.6) | AI attribution rollup, lockstep rev equality, lineage rename-index, NULL-safe distinct elimination | Shipped (F69/F70 bench-gated closed) |
| **F77–F87** (v0.5.0) | Bare-repo clone discovery, theme-controller migration, multi-column SPA grid, cognitive-color sunburst, JSX/TSX grammar coverage | Shipped |
| **F84, F88** | Refuted at source-quote level (recycled-path lineage, silent ODB-skip rationale) | Refuted |

Refuted-finding rationale stays in `git log` against the validation PR (commit 2f8a7bc, PR #20) so the next audit cycle doesn't rediscover them.

---

## 3. New Audit Cycle (2026-06-14, post-v0.5.0)

Validation methodology: five parallel read-only sub-agents covered (1) ingest & threading, (2) SQL analyses, (3) SPA frontend, (4) Rust deps & idioms, (5) CLI & output emitters. Their raw findings were adversarially verified against `main` source — claims that traced to a real defect become Active F-findings below; refuted claims are listed at the end so the next pass doesn't rediscover them.

**Score**: 16 raw HIGH/MED candidates verified → **10 Active** findings (F89–F98) + **6 Refuted** (logic-correct on closer reading) + **3 Improvement** (non-bug architecture follow-ups, V4–V6).

### Active Findings

#### F89 — Producer-thread panic surfaces as main-thread panic

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs:78` — `producer.join().expect("producer panicked")?;`
*   **Severity**: MED
*   **Category**: Threading correctness / error UX
*   **Status**: Active
*   **Description**: The ingest scope spawns the producer thread (commit walker on a rayon pool) and waits for it via `.join().expect(...)?`. The `.expect()` form turns a producer-thread panic into a *main-thread panic*, which bypasses `CodeLoreError`'s typed-error mapping and lands as exit-code-101 garbage instead of a typed `CodeLoreError::Repo`. Consumer-side errors already round-trip as `Result`; the producer should match.
*   **Reproduce**: induce a panic inside `walk_commits` (e.g. an `unreachable!()` for a malformed pack), run `codelore analyze`. Observe `thread 'main' panicked at 'producer panicked'` instead of `Error: repo: ...`.
*   **Suggested fix**: replace `.expect("producer panicked")` with `.map_err(|panic| CodeLoreError::Repo(format!("commit walker thread panicked: {:?}", panic)))?` so the typed-error chain stays intact and `main()`'s exit-code switch returns 3 (Repo).
*   **Cross-ref**: maps to the `workspace.lints.rust: unsafe_code = "forbid"` philosophy — typed-error fidelity is a CodeLore invariant.

#### F90 — SPA X-Ray sunburst container rings use hardcoded colors (no theme adaptation)

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:1213-1215`
*   **Severity**: MED
*   **Category**: UI / theming
*   **Status**: Active
*   **Description**: After F79's DaisyUI theme-controller migration shipped (light + dark themes that swap on `prefers-color-scheme`), the X-Ray sunburst's container rings still use hex-baked colors (`#1f3f29`, `#2c5d3a`, label `#1a1a1a`). On the light theme the dark-green rings + white text are fine; on the dark theme the rings are visually flat against the elevated background. Worse, leaf labels (`#1a1a1a`) sit on the cognitive-complexity heatmap (yellow → red) — near-black labels on saturated red drop below WCAG AA contrast (~3:1).
*   **Reproduce**: render the SPA, switch to dark theme via the DaisyUI swap toggle, scroll to X-Ray sunburst, observe the container rings vanish into the navy background.
*   **Suggested fix**: read CSS vars (`var(--codelore-ring-1)`, etc.) into `levels[*].itemStyle.color` at render time — the file already has a `getCssVar` helper used by hotspot circle-pack. Same trick: define two ring tokens per theme in `tailwind-src/input.css` and let DaisyUI's `:has(.theme-controller[value=dark]:checked)` cascade them. Sunburst re-renders on theme toggle anyway (F57 listener), so the lookup happens once per swap.

#### F91 — Markdown emitter doesn't escape `|` in table cells

*   **Location**: `crates/codelore-lib/src/output/markdown.rs` — every `writeln!(w, "| {path} | ...")` call site (lines 31, 68, 89, 105, 120, 135, 148, etc.)
*   **Severity**: MED
*   **Category**: Output correctness
*   **Status**: Active
*   **Description**: Markdown table cells are written with the literal interpolation `| {value} |` — no escaping. Per GFM spec, a `|` *inside* a cell must be backslash-escaped (`\|`) or the table row's column count breaks (renderers split the row at the unescaped pipe). Paths are unlikely to contain `|` but legal on every filesystem CodeLore supports; author names, commit messages, and entity names absolutely can. Worst case: a single `|` in any cell silently corrupts the entire downstream table.
*   **Reproduce**: ingest a repo where any file is named `foo|bar.rs` (legal); run `codelore --format markdown hotspots`. Observe the table shape breaks for the affected row.
*   **Suggested fix**: extract a `fn escape_md_cell(s: &str) -> Cow<'_, str>` helper (Cow because most cells need no escape — keeps the common-case allocation-free), apply at every cell-write site. Mirror the `quote_if_needed` pattern in `csv.rs`. Add a regression test with a path containing `|` and `\n` to `tests/output_markdown_test.rs`.

#### F92 — Provenance sidecar atomicity gap

*   **Location**: `crates/codelore-cli/src/main.rs:1006-1009` (streaming formats) and `:275` (parquet)
*   **Severity**: MED
*   **Category**: Output atomicity / observability
*   **Status**: Active
*   **Description**: The main output's `BufWriter<File>` is dropped on line 1006 (flushing OS buffers), then `write_provenance_sidecar(&db, &opts, analysis_name, path)?` runs on line 1009. A crash, OOM kill, or disk-full event between those lines leaves the main output committed on disk but the `.provenance.json` sidecar missing — downstream tools that gate on provenance (SLSA attestation, CI gates) see corrupted state. The same window exists for parquet at line 275 (sidecar after the COPY). Neither path calls `sync_all()` on the main handle either, so even a power-loss in the same window can leave the main output truncated.
*   **Suggested fix**: write the sidecar to a `.tmp.<pid>` path first, fsync the main output, then `rename` the sidecar atomically. Alternatively, document explicitly in `docs/advanced-usage.md` §provenance that "absent sidecar means abort during emit; retry" so downstream consumers can implement graceful degradation. The atomic-rename approach mirrors the cache write strategy F23 already uses.
*   **Note**: SQLite output (line 282) is correctly exempt — provenance lives inside the same `.sqlite` file.

#### F93 — `cache_key` silent canonicalize fallback can cause cache miss

*   **Location**: `crates/codelore-lib/src/cache.rs:32` and `:67` (the latter is the F33 fix's cousin)
*   **Severity**: LOW (but symmetric with F33)
*   **Category**: Cache invariant robustness
*   **Status**: Active
*   **Description**: Both `cache_key` (line 32) and `cache_path_with_root` (line 67) use `fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf())`. The fallback path is functionally correct **as long as both call sites fail consistently** — the F33 fix specifically pairs them. But if a repo path becomes canonicalizable mid-run (e.g., a symlink target is created or a permission flips), the next invocation's key drifts and the cache misses. There is no `tracing::warn!` to surface the fallback.
*   **Suggested fix**: emit a `tracing::debug!("canonicalize fallback for repo_path={}", repo_path.display())` inside the `unwrap_or_else` closure so operators on dirty containers or shared mounts can spot silent cache misses. No semantic change.

#### F94 — `ingest.rs` is 1080 lines, three logical layers fused

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs` (1080 lines)
*   **Severity**: MED
*   **Category**: Maintainability / cognitive complexity
*   **Status**: Active
*   **Description**: `ingest.rs` mixes (1) the producer/consumer `ingest_loop` (the canonical channel-around-`!Send` pattern), (2) `ingest_complexity_at_head` (rayon-then-serial-drain HEAD scan), (3) `populate_clones_at_head` (clone fingerprinting), (4) `materialize_path_lineage` (recursive CTE construction), and (5) `apply_grouping` (group-file post-pass). Each is independently complex; future work on one (e.g. the v0.5.x serve mode wanting an incremental ingest) lands in a 1000-line file. Reading it requires holding 5 mental models. Splitting follows the CLAUDE.md "before touching specific areas, read" lookup-table partition exactly.
*   **Suggested fix**: split into `ingest/loop.rs` (producer/consumer + `ingest_loop`), `ingest/complexity.rs` (HEAD-time complexity), `ingest/clones_head.rs` (HEAD-time clones), `ingest/lineage.rs` (`materialize_path_lineage`), `ingest/grouping.rs` (`apply_grouping`). Re-export public surface from `facts/ingest.rs` to keep API stability. Pure code organisation — no semantic change.

#### F95 — `communication.rs::author_files` has no `--since` / `--until` filter

*   **Location**: `crates/codelore-lib/src/analyses/communication.rs:60-66`
*   **Severity**: LOW
*   **Category**: Time-window semantics
*   **Status**: Active
*   **Description**: Most CodeLore analyses respect `--since` and `--until` boundary filters via WHERE clauses on `commits.date`. `communication`'s `author_files` CTE has no such filter — it's an unbounded `SELECT DISTINCT path, author FROM commits INNER JOIN changes`. If a user runs `codelore communication --since=2024-01-01`, the rest of the pipeline filters the time-window but the per-pair shared-file count silently includes pre-window history. (Knowledge-Islands does anchor; communication does not.)
*   **Suggested fix**: if `opts.since` / `opts.until` are set, add the boundary `WHERE commits.date BETWEEN ? AND ?` inside `author_files` and `totals` CTEs, with the param binds threaded through to `prepare`. Adds two param slots. Regression test: a fixture spanning a year, communication with `--since` halfway through; the shared-file count must drop for pairs whose co-edits were pre-window.
*   **Verify-then-fix**: confirm `communication` is expected to honor `--since` / `--until` (the codebase has `since/until` flags but several analyses are documented as full-history). If full-history is by design, downgrade to docs-only.

#### F96 — ECharts mount + dispose pattern duplicated 5× across widgets

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:250-251, 787-788, 1021-1022, 1095-1096, 1185-1186` (each pair followed by `bindChartResize`)
*   **Severity**: LOW
*   **Category**: Code quality / drift risk
*   **Status**: Active
*   **Description**: Five widgets (hotspot circle-pack, sankey, trends, calendar, sunburst) repeat the exact triplet `const prior = echarts.getInstanceByDom(container); if (prior) prior.dispose(); ... bindChartResize(chart, container);`. F64 fixed dispose drift; the next widget added will copy-paste the same lines and the next bug fix will need to touch all five sites. F71 already extracted `bindChartResize`; the dispose-then-init prelude should follow.
*   **Suggested fix**: extract a `mountEcharts(container, option, themeAware = true) -> chart` helper that owns the dispose-then-init-then-resize-bind triplet. Each widget becomes `const chart = mountEcharts(container, opt); chart.on('click', …);`. Net change: one helper, five widget call-sites simplify. Pure refactor.

#### F97 — SPA `JSON.parse` of embedded payload blocks first paint on large repos

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:23` (`JSON.parse(dataBlock.textContent)`) + the inline `<script type="application/json">` embed in `spa.rs::render_spa`
*   **Severity**: MED
*   **Category**: Initial render perf
*   **Status**: Active
*   **Description**: The SPA emits a single self-contained HTML — the dashboard data ships inline as a `<script type="application/json">…</script>` block. On real-world repos (the codelore-self dashboard, ~300 hotspots + ~500 X-Ray entities + cell rollups) the JSON payload is in the 1-2 MB range. `JSON.parse` runs synchronously on the main thread before any widget renders, then every widget's render also runs sync. On low-end laptops this is a hard ~200-500 ms freeze at first paint — visible to users.
*   **Suggested fix** (two options, ranked):
    1.  **Streaming parse + idle-callback render**: split the JSON block into a header `{...metadata}` + per-widget `<script type="application/json" id="widget-X">`. Parse + render each widget in `requestIdleCallback` chain — first paint shows skeletons; widgets fill in progressively. ~half day.
    2.  **gzip the HTML at emit time**: not a runtime fix but most servers re-gzip already; the file size complaint is largely a download-time problem.
*   The SPA's "single-file portability" brand is the constraint that ruled out external-file fetches — the idle-callback chain preserves it.

#### F98 — Chart-click → drawer has no keyboard equivalent (a11y)

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:362-367, 814-818, 1220-1225` — every `chart.on('click', ...)` that opens `showFileDetailDrawer`
*   **Severity**: LOW
*   **Category**: Accessibility
*   **Status**: Active
*   **Description**: The detail drawer is reachable only by mouse-clicking a circle-pack node, sankey link, or sunburst leaf. Keyboard users have no path to the drawer — ECharts custom-series shapes don't expose to the accessibility tree. The hotspot *table* row-click does open the drawer and the table is keyboardable, so a workaround exists, but the chart-first widgets are the visual headline.
*   **Suggested fix**: add a sibling "View files" toggle button next to each chart's heading that opens a focusable list-view of the same data (DaisyUI `<select>` or `<menu>`). Each list item triggers `showFileDetailDrawer(path)`. Mark up the chart container with `aria-describedby` pointing at a visually-hidden text alternative ("Interactive map of 287 hotspot files; use the list below for keyboard access"). Costs one widget pattern, applied four times.

### Improvement Opportunities (non-bug, architectural)

#### V4 — `widgets.js` is 1365 lines of monolithic IIFE

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js`
*   **Category**: SPA maintainability
*   **Status**: Improvement
*   **Description**: All 14 widgets + theme system + drawer + helpers live in one IIFE closure. The file already gates on Tailwind v4's `@source` scanner, which precludes a true ES-module split without retooling the build. But the IIFE can be sectioned into per-widget sub-closures (`function renderHotspotCirclePack(...)`, `function renderXraySunburst(...)`, etc.) with a top-level "boot all widgets" function — same single file, far better local-edit cognitive load. The F96 `mountEcharts` extract is the natural seed for this work.
*   **Effort**: ~half day; depends on shipping F96 first.

#### V5 — Fisher significance / coupling-strength thresholds shown in SPA tooltips

*   **Status**: Improvement
*   **Description**: The `--fisher-significance` / `--min-shared-revs` thresholds are already CLI flags (verified at `options.rs:65, 305-308`), but the SPA dashboard doesn't surface which thresholds the rendered numbers were computed against. A reader looking at the coupling widget can't tell if `min_shared_revs=5` (default) or `min_shared_revs=20` produced the numbers. UI-2's "?-tooltip" plan (per `docs/roadmap-v1.x-and-beyond.md`) is the natural home — the provenance manifest already carries the effective option values, so wiring is pure SPA-side.
*   **Effort**: ~1 day, bundled with UI-2.

#### V6 — Producer/consumer channel capacity (`CHANNEL_CAPACITY = 64`) is unmeasured

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs:56`
*   **Category**: Performance scaling
*   **Status**: Improvement
*   **Description**: The 64-event bounded channel is a defensible default but has not been benchmarked. On very large repos (linux-kernel-scale, ~1M+ commits) backpressure could either underfeed the Appender (consumer-idle gaps) or overflow into producer-block stalls. A microbench inside `benches/` measuring throughput at 16 / 64 / 256 / 1024 capacities would either validate the default or motivate a CLI flag for tuning.
*   **Effort**: ~half day, including a small synthetic-history fixture builder.

### Refuted in This Cycle

For audit-trail completeness, raw findings that survived first-pass triage but fell to source-quote verification:

| Claim | Refutation |
|---|---|
| `apply_grouping` hunks DELETE uses `g.group_name = c.path` — should be `= h.path` | Refuted. `c.path = h.path` is already in the JOIN; `g.group_name = c.path` is the additional filter that distinguishes "identity-mapped" paths (kept) from "rewritten-to-group-name" paths (dropped, because hunks track line ranges against the original path). The comment at line 1031-1035 documents the intent: "Hunks aren't path-rewritten ... so they get dropped for any path that collapsed". Verified across strict-mode + non-strict-mode + identity-mapped + collapsed-mapped path scenarios. |
| `renderHeader` accumulates click listeners on every re-render | Refuted. Line 450 sets `container.innerHTML = html` *before* line 453 queries the new `<th>` elements — the old `<th>` elements are detached from the DOM (taking their listeners with them) before new ones are bound. Safe-by-construction. |
| Parquet/SQLite path strings lack backslash escape → SQL injection / path mangling | Refuted. DuckDB's standard single-quoted string parser doesn't honor `\` as an escape sequence (that's the `e'...'` prefix syntax); literal backslashes in paths pass through cleanly. The `.replace('\'', "''")` is the complete escape. Windows paths like `C:\Users\...\out.parquet` work as-is. |
| `hotspots.rs::file_complexity` / `code_health.rs::file_cognitive` CTEs miss `file_revs` join → sub-threshold files leak | Refuted. The outer `joined` CTE does `FROM file_revs fr LEFT JOIN file_complexity fc ON fc.path = fr.path` — the LEFT JOIN drives from the *filtered* `file_revs`, so sub-`min_revs` files never produce output rows. The unjoined complexity rows for sub-threshold paths exist in the CTE but are dead weight, not contamination. |
| `--fisher-significance` not exposed as a CLI flag | Refuted. `Options::fisher_significance: f64` is at `options.rs:65`, validated at `:305-308` (`must be in [0.0, 1.0]`), defaulted at `:336` (`DEFAULT_FISHER_SIGNIFICANCE`), and exposed as the CLI flag `--fisher-significance`. |
| SPA hotspot-color-mode toggle buttons missing `aria-label` (a11y) | Refuted. The buttons are `<button type="button">Complexity</button>` etc. — the text content *is* the accessible name. Screen readers announce them correctly. (The genuine a11y gap is chart-click navigation, captured as F98.) |

---

## 4. Next Audit Cycle

When this report's `Active` count reaches zero again, the next read-only sweep can re-open with F-IDs starting at F104 (preserving 89–98 plus V4–V6 in audit-trail). The validation methodology (parallel sub-agents → adversarial verification → strict source-quote requirement before promoting to `Active`) is the load-bearing discipline — without it, the 6 refuted findings would have shipped as work and burnt cycles.
