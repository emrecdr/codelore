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
| **F107, F108** (v0.5.1 hotfix) | SPA runtime errors caught in production after v0.5.0 ship: METRIC_DEFS Temporal Dead Zone in widgets.js IIFE (F107) + Alpine inline-script order causing `$store.*` undefined at first paint (F108). Both shipped through every SPA-touching PR since v0.4.x. Caught by user browser console, not by CI. PR #37. | Shipped |
| **F109** (post-F91 sweep gap) | `codelore-cli/src/diff_output.rs` had 4 cell-emit sites (`writeln!(out, "\| `{}` \| ...")` for hotspots/coupling/clones rows) that F91's `escape_md_cell` fix in PR #48 did not sweep — F91 only touched `output/markdown.rs`. §4's Reaffirmed section explicitly flagged this gap; missed during F91 implementation. Fixed by promoting `escape_md_cell` to `pub` in codelore-lib::output::markdown and wrapping all four diff-PR-mode cell-emit sites. | Shipped |

Refuted-finding rationale stays in `git log` against the validation PR (commit 2f8a7bc, PR #20) so the next audit cycle doesn't rediscover them.

**Methodology note (F107 / F108 post-mortem)**: both the F89–F98 audit cycle (§3) and the F99–F106 second pass (§4) ran read-only sub-agents over the source tree using static-grep + inspection. Neither surfaced F107 + F108 because both are *runtime* initialization-order defects — the bugs only manifest when the JS actually executes in a browser. The existing `spa_integration_test` shares the same blind spot: it greps the rendered HTML for string presence and never runs the JS. **Captured as the open structural follow-up: a headless-browser smoke test (chromedp / playwright via cargo) for the SPA emitter that would catch both classes of defect at CI time** — pairing this with the next audit cycle would close the runtime-defect coverage gap permanently.

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

*   **Status**: **Refuted** (post-verification follow-up to the §3 cycle).
*   **Why refuted**: The claim assumed `--since` / `--until` are applied at SQL-query time via WHERE clauses on `commits.date`. Verification against current `main` shows the time-window filter actually lives at the *ingest* level — `repo/gix_repo.rs:59` wires `opts.after` into `gix::revision::walk::Sorting::ByCommitTimeCutoff`, and `repo/gix_repo.rs:97-98` re-checks each commit against `opts.after` / `opts.before` in the per-commit filter pass. Commits outside the window never make it into the consumer's `CommitEvent` stream, so they never land in the `commits` or `changes` tables. `communication`'s `author_files` CTE operates on `commits` ⊆ window by construction — it's already time-bounded transitively. Adding a redundant `WHERE commits.date BETWEEN ?` clause would be a no-op at best and a maintenance liability at worst (every analysis would acquire the same redundant guard). The original finding's own "Verify-then-fix" caveat correctly anticipated this outcome.
*   **Original audit text preserved for trail**: "Most CodeLore analyses respect `--since` and `--until` boundary filters via WHERE clauses on `commits.date`. `communication`'s `author_files` CTE has no such filter — it's an unbounded `SELECT DISTINCT path, author FROM commits INNER JOIN changes`. If a user runs `codelore communication --since=2024-01-01`, the rest of the pipeline filters the time-window but the per-pair shared-file count silently includes pre-window history."
*   **CLI-flag naming nit**: the audit text said `--since` / `--until`; the actual CLI flags are `--after` / `--before` (mirrored as `Options::after` / `Options::before` in `options.rs:51-52`). Not a defect; just naming drift between the audit phrasing and the implementation.

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

## 4. Second Audit Pass (2026-06-14, post-§3 sweep)

User requested an additional deep-dive loop "until you find real improvement points or real practical or potential issues" — second pass covered surface areas the first pass under-served: **CI/CD + release pipeline**, **identity layer + diff PR-mode + provenance manifest**, and **analytical-formula correctness (Kamei, Fisher exact, clone fingerprinting)**.

Methodology unchanged: three parallel read-only sub-agents → adversarial source-quote verification → only verified-real findings become Active.

**Score**: 21 raw HIGH/MED candidates → **8 Active** findings (F99–F106) + **7 Refuted** (over-fired or misread source) + **5 already-captured-in-§3** (dropped to avoid double-counting).

### Active Findings

#### F99 — Container OCI label `image.source` hardcoded to `<owner>` placeholder

*   **Location**: `Containerfile:60` — `LABEL org.opencontainers.image.source="https://github.com/<owner>/codelore"`
*   **Severity**: MED
*   **Category**: Container image / supply-chain hygiene
*   **Status**: Active
*   **Description**: The Containerfile's `image.source` OCI label is the literal string `https://github.com/<owner>/codelore` — the `<owner>` placeholder was never templated. Anyone who runs `docker inspect ghcr.io/emrecdr/codelore:latest` sees `<owner>` in the source URL. This breaks the OCI spec's intent (clients should be able to dereference `image.source` to the canonical repo), breaks `cosign verify --certificate-identity-regexp ...` style attestation chains that rely on the label, and produces nonsense in security-scanner output (Snyk/Grype/Trivy all surface `image.source`).
*   **Reproduce**: `docker pull ghcr.io/emrecdr/codelore:latest && docker inspect ghcr.io/emrecdr/codelore:latest | jq '.[0].Config.Labels["org.opencontainers.image.source"]'` → `"https://github.com/<owner>/codelore"`.
*   **Suggested fix**: add `ARG REPO=emrecdr/codelore` near the top of the Containerfile, replace the label with `LABEL org.opencontainers.image.source="https://github.com/${REPO}"`. Pass `--build-arg REPO=${{ github.repository }}` from `.github/workflows/container.yml` so the value tracks fork ownership automatically. Confirm via `docker inspect` after the next container build.

#### F100 — `cut-release.sh` ruleset-restore trap can hang indefinitely on stuck `gh api`

*   **Location**: `scripts/cut-release.sh:109-155` (the `restore_ruleset` function registered via `trap … EXIT` at `:156`)
*   **Severity**: MED
*   **Category**: Release-pipeline robustness
*   **Status**: Active
*   **Description**: The trap fires `gh api -X PUT repos/${REPO}/rulesets/${RULESET_ID}` with no timeout. If GitHub's API returns slow / hangs / rate-limits the request, the trap blocks. Worse, the trap already runs *during* shell exit, so a Ctrl-C while it's hung doesn't run a second cleanup — the user kills `gh`, the script exits, and the protect-release-tags ruleset stays in `enforcement: disabled` state on the live repo until someone notices. Per CLAUDE.md, this dance is "the ONLY safe way to publish a `v*` tag" — leaving the repo unprotected breaks that contract. The non-hung failure case is handled (the `else` branch at line 148 prints a manual-recovery command), but the hung case isn't.
*   **Reproduce**: hard to reproduce in dev (would need to script a `gh` hang); review-by-inspection only.
*   **Suggested fix**: wrap the `gh api` call with `timeout 30s gh api …` (GNU coreutils `timeout`, available on Linux + macOS via `brew install coreutils` or as `gtimeout`). On timeout, fall through to the existing manual-recovery `else` branch so the operator gets a paste-able recovery command. Update `docs/RELEASING.md`'s "Tag push ruleset dance" with the timeout caveat.

#### F101 — GitHub Actions cache keys omit `rust-toolchain.toml` fingerprint

*   **Location**: `.github/workflows/release.yml:101` (`key: release-${{ matrix.target }}-${{ hashFiles('**/Cargo.lock') }}`) and `.github/workflows/bench.yml:28` (`key: bench-${{ runner.os }}-${{ hashFiles('**/Cargo.lock') }}`)
*   **Severity**: LOW
*   **Category**: CI cache correctness
*   **Status**: Active
*   **Description**: Cache keys hash `Cargo.lock` but not `rust-toolchain.toml`. The toolchain pin (`1.96.0` today) is the authoritative source per CLAUDE.md — bumping it should invalidate all cache artifacts, since rustc-version is part of every `.rmeta` / `.rlib` hash. Today the workspace pins `1.96.0`, so this is a theoretical concern. The moment the next Rust-bump batch ships (rust-toolchain.toml + workspace rust-version + 5 action invocations + CHANGELOG), CI could hit stale-cache linker errors that are diagnosed as "flaky CI" but are deterministically the missing toolchain fingerprint.
*   **Suggested fix**: change the keys to `${{ matrix.target }}-${{ hashFiles('**/Cargo.lock', 'rust-toolchain.toml') }}` (hashFiles takes a glob list and concatenates). One-line change in two workflow files. Confirms with a forced Rust-version bump in a draft PR — first run must miss the cache.

#### F102 — `bench.yml` kernel-snapshot fetch has no error handling

*   **Location**: `.github/workflows/bench.yml:40-45` — `git clone --depth=10000 --filter=blob:none https://github.com/torvalds/linux.git /tmp/linux-kernel-snapshot`
*   **Severity**: LOW
*   **Category**: CI workflow robustness
*   **Status**: Active
*   **Description**: The `run:` block doesn't `set -euo pipefail`, so if the clone fails (network flap, GitHub rate-limit, transient DNS) the step still exits 0. The subsequent bench step then crashes with `CODELORE_BENCH_LINUX_KERNEL_PATH not found` — a cryptic-symptom-of-a-clear-cause failure pattern. The 2026-06 cache key (`linux-kernel-snapshot-2026-06`) helps on warm runs but the cold-cache cycle is exposed.
*   **Suggested fix**: prepend `set -euo pipefail` to the run block, or explicitly assert post-clone: `[ -d /tmp/linux-kernel-snapshot/kernel ] || { echo "clone failed or repo empty"; exit 1; }`. Two-line change.

#### F103 — Third-party action `softprops/action-gh-release@v3` is tag-pinned (mutable upstream)

*   **Location**: `.github/workflows/release.yml:169` (and adjacent `actions/upload-artifact@v7`, `actions/checkout@v6`, etc.)
*   **Severity**: LOW
*   **Category**: Supply-chain hygiene
*   **Status**: Active
*   **Description**: Industry consensus is *first-party* GitHub actions (`actions/*`) at major-tag pin is acceptable (tight upstream control), but *third-party* actions at tag pin are a known supply-chain risk — the tag can be force-moved upstream to point at a malicious commit. `softprops/action-gh-release@v3` is the only third-party action in the release path and runs with `GITHUB_TOKEN` permissions to create releases (i.e., everything the release pipeline needs to be subverted). The action has a clean reputation today, but tag-pinning a *third-party* action that handles credentials is a measurable risk worth not taking.
*   **Suggested fix**: replace `softprops/action-gh-release@v3` with a full commit SHA: `softprops/action-gh-release@<40-char-sha>`. Pin once, let Dependabot offer SHA bumps. The other third-party actions in the workflow (none found in release.yml besides this one) should follow the same rule. First-party `actions/*` pins stay at major-tag form.

#### F104 — Fisher-exact contingency table can produce degenerate cells on inconsistent inputs

*   **Location**: `crates/codelore-lib/src/analyses/coupling.rs:282-289` (`fisher_two_tail`)
*   **Severity**: LOW
*   **Category**: Statistical robustness
*   **Status**: Active
*   **Description**: The 2×2 contingency-table cells are computed via chained `saturating_sub`: `b = revs_a - shared`, `c = revs_b - shared`, `d = total - a - b - c`. If the inputs are inconsistent (`shared > revs_a` or `shared > revs_b` or `a+b+c > total`) — which "shouldn't happen" under correct SQL — the saturated subtractions silently clamp to 0 and `fishers_exact` is called on a degenerate table. The `.ok()` swallows fishers_exact's error, but a wrong-shaped table that happens to satisfy the crate's input validation still yields a meaningless p-value treated as significant. Inputs come from SQL aggregates over `good_commits`, so they're internally consistent in correct usage — but a future bug in the upstream SQL (esp. under time-bucket aliasing or post-cache hot-fix UPDATE statements) would surface as "more significant pairs than usual" rather than as a typed error.
*   **Suggested fix**: add an invariant check at the top of `fisher_two_tail`: `if shared > revs_a || shared > revs_b || a + b + c > total { return None }`. Three-line defensive add; the `None` propagates the same way today's error case does (caller filters out None).

#### F105 — `ureq = "2"` in build-deps is on maintenance-only branch

*   **Location**: `crates/codelore-lib/Cargo.toml:18` — `ureq = { version = "2", features = ["tls"] }`
*   **Severity**: LOW
*   **Category**: Dependency currency
*   **Status**: Active
*   **Description**: ureq 3.x has shipped as the active release line; ureq 2.x receives only security backports. The build script's network surface is small (one GET per asset, with explicit timeout) so the practical risk today is near-zero — but `ureq 2.x` will eventually stop receiving security updates entirely. Upgrade is a build-script-only change with no runtime impact.
*   **Suggested fix**: bump to `ureq = { version = "3", features = ["tls"] }`, port the `Duration::from_mins(2)` timeout and `.into_reader()` / `.read_to_end()` calls to ureq 3's API (`Agent::run` + `Body::into_reader`). Sanity-test by deleting the cached `OUT_DIR/echarts.min.js` and rebuilding with `--features spa` — the SHA-256 must still match.

#### F106 — Provenance manifest has no explicit schema-version field

*   **Location**: `crates/codelore-lib/src/provenance/mod.rs:16-41` — `struct Manifest`
*   **Severity**: LOW
*   **Category**: Forward-compatibility
*   **Status**: Active
*   **Description**: The `.provenance.json` sidecar carries `codelore_version` (a useful proxy for "what schema is this?"), but no explicit `manifest_version`. Consumers (the planned audit-trail tooling, downstream SLSA tooling, the hypothetical `codelore serve` API) must heuristically reason about schema from `codelore_version` — which couples *consumers* to *codelore's release cadence* even when the manifest schema is stable. A `manifest_version: 1` field would let consumers gate on the schema separately from the producer version.
*   **Suggested fix**: add `pub manifest_version: u8` to `Manifest` (default `1`). Document in the manifest's module docstring that "bump this whenever a field changes type, is removed, or has its semantics changed; *adding* fields is forward-compatible and doesn't bump it." Add a `tests/provenance_test.rs` assertion that the field is present and `>= 1`.

### Refuted in This Pass

| Claim | Refutation |
|---|---|
| Kamei SEXP uses strict `<` instead of paper's `<=` → silently diverges on same-second commits | Refuted. The semantic shift is *explicitly documented* at `kamei/mod.rs:166-173`: "In real repos commits are distinct-second by construction (git commits are sequential), so this is a no-op semantic change. Test fixtures that manufacture same-second commits would notice; the existing `windowed_history_matches_legacy_semantics_on_hot_path` test uses explicit distinct timestamps so `<` and `<=` agree on it." This is a *design decision*, not a bug — the alternative (`<=`) would require an additional tie-break against `rowid` to stay deterministic. |
| Tree-sitter `kind_id()` is not ABI-stable → cache-fingerprint silently invalidated by grammar bumps | Refuted. Tree-sitter grammars are pinned `=0.23.x` in `crates/codelore-lib/Cargo.toml:38-43` exactly because of this concern (documented in CLAUDE.md's "Dependabot has intentional ignore rules" section). A grammar bump can only land coordinated with the codelore version bump; the cache key includes `CARGO_PKG_VERSION` (cache.rs:37), so the cache invalidates by construction whenever grammars change. The "salt" fix the agent proposed duplicates the protection that's already shipped. |
| AI-assist pattern `"co-authored-by: cody"` produces false positives on commits containing "cody" | Refuted. The pattern is the *full* literal `"co-authored-by: cody"`, not bare `"cody"`. `str::contains("co-authored-by: cody")` against a commit message `"wrote cody helpers for testing"` returns false — the `"co-authored-by: "` prefix is the anchor. The agent misread the substring as `"cody"` standalone. |
| AI-attribution `file_ai` CTE conflates NULL `ai_attribution` with human | Refuted. The schema column `commits.ai_attribution` is populated at ingest time by `identity::ai_attribution(...)` for every row (never NULL on a freshly-ingested DB). Older cached DBs are protected by the cache key's `CARGO_PKG_VERSION` slot — a codelore upgrade that changes `ai_attribution` semantics also invalidates the cache. The NULL-on-cache scenario requires both: (a) an old cache survives the version bump, AND (b) the schema migration doesn't touch the column. Neither is currently possible. |
| DuckDB version is pinned 12+ months old, may produce different `--time-bucket` boundaries vs current upstream | Refuted as speculative. No specific upstream DuckDB changelog entry was cited for week-boundary regression; the version pin `=1.10503.1` is intentional (vendored to work around `libduckdb-sys` MSVC 19.40 — see CLAUDE.md and `Cargo.toml`'s `[patch.crates-io]` block). The pin moves when the upstream `duckdb-rs#786` ships; planning the bump on speculation about a different changelog risks regressing the fixed Windows build. |
| Code-health weights (0.40 / 0.25 / 0.15 / 0.20) lack a citation for the *weight* values | Refuted as a finding, accepted as a documentation enhancement. The component citations are in `docs/research-foundations.md` (Campbell 2018 cognitive, Nagappan & Ball 2005 churn, Mockus & Herbsleb 2002 ownership, Tornhill 2018 coupling). The *weighting* is CodeLore's calibration choice — already documented in module-level comments. Capturing it as a F-finding double-counts what's already accepted methodology. |
| SoC `HAVING MAX(files) <= ?` admits boundary cases vs code-maat's strict `<` | Refuted as a finding, accepted as a documented departure. CLAUDE.md and `feedback_modernize_dont_migrate` make explicit: code-maat parity is *opt-in* via `--code-maat-compat`, not the default. The boundary semantics are a *deliberate departure* — modern best practice (inclusive thresholds) over legacy code-maat exact behaviour. |

### Reaffirmed from §3 (deduplicated)

The Round-2 sub-agents independently re-surfaced these Round-1 findings; counted once each, not double-numbered:
- Markdown emitter pipe-escape (F91) — Round-2 confirmed the same gap exists in `diff_output.rs` (lines 165, 184, 212, 234). F91's fix should sweep both `output/markdown.rs` and `codelore-cli/src/diff_output.rs` simultaneously.
- Provenance sidecar atomicity (F92) — independently surfaced, same diagnosis.
- Communication `--since` / `--until` boundary (F95) — independently surfaced.
- Cache canonicalize fallback (F93) — independently surfaced.
- ECharts mount-pattern duplication (F96) — independently surfaced; reinforces that V4 (`widgets.js` modularization) is the natural home for F96's extract.

---

## 5. Next Audit Cycle

Combined Active count after both passes: **F89–F106 = 18 Active findings + V4–V6 improvements**. The next sweep should re-open with F-IDs starting at F107.

The validation methodology held across both passes: 16 + 21 raw HIGH/MED candidates → only 10 + 8 = **18 Active** after source-quote verification. The 13 refuted candidates would have shipped as work if the audit pipeline lacked the verify-against-source gate.
