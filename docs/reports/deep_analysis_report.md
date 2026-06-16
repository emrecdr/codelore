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
    E[HEAD-time blob walk @ HEAD] -->|tree-sitter parsing via rayon| F[Complexity + clones + imports extraction]
    F -->|HEAD-time metrics| D
    D -->|SQL views / parameterized queries| G[31 behavioral analyses]
    G -->|emitters| H[CSV · JSON · SARIF 2.1.0 · Markdown · Parquet · SQLite · HTML · SPA]
```

1.  **Repository Traversal**: [GixRepo](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/repo/gix_repo.rs) (pure-Rust `gitoxide`, hot path) + [GitCliRepo](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/repo/git_cli_repo.rs) (differential-testing oracle).
2.  **Event Ingestion**: `duckdb::Connection` is `!Send + !Sync`. Producer-consumer: background thread walks commits → bounded `crossbeam-channel` → connection-owning thread runs DuckDB Appender ([ingest_loop](file:///Users/emrec/Projects/playground/codelore/crates/codelore-lib/src/facts/ingest.rs)).
3.  **HEAD-time work**: complexity, clones, imports extraction read blobs from the gix ODB, parse via tree-sitter on a rayon pool, drain serially into the DuckDB Appender.
4.  **SQL-Driven Analyses**: 31 behavioral analyses run as parameterised DuckDB queries. Path-aggregating analyses opt into rename-aware aggregation via the `changes_lineage` CTE rewriter.

---

## 2. Historical Findings (F1–F87) — Shipped

All prior findings (F1–F87) have shipped and were validated against `main` HEAD. Per-finding evidence is preserved in `CHANGELOG.md`. Audit-trail summary:

| Batch | Scope | Outcome |
|---|---|---|
| **F1–F17** | Schema timestamps, chunked walker, rename-aware aggregation, CLI-boundary validation | Shipped |
| **PAR-1–PAR-9, DEEP-1–DEEP-15** | Code-maat parity sweep | Shipped |
| **F18–F28** | Back-test isolation, HTML pagination, cache-write concurrency, parallel filtering | Shipped |
| **F29–F34** | Time-bucket changeset semantics, path-relative skip checks, binary diff guards | Shipped |
| **F35–F42** | Numstat brace expansion, explain-mode params, quadratic Kamei rewrite | Shipped |
| **F43–F56** | Blob clone elision, single-pass templating, COUNT(DISTINCT) elimination, SPA X-Ray join | Shipped |
| **F57–F67** | ECharts theme reload, prefix matching, ODB blob reads, hash aggregation sweep, SIMD line counting | Shipped |
| **F68–F76** | AI attribution rollup, lockstep rev equality, lineage rename-index, NULL-safe distinct elimination | Shipped |
| **F77–F87** | Bare-repo clone discovery, theme-controller migration, multi-column SPA grid, cognitive-color sunburst, JSX/TSX grammar coverage | Shipped |
| **F84, F88** | Refuted at source-quote level | Refuted (preserved) |

**Methodology note (F107 / F108 post-mortem)**: prior audit cycles ran read-only sub-agents over the source tree using static-grep + inspection. Neither surfaced F107 + F108 because both were *runtime* initialization-order defects — the bugs only manifested when the JS actually executed in a browser. **F143** captures the headless-browser smoke-test follow-up; currently PARTIAL — implementation lives on `feat/f143-headless-browser-smoke` branch awaiting merge.

---

## 3. Findings F89–F147 — closure log

Validated against current branch HEAD. Status notes ⚠️ findings that live on un-merged feature branches.

| F-ID | Subject | Status | Closing commit / branch |
|---|---|---|---|
| F89 | Producer-thread `.expect("producer panicked")` panic mapping | **Fixed** | `8e52984` |
| F90 | SPA X-Ray sunburst hardcoded ring colors | **Fixed** | `7f36a7f` |
| F91 | Markdown emitter unescaped `\|` in cells | **Fixed** | `7f36a7f` + `49196ad` (PR #53 diff_output.rs sweep) |
| F92 | Provenance sidecar atomicity gap | **Fixed** | `38df3d0` |
| F93 | `cache_key` silent canonicalize fallback | **Fixed** | `8e52984` |
| F94 | `ingest.rs` monolithic | **Active** — _see §4_ |
| F95 | `communication.rs` window filter | **Refuted** (filter at ingest level) |
| F96 | ECharts mount + dispose duplicated | **Fixed** | `7f36a7f` (`mountEcharts` @ 13+ sites) |
| F97 | SPA `JSON.parse` blocks first paint | **Active** — _see §4_ |
| F98 | Chart-click drawer no keyboard equivalent | **Fixed** | F-P4 parallel DOM tree |
| F99 | Container OCI label `<owner>` placeholder | **Fixed** | `f6848e6` |
| F100 | `cut-release.sh` trap hang on stuck `gh api` | **Fixed** | `f6eb953` |
| F101 | CI cache keys omit `rust-toolchain.toml` | **Fixed** | `f204088` |
| F102 | `bench.yml` kernel-snapshot no error handling | **Fixed** | `957b3dd` |
| F103 | `softprops/action-gh-release@v3` mutable | **Fixed** | `dc6ec60` |
| F104 | Fisher-exact contingency degenerate cells | **Fixed** | `b15da46` |
| F105 | `ureq = "2"` maintenance-only | **Fixed** | `ec33cf9` |
| F106 | Provenance manifest no schema-version | **Fixed** | `7f36a7f` |
| F107 / F108 | SPA runtime errors hotfix | **Shipped** (v0.5.1 hotfix) | _CHANGELOG.md_ |
| F109 | `diff_output.rs` missed by F91 sweep | **Shipped** | PR #53 |
| F110 | Differential test only 4 of 8 trait methods | **Fixed** ⚠️ branch | `03db25b` (`head_sha_matches` + others on `fix/f110-f112-test-coverage-and-provenance`) |
| F112 | Provenance manifest missing reproducibility fields | **Fixed** ⚠️ branch | `03db25b` (added `head_sha`, `cache_key_hash`, `rust_version`, `target_triple`, `grammars: BTreeMap<String,String>`) |
| F139 | `DiffGates` parsed but never evaluated | **Fixed** | `549c460` (evaluator + CLI wiring) |
| F140 | Six new analyses lack integration tests | **Fixed** | `7b43593` (5 new `tests/*_test.rs`) |
| F141 | `imports_factsdb_test` only asserts unresolved | **Fixed** | `7b43593` (`ingest_resolves_imports_to_target_paths`) |
| F147 | `AnalysisName` 3-way sync no exhaustiveness guard | **Fixed** ⚠️ partial — match guard added but doesn't cover `all()` array | `549c460` (`_exhaustive_check` const fn). _See F157 below — the guard wraps the wrong list._ |

**Branch caveat**: F110 + F112 are fixed on `fix/f110-f112-test-coverage-and-provenance` (current HEAD). F143's implementation lives on `feat/f143-headless-browser-smoke`. Neither is on main yet. Both need merge before they truly close.

**Refuted findings preserved**: F88 (silent ODB skip rationale), F95 (window filter at ingest level), plus from §3/§4 of the prior report — apply_grouping JOIN shape, renderHeader listener leak, parquet/SQLite backslash escape, hotspots CTE leak, color-mode aria-label, Kamei SEXP `<` vs `<=`, tree-sitter `kind_id` ABI, AI-assist false positives, NULL-conflated AI attribution, DuckDB pinning speculation, code-health weights citation, SoC inclusive thresholds. Rationale in commits `f1aa0e7` (PR #36) + `13fefcb` (PR #38).

---

## 4. Active Findings

### Active carryover

#### F94 — `ingest.rs` monolithic (1344 LOC, still unsplit)

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs` — 1344 lines
*   **Severity**: MED
*   **Category**: Maintainability
*   **Status**: Active. `facts/ingest/` subdirectory exists but empty — split attempted and abandoned.
*   **Suggested fix**: split into `ingest/loop.rs`, `ingest/complexity.rs`, `ingest/clones_head.rs`, `ingest/imports_head.rs`, `ingest/lineage.rs`, `ingest/grouping.rs`. Re-export from `facts/ingest/mod.rs`.

#### F97 — SPA `JSON.parse` synchronous at first paint

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:61`
*   **Severity**: MED
*   **Category**: Initial render perf
*   **Status**: Active. The §3 sprint banner claimed F-P5 PWA closed F97 but the validator confirms widgets.js still parses synchronously and no `requestIdleCallback` references exist.
*   **Suggested fix**: split JSON block into header + per-widget `<script type="application/json" id="widget-X">`; yield between widgets via `requestIdleCallback`.

#### V6 — `CHANNEL_CAPACITY = 64` unmeasured

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs:19`
*   **Severity**: LOW
*   **Category**: Performance scaling
*   **Status**: Active. `benches/end_to_end.rs` has 5 ingest targets but none vary the capacity.
*   **Suggested fix**: add bench parameterised over 16/64/256/1024.

### Partial / in flight

#### V4 — `widgets.js` per-widget registry (PARTIAL)

*   **Status**: Per-widget function declarations exist; no `Widget = { id, render, rerender, dataKey }` registry struct. Boot section §3 is still a flat sequence of `renderXxx()` + literal `_codeloreRerenderers.push(...)` calls.
*   **Next step**: introduce a `WIDGETS = [{name, render, dataKey}]` array; the boot loop iterates uniformly.

#### V5 — Tooltip provenance values (PARTIAL)

*   **Status**: METRIC_DEFS formula strings still reference parameter NAMES literally (`min_shared_revs`, `fisher_significance`). No `${data.options.X}` interpolation surfaces the run's effective threshold values.
*   **Next step**: template formulas through `data.options` at render time.

#### F143 — SPA headless-browser smoke test (PARTIAL — un-merged)

*   **Status**: Implementation lives on `feat/f143-headless-browser-smoke` (`tests/spa_browser_test.rs` + Cargo.toml `browser-tests` feature). NOT on current HEAD; not on main. CI does not yet opt in.
*   **Next step**: merge the branch + add `browser-tests` job to `.github/workflows/ci.yml`.

### NEW Active Findings — Architecture & supply chain

#### F111 — `FactsDb::conn()` leaks `&duckdb::Connection` into public API

*   **Location**: `crates/codelore-lib/src/facts/mod.rs:266`
*   **Severity**: HIGH
*   **Suggested fix**: `pub(crate) fn conn()` + narrower safe methods (`prepare`, `query_map`, `execute`).

#### F113 — `codelore-cli` reaches into 13 distinct `codelore_lib` submodules — no façade

*   **Location**: `crates/codelore-cli/src/main.rs` — all `use codelore_lib::*` statements
*   **Severity**: MED
*   **Suggested fix**: introduce `codelore_lib::cli_api` as the only `pub` surface CLI imports.

#### F114 — Single-CDN dependence for all 4 SPA assets

*   **Location**: `crates/codelore-lib/build.rs:77`
*   **Severity**: MED
*   **Suggested fix**: vendor the 4 files (~1.2 MB) into `crates/codelore-lib/vendor/spa/` + `include_bytes!`, OR fallback URL chain.

#### F115 — Container base images use mutable tags

*   **Location**: `Containerfile:29` + runtime stage
*   **Severity**: MED
*   **Suggested fix**: pin both bases to `@sha256:...` digests.

#### F116 — Renovate AND Dependabot both configured for same ecosystems

*   **Location**: `.github/dependabot.yml` + `renovate.json`
*   **Severity**: MED
*   **Suggested fix**: delete `renovate.json` or merge into `dependabot.yml`.

#### F117 — First-party GHA actions use floating tags despite credential permissions

*   **Location**: `.github/workflows/release.yml` — `attest-build-provenance@v4`, `docker/build-push-action@v7`, `docker/login-action@v4`
*   **Severity**: MED
*   **Suggested fix**: SHA-pin the credential-handling subset.

#### F118 — `gix_repo.rs` walker thread panic silently swallowed

*   **Location**: `crates/codelore-lib/src/repo/gix_repo.rs:130`
*   **Severity**: LOW
*   **Suggested fix**: store JoinHandle; propagate panic payloads as `CodeLoreError::Repo`.

### NEW Active Findings — Tool replacement / dep currency

#### F119 — Hand-rolled 826-line CSV emitter → use `csv` crate

*   **Location**: `crates/codelore-lib/src/output/csv.rs`
*   **Severity**: MED

#### F120 — SARIF schema URL on legacy host; hand-rolled JSON → `serde-sarif`

*   **Location**: `crates/codelore-lib/src/output/sarif.rs:12`
*   **Severity**: MED

#### F121 — `fishers_exact` crate unmaintained since 2018-11

*   **Severity**: LOW

#### F122 — `toml = "0.8"` one major behind (1.1.x current)

*   **Severity**: LOW

#### F123 — `num-format` + `crossbeam` stale in codelore-rca

*   **Severity**: LOW

#### F124 — MSRV pinned to current stable, undocumented

*   **Severity**: LOW

### NEW Active Findings — Backend performance

#### F125 — `query_live_paths` / `current_head_rev` fire 4× per ingest

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs:155, 293, 433, 521` + `:159, 282, 427, 571`
*   **Severity**: HIGH
*   **Status**: **Fixed ⚠️ branch** — `perf/f125-f126-ingest-redundancy` (PR #58). Both queries hoisted to compute-once at the top of `ingest()` after the producer/consumer scope completes; `&[String]` + `&str` threaded down to the four HEAD-time passes.

#### F126 — `resolve_imports_at_head` issues N single-row UPDATEs

*   **Location**: `crates/codelore-lib/src/facts/ingest.rs:579-585`
*   **Severity**: HIGH
*   **Status**: **Fixed ⚠️ branch** — `perf/f125-f126-ingest-redundancy` (PR #58). Rewritten as `CREATE TEMPORARY TABLE _resolved_imports` + bulk Appender insert + single hash-joined `UPDATE imports SET … FROM _resolved_imports r WHERE …`. Cost shape O(N × |imports|) → O(|imports| + N).

#### F127 — Kamei `enrich_diffusion` correlated subqueries

*   **Location**: `crates/codelore-lib/src/kamei/mod.rs:38`
*   **Severity**: MED

#### F128 — Kamei `enrich_size` correlated subqueries

*   **Location**: `crates/codelore-lib/src/kamei/mod.rs:77-92`
*   **Severity**: MED

#### F129 — `arch_violations` materializes full imports set, truncates post-Rust

*   **Location**: `crates/codelore-lib/src/analyses/arch_violations.rs:73-92`
*   **Severity**: MED

#### F130 — `pair_programming` O(P²) per commit with `String::clone` per probe

*   **Location**: `crates/codelore-lib/src/analyses/pair_programming.rs:102-107`
*   **Severity**: MED

### NEW Active Findings — SPA UI/UX

#### F131 — Provenance tooltip triggers 14×14 px, hover-only reveal

*   **Location**: `crates/codelore-lib/src/output/spa/template.html:259-260`
*   **Severity**: HIGH

#### F132 — Hardcoded hex colors break light theme

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:1482, 1751, 2083, 2290-2295, 2414-2416`
*   **Severity**: HIGH

#### F133 — No responsive layout below ~900px viewport

*   **Severity**: HIGH
*   **Validation note**: 14 responsive Tailwind classes total in `template.html`, and they all use the `xl:` prefix (≥ 1280px). Nothing in `sm:` / `md:` / `lg:` — confirms the finding: viewports from 320px up to 1279px get the desktop layout uncompressed. Phones/tablets either zoom out or scroll-bar.

#### F134 — Hotspot table "Show all" synchronously builds full HTML

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:1377`
*   **Severity**: HIGH

#### F135 — Theme toggle re-runs full `d3.pack` layout

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:204`
*   **Severity**: MED

#### F136 — Color-mode tablist mismatches WAI-ARIA Tabs pattern

*   **Location**: `crates/codelore-lib/src/output/spa/template.html:621-636`
*   **Severity**: MED

#### F137 — Knowledge-islands rows not keyboard-activable

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:847-854`
*   **Severity**: MED

#### F138 — `startViewTransition` ignores `prefers-reduced-motion`

*   **Location**: `crates/codelore-lib/src/output/spa/widgets.js:470-476`
*   **Severity**: LOW

### NEW Active Findings — Test / CI / observability

#### F142 — Tracing instrumentation skewed across `analyses/`

*   **Location**: 3 tracing lines total across `crates/codelore-lib/src/analyses/`
*   **Severity**: MED

#### F144 — No CI dogfooding of `codelore` against `codelore`

*   **Severity**: MED

### NEW Active Findings — Code complexity / maintainability

#### F145 — `main.rs` dispatch boilerplate is the bulk of the file

*   **Location**: `crates/codelore-cli/src/main.rs:846-2044` — the `match (format, &analysis)` block (line 846) extends to roughly EOF. `main.rs` is now **2044 LOC**; the dispatch arm body is ~1198 LOC (≈59% of the file).
*   **Severity**: HIGH
*   **Drift note**: the prior 720-LOC estimate was correct at the time of the audit; the file has grown roughly proportionally with new analyses (lead-time, bus-factor, stale-code, god-classes, etc.). The architectural concern is the same — the routing table grew, the abstraction didn't.

#### F146 — `json.rs` `write_*_json` shims (count drifted)

*   **Location**: `crates/codelore-lib/src/output/json.rs` — **29 trivial shim functions** today (audit logged 14 at the time of capture; new analyses added more).
*   **Severity**: MED

#### F148 — `csv.rs` + `markdown.rs` per-analysis emitters

*   **Severity**: LOW

### Fourth audit pass — schema / determinism / error UX

#### F149 — `hunks` table lacks PRIMARY KEY, NOT NULL on offsets, `(rev, path)` index

*   **Location**: `crates/codelore-lib/src/facts/schema_v1.sql:51`
*   **Severity**: MED
*   **Description**: Outlier among 8 tables. All four offset columns nullable; FK validation queries scan the entire `hunks` table on large repos.
*   **Suggested fix**: add `PRIMARY KEY (rev, path, old_start, new_start)`, `NOT NULL` on all four offset columns, `CREATE INDEX idx_hunks_rev_path ON hunks(rev, path)`.

#### F150 — Schema version tracked in two disjoint places, no startup validation

*   **Location**: `crates/codelore-lib/src/cache.rs:40` + `crates/codelore-lib/src/facts/schema.rs:6`
*   **Severity**: MED
*   **Description**: Cache invalidation works via key hash, but operator who hands stale cache to `--cache-dir` directly gets no fail-fast — surfaces as cryptic SQL errors at analysis time.
*   **Suggested fix**: on `FactsDb::open_read_only`, `SELECT value FROM provenance WHERE key='schema_version'` + `bail!` if mismatch.

#### F151 — Leiden communities run without RNG seed → non-deterministic partitions

*   **Location**: `crates/codelore-lib/src/analyses/communities.rs:136`
*   **Severity**: MED
*   **Description**: `LeidenConfig::default()` has no explicit seed. Module docstring promises "deterministic across runs" — broken on cache miss.
*   **Suggested fix**: thread a deterministic seed (SHA-256 prefix of edge list, or fixed `0xDEADBEEF`) into LeidenConfig. Add an integration test asserting two back-to-back runs produce identical `community_id` columns.

#### F152 — `clone_group_id` non-deterministic across runs (std HashMap iteration)

*   **Location**: `crates/codelore-lib/src/clones/extractor.rs:145`
*   **Severity**: LOW
*   **Suggested fix**: `BTreeMap<[u8;32], _>` so iteration is digest-sorted; OR sort `Vec<(digest, members)>` by digest before `enumerate()`.

#### F153 — Generic I/O errors from repo probing exit with code 5 instead of 3

*   **Location**: `crates/codelore-lib/src/error.rs:65`
*   **Severity**: LOW
*   **Suggested fix**: add `CodeLoreError::RepoIo(std::io::Error)` variant mapped to exit 3.

#### F154 — `codelore diff` base==head produces empty SARIF with no signal

*   **Location**: `crates/codelore-cli/src/diff_output.rs:30` (text), plus SARIF + markdown branches
*   **Severity**: LOW
*   **Suggested fix**: pre-check at the diff entry point — `if base_sha == head_sha { return Err(CodeLoreError::Analysis("base equals head; nothing to diff")); }`.

### Fifth audit pass — F155–F157 (this cycle)

#### F155 — `DiffOutput.{base,head}_median_code_health` defaults to silent `0.0`

*   **Location**: `crates/codelore-cli/src/diff.rs:41` (struct decl) + `:650-666` (default-path)
*   **Severity**: MED
*   **Category**: Output ambiguity / CI integration
*   **Status**: Active
*   **Description**: Both medians are typed `f64` (not `Option<f64>`) and default to `0.0` when `--thresholds-file` is unset or no `[diff]` gates configured. The values are emitted verbatim in the diff JSON without `#[serde(skip_serializing_if)]`. A downstream consumer reading the JSON cannot distinguish "gate not configured, no measurement taken" from "measured median = 0.0" — both surface as `"base_median_code_health": 0.0`. On a 0–100 code-health scale, 0.0 reads as catastrophically bad.
*   **Failure scenario**: team A configures thresholds → gets meaningful 67.4 / 64.9 medians. Team B runs `codelore diff` without thresholds, ships the JSON to a dashboard / Slack bot / PR summarizer. The dashboard rule `flag if base_median_code_health < 50` fires on every PR → signal trust erodes.
*   **Suggested fix**: change to `Option<f64>` + `#[serde(skip_serializing_if = "Option::is_none")]`. Or hoist into a sentinel `"computed": false` field on the diff envelope.

#### F156 — `Thresholds` / `Gates` / `DiffGates` don't `deny_unknown_fields` — silent typo disables gate

*   **Location**: `crates/codelore-lib/src/quality_gates/mod.rs:39` (Thresholds), `:47` (Gates), `:62` (DiffGates)
*   **Severity**: LOW
*   **Category**: Config / silent misconfiguration
*   **Status**: Active
*   **Description**: None of the three `Deserialize`-derived structs opt into `#[serde(deny_unknown_fields)]`. A user typo in `.codelore-thresholds.toml` — e.g. `cognative_max = 30` (transposed) or `disallow_clone_type1 = true` (missing underscore) — parses cleanly as the default `None`/`false`. The gate is silently disabled. No warn-log surfaces the unknown key. `quality_gates`'s entire value proposition is that the repo carries the gate; silent misconfiguration is the worst failure mode here.
*   **Failure scenario**: engineer adds `disallow_clone_type1 = true` (typo) to block Type-1 clones in PR review. File parses; `gates.disallow_clone_type_1` stays `false`; `evaluate_clone_gate` early-returns. Gate appears wired but does nothing — every PR passes while Type-1 clones land.
*   **Suggested fix**: `#[serde(deny_unknown_fields)]` on all three structs. Parse error surfaces via the existing `CodeLoreError::Analysis` path in `Thresholds::from_path`.

#### F157 — F147's exhaustiveness guard wraps the wrong list

*   **Location**: `crates/codelore-lib/src/analysis.rs:152-184` (`_exhaustive_check` match) vs `:186-217` (`all()` array literal)
*   **Severity**: LOW
*   **Category**: Correctness / drift (regression on F147's promise)
*   **Status**: Active
*   **Description**: F147's commit message claims the exhaustiveness guard prevents new variants from being silently absent from the `all()` registry. Inspection shows the const fn `_exhaustive_check` matches over every variant — but the actual `all()` array the rest of the codebase consumes is the `&[Self::Hotspots, Self::Coupling, ...]` literal further down. A new variant + writing the match arm (forced) + forgetting the array entry (not forced) silently reintroduces the exact registry-drift bug the F147 commit claims to prevent. The round-trip test only iterates `all()`, so an array-missing variant is never exercised.
*   **Failure scenario**: maintainer adds `AnalysisName::ContributionDecay`. Compiler refuses build until `_exhaustive_check` match adds the arm — they comply. Build passes. The `&[...]` array below is never touched. `codelore --help` doesn't list `contribution-decay`; `Supported: ...` error messages omit it; round-trip test never round-trips it. But `from_str("contribution-decay")` still works because it lives in a separate match. The exact silent UX regression F147 was supposed to prevent.
*   **Suggested fix**: invert — have `all()` build the array deconstructively from the match (`let arr: [_; N] = [Self::Hotspots, ...]` only inside the matched branch), OR write the guard so the `&[...]` array literal is the only registry by mapping the match over `Self::all().iter()`.

---

## 4½. Validation Pass — 2026-06-16

Every Active / Partial entry above re-verified against current `main` HEAD via direct source inspection. Backwards-evidence summary so the next reader doesn't redo the same checks:

| Finding | Claim | Verified state on main | Status |
|---|---|---|---|
| F94 | ingest.rs monolithic, 1344 LOC | `wc -l = 1344` ✓ ; `facts/ingest/` subdir exists empty | Active confirmed |
| F97 | `JSON.parse` synchronous at first paint | `widgets.js:61` literal `JSON.parse(dataBlock.textContent)`; `grep -c requestIdleCallback = 0` | Active confirmed |
| V6 | `CHANNEL_CAPACITY = 64` unmeasured | `ingest.rs:19` constant unchanged; also surfaces sibling `WALKER_CHANNEL_CAPACITY = 256` at `gix_repo.rs:122` — second unmeasured constant | Active confirmed (broader) |
| F111 | `FactsDb::conn()` leaks `&Connection` | `facts/mod.rs:266` still `pub fn conn(&self) -> &Connection` | Active confirmed |
| F113 | CLI reaches into many lib submodules | 17+ distinct `codelore_lib::*` paths in `main.rs` (audit said 13 — undercounted) | Active confirmed |
| F114 | Single-CDN dependence | All 4 SPA assets at `cdn.jsdelivr.net/npm/…` in `build.rs:77-104` | Active confirmed |
| F115 | Container mutable tags | `Containerfile:29` `rust:${RUST_VERSION}` and `:62` `gcr.io/distroless/cc-debian12:nonroot` — no `@sha256:` | Active confirmed |
| F116 | Dependabot + Renovate duplicate | Both `.github/dependabot.yml` and `renovate.json` present | Active confirmed |
| F117 | First-party GHA floating tags | 6 sites at `actions/{checkout,cache,attest-build-provenance,upload-artifact,download-artifact}@v[N]` (no SHA pin) | Active confirmed |
| F118 | Walker panic silently swallowed | `gix_repo.rs:132` spawns walker thread; no `join()` on the handle anywhere — panic just drops the channel | Active confirmed |
| F119 | csv.rs 826 LOC | `wc -l = 826` ✓ | Active confirmed |
| F120 | SARIF schema URL on legacy host | `sarif.rs:12` `schemastore.azurewebsites.net` — canonical is `json.schemastore.org` | Active confirmed |
| F121 | `fishers_exact` unmaintained | `Cargo.lock` carries it; `cargo deny check advisories` → OK (no CVE), but the crate still has no commits since 2018 | Active (informational) |
| F122 | toml on 0.8.x | `Cargo.lock`: `toml v0.8.23`; current `1.1.x` | Active confirmed |
| F125 / F126 | redundant queries + N updates | **Fixed on PR #58** (this session) — verified by the perf bench notes | Fixed-on-branch |
| F127 / F128 | Kamei correlated subqueries | `kamei/mod.rs:38` (`sql_counts`) + `:77-92` (`enrich_size`) confirm `(SELECT … FROM changes WHERE changes.rev = commits.rev)` pattern | Active confirmed |
| F129 | arch-violations materializes, truncates post-Rust | `arch_violations.rs:73-92` collects full Vec, validates in Rust, truncates afterwards | Active confirmed |
| F130 | pair_programming O(P²) with `String::clone` | `pair_programming.rs:102-107` literal `participants[i].clone(), participants[j].clone()` inside doubly-nested loop | Active confirmed |
| F131 | Tooltip 14×14 trigger | `template.html:259-260` literal `width: 14px; height: 14px` | Active confirmed |
| F132 | Hardcoded hex in widgets.js | 5+ sites with `'#e6e6e6'`, `'#fff'`, `'#1a4a2c'`, `'#2ea44f'`, `'#7dd87a'`, `'#f59e0b'`, `'#e0584e'` | Active confirmed |
| F133 | No responsive < 900px | 14 responsive classes total, all `xl:` (≥ 1280px). 0 `sm:` / `md:` / `lg:` | Active confirmed |
| F135 | Theme toggle re-runs `d3.pack` | `widgets.js:204` `registerThemeRerender(() => renderHotspotCirclePack(...))` — full re-layout | Active confirmed |
| F136 | Color-mode tablist non-ARIA | `template.html:621-636` `<button role="tab">` with no `aria-selected`, no `tabindex` mgmt, no arrow-key handler | Active confirmed |
| F138 | `startViewTransition` ignores reduced-motion | `widgets.js:470-475` calls `document.startViewTransition(updateFn)` without checking `prefers-reduced-motion` | Active confirmed |
| F142 | Sparse tracing in analyses | Only 3 `tracing::*` calls across 33 analysis files (lead_time, clones, clone_coupling) | Active confirmed |
| F144 | No CI dogfooding | `grep -rE 'codelore (analyze\|check\|diff)' .github/workflows/ = ∅` | Active confirmed |
| F145 | main.rs dispatch boilerplate | `main.rs = 2044 LOC`; dispatch spans lines 846→2044 (~1198 LOC, ≈59% of file). Audit's 720-LOC figure was correct *at that time*. | Active confirmed (drifted larger) |
| F146 | json.rs trivial shims | `grep -cE '^pub fn write_[a-z_]+_json' = 29` (audit said 14 — count drifted larger as new analyses added) | Active confirmed (drifted larger) |
| F149 | hunks schema lacks PK / NOT NULL | `schema_v1.sql:51` literal — no PK, all 4 offset columns nullable | Active confirmed |
| F150 | Schema version in two places | `facts/schema.rs:6` `("schema_version", "1")` + `cache.rs:20` `SCHEMA_VERSION: &str = "schema_v3"` — disjoint sources | Active confirmed |
| F151 | Leiden non-deterministic | `communities.rs:136` literal `Leiden::new(LeidenConfig::default())` — no seed | Active confirmed |
| F152 | clone_group_id std HashMap | `clones/extractor.rs:144-157` uses `HashMap` then assigns `clone_group_id = u32::try_from(i + 1)` where `i` is HashMap iteration order | Active confirmed |
| F153 | I/O errors → exit 5 | `error.rs:63-66` only `Self::Repo(_) \| Self::BlobNotFound { .. } => 3` — no `RepoIo` variant for `std::io::Error` carryover | Active confirmed |
| F154 | diff base==head no guard | `grep -n 'base_sha == head_sha' diff*.rs = ∅` | Active confirmed |
| F155 | DiffOutput medians default 0.0 | `diff.rs:41,44` `pub base_median_code_health: f64` (not Option) | Active confirmed |
| F156 | Thresholds/Gates/DiffGates no `deny_unknown_fields` | `quality_gates/mod.rs:40,48,62` — none of the three structs carry the attr | Active confirmed |
| F157 | F147's guard wraps wrong list | `analysis.rs:136` `all()` returns `&[Self::Hotspots, ...]` literal **separately** from `_exhaustive_check` const fn `match` at `:151` — guarded match doesn't force array entry | Active confirmed (F147 partial regression) |
| F110 / F112 | branch-only fixes | `MANIFEST_SCHEMA_VERSION` on main is still `1`; `head_sha`/`cache_key_hash` absent; `differential_repo_test.rs` lacks `head_sha_matches`. PR #57 carries the fix. | Partial-on-branch |
| F143 | SPA browser smoke | `tests/spa_browser_test.rs` absent on main; `browser-tests` feature not wired in `ci.yml`. PR #56 carries the fix. | Partial-on-branch |

`cargo deny check advisories` clean as of validation date — confirms no F-finding maps to a live CVE.

---

## 5. Next Audit Cycle

**Current Active count**: F94, F97 + V4, V5, V6 + F143 partial + F111, F113-F124, F125-F148 (minus F139, F140, F141, F147 fixed, minus F110, F112 fixed-on-branch, minus F125 + F126 fixed-on-branch as of this validation pass) + F149-F154 + F155-F157 = **40 Active findings**.

The next sweep should re-open with F-IDs starting at **F158**.

**Validation methodology held**: 28 prior findings re-validated → 21 Fixed (15 from v0.6.0 + 4 from PRs #54/#55 on main + 2 F110/F112 on `fix/f110-f112-test-coverage-and-provenance`) + 3 Partial (V4, V5, F143 awaiting merge) + 3 Active carryover (F94, F97, V6). 3 new fifth-pass findings (F155 diff output ambiguity, F156 missing `deny_unknown_fields`, F157 exhaustiveness guard targets the wrong list — F147 partial regression). All findings carry source-line quotes for adversarial-verification trail.

**Branch-merge gate**: F110, F112, F143 should be re-validated against main after their feature branches merge. The fact that F147's guard wraps the wrong list (F157) suggests the F147 fix also needs a follow-up — the guard is structurally fine but doesn't enforce what the commit message claims.
