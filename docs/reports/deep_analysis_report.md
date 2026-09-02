# CodeLore — Deep Codebase Analysis Report

Read-only audit log. Findings are immutable F-IDs; the status field tracks state.
Shipped/fixed findings are condensed to a one-line closure row once validated against `main` (full history in `CHANGELOG.md` + git); refuted findings stay documented to prevent rediscovery.

**Last pass: 2026-09-02** (the second-wave discovery report, `docs/reports/2026-09-02-deep-analysis-second-wave.md`). The 2026-07-01 validation + 5-dimension discovery pass added **F200–F230**; the 2026-07-02 implementation pass landed 28 of them on `main` (PRs #71, #74) and refuted F188/F202. See §3 "Implemented" tables + §6 for the current disposition.

---

## 1. Architectural Overview & Pipeline Data Flow

CodeLore is a multi-crate Rust workspace:
*   **codelore-rca**: Vendored fork of Mozilla `rust-code-analysis` — structural syntax hashing + complexity metrics. Hands-off (MPL); out of audit scope.
*   **codelore-lib**: Core engine — repository walk abstraction, identity resolution, fact-store management, analyses, caching, output emitters.
*   **codelore-cli**: Argument parsing, option consolidation, dispatch, output routing.

### Data Ingest Flow

```mermaid
graph TD
    A[GixRepo / GitCliRepo] -->|walk_commits → CommitEvent stream| B[Bounded crossbeam channel]
    B -->|producer → consumer| C[FactsDb ingest]
    C -->|DuckDB Appender bulk-insert| D[(DuckDB fact store)]
    E[HEAD-time blob walk @ HEAD] -->|tree-sitter parsing via rayon| F[Complexity + clones + imports extraction]
    F -->|HEAD-time metrics| D
    D -->|SQL views / parameterized queries| G[57 behavioral analyses]
    G -->|emitters| H[CSV · JSON · SARIF 2.1.0 · Markdown · Parquet · SQLite · HTML · SPA · GHA]
```

1.  **Repository Traversal**: `GixRepo` (pure-Rust `gitoxide`, hot path) + `GitCliRepo` (differential-testing oracle).
2.  **Event Ingestion**: `duckdb::Connection` is `!Send + !Sync`. Producer-consumer: background thread walks commits → bounded `crossbeam-channel` → connection-owning thread runs DuckDB Appender (`facts/ingest/consumer.rs::ingest_loop`).
3.  **HEAD-time work**: complexity, clones, imports extraction read blobs from the gix ODB, parse via tree-sitter on a rayon pool, drain serially into the DuckDB Appender.
4.  **SQL-Driven Analyses**: 57 behavioral analyses run as parameterised DuckDB queries. Path-aggregating analyses opt into rename-aware aggregation via the `changes_lineage` CTE rewriter.

---

## 2. Historical Findings (F1–F87) — Shipped

All prior findings (F1–F87) shipped and were validated against `main`. Per-finding evidence is in `CHANGELOG.md`. Audit-trail summary:

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

---

## 3. Findings F89–F199 — closure log (validated 2026-07-01)

Every row below was re-verified against `main` HEAD by read-only source inspection on 2026-07-01. Fixed rows are condensed to one line (details in `CHANGELOG.md` / git). Rows still open carry forward to §4.

| F-ID | Subject | Status |
|---|---|---|
| F89 | Producer-thread `.expect` panic mapping | Fixed (`8e52984`) |
| F90 | SPA X-Ray sunburst hardcoded ring colors | Fixed (`7f36a7f`) |
| F91 | Markdown emitter unescaped `\|` in cells | Fixed (`7f36a7f`+`49196ad`) |
| F92 | Provenance sidecar atomicity gap | Fixed (`38df3d0`) |
| F93 | `cache_key` silent canonicalize fallback | Fixed (`8e52984`) |
| F94 | `ingest.rs` monolithic → `facts/ingest/` module split | Fixed |
| F95 | `communication.rs` window filter | Refuted (filter at ingest level) |
| F96 | ECharts mount + dispose duplicated | Fixed (`7f36a7f`) |
| F97 | SPA boot-time render storm blocks first paint | Fixed (async `bootWidgets`) |
| F98 | Chart-click drawer no keyboard equivalent | Fixed |
| F99 | Container OCI label `<owner>` placeholder | Fixed (`f6848e6`) |
| F100 | `cut-release.sh` trap hang on stuck `gh api` | Fixed (`f6eb953`) |
| F101 | CI cache keys omit `rust-toolchain.toml` | Fixed (`f204088`) |
| F102 | `bench.yml` kernel-snapshot no error handling | Fixed (`957b3dd`) |
| F103 | `softprops/action-gh-release@v3` mutable | Fixed (`dc6ec60`) |
| F104 | Fisher-exact contingency degenerate cells | Fixed (`b15da46`) |
| F105 | `ureq = "2"` maintenance-only | Fixed (`ec33cf9`; now `ureq 3`) |
| F106 | Provenance manifest no schema-version | Fixed (`7f36a7f`) |
| F107/F108 | SPA runtime errors hotfix | Shipped (v0.5.1) |
| F109 | `diff_output.rs` missed by F91 sweep | Shipped (PR #53) |
| F110 | Differential test only 4 of 8 trait methods | Fixed (PR #57) |
| F111 | `FactsDb::conn()` leaks `&Connection` | Fixed (`pub(crate)` + safe methods) |
| F112 | Provenance manifest missing reproducibility fields | Fixed (PR #57) |
| F113 | CLI reaches into many lib submodules — no façade | Fixed (`cli_api`) |
| F114 | Single-CDN dependence for SPA assets | Fixed (`url_fallbacks`) |
| F115 | Container base images use mutable tags | Fixed (`@sha256:` pins) |
| F116 | Renovate AND Dependabot duplicate ecosystems | Refuted (partitioned by ecosystem) |
| F117 | First-party GHA credential actions use floating tags | Fixed (SHA-pinned) |
| F118 | gix walker thread panic silently swallowed | Fixed (PR #62) |
| F120 | SARIF schema URL on legacy host | Fixed (canonical schemastore URL) |
| F121 | `fishers_exact` crate unmaintained | Fixed (in-tree `stats::fisher_two_tail_pvalue`) |
| F122 | `toml = "0.8"` one major behind | Fixed (`toml = "1"`) |
| F123 | codelore-rca crossbeam/num-format stale | Refuted (both are current releases) |
| F124 | MSRV pin has zero buffer | Fixed (documented policy) |
| F125 | Redundant queries fire 4× per ingest | Fixed (PR #58, hoisted once) |
| F126 | N single-row UPDATEs in resolve_imports | Fixed (PR #58, bulk UPDATE…FROM) |
| F127 | Kamei `enrich_diffusion` correlated subqueries | Fixed (full, incl. entropy block) |
| F128 | Kamei `enrich_size` correlated subqueries | Fixed (PR #64) |
| F129 | `arch_violations` materialise-then-truncate | Fixed (direct-iterate + early-break) |
| F130 | `pair_programming` O(P²) with `String::clone` | Fixed (integer-interned keys) |
| F131 | Provenance tooltip 14×14 px target | Fixed (24×24, WCAG 2.5.5) |
| F132 | Hardcoded hex in widgets.js breaks light theme | Fixed (CSS tokens) |
| F133 | No responsive layout < ~1280px | Fixed (`md:grid-cols-2`) |
| F134 | Hotspot 'Show all' synchronous HTML build | Fixed (chunked + `yieldToMain`) |
| F135 | Theme toggle re-runs full d3.pack layout | Fixed (yield between rerenderers) |
| F136 | Color-mode tablist mismatches WAI-ARIA | Fixed (`aria-selected`) |
| F137 | Knowledge-islands rows not keyboard-activable | Fixed (`wireRowKbActivation`) |
| F138 | `startViewTransition` ignores reduced-motion | Fixed (PR #62) |
| F139 | `DiffGates` parsed but never evaluated | Fixed (`549c460`) |
| F140 | Six new analyses lack integration tests | Fixed (`7b43593`) |
| F141 | `imports_factsdb_test` only asserts unresolved | Fixed (`7b43593`) |
| F142 | Sparse tracing across `analyses/` | Fixed — **residual: 6 `dashboard.rs` fns still uninstrumented → new F224** |
| F143 | SPA headless-browser smoke test | Fixed (PR #56) |
| F144 | No CI dogfooding of `codelore` on itself | Fixed (`dogfood` job) |
| F145 | `main.rs` dispatch boilerplate | Fixed (1-D dispatch fns) |
| F146 | `json.rs` trivial `write_*_json` shims | Fixed (generic `write_json`) |
| F147 | `AnalysisName` 3-way sync no exhaustiveness guard | Fixed (`registry!` macro) |
| F149 | `hunks` table lacks PK + NOT NULL + index | Fixed (schema + full ingest wiring) |
| F150 | Schema version disjoint, no startup validation | Fixed (PR #61) |
| F151 | Leiden communities non-deterministic | Fixed (PR #61, `LEIDEN_SEED`) |
| F152 | `clone_group_id` non-deterministic | Fixed (PR #61, `BTreeMap`) |
| F153 | `--team-map` config IO error exits 5 not 3 | Fixed (`RepoIo`) — **residual: other config readers still wrong → new F211** |
| F154 | `codelore diff` base==head empty SARIF | Fixed (PR #62) |
| F155 | `DiffOutput` medians default silent 0.0 | Fixed (PR #60, `Option<f64>`) |
| F156 | Thresholds/Gates/DiffGates no `deny_unknown_fields` | Fixed (PR #60) |
| F157 | F147 guard wraps the wrong list | Fixed (PR #60) |
| F158 | SARIF `informationUri` wrong project URL | Fixed (PR #63) |
| F159 | SARIF `artifactLocation.uri` not percent-encoded | Fixed (PR #63) |
| F160 | Kamei NDEV/EXP same-second `<` vs `<=` | Fixed (PR #64) |
| F162 | Parquet column types drift from CSV row-type | Fixed (shared SQL generators) |
| F163 | SARIF `automationDetails.id` static | Fixed (PR #63) |
| F164 | Task-ID (`F<NN>`) refs in code comments | Fixed — **residual: `Plan 1/4` markers in gix_repo.rs → new F205** |
| F165 | `--format ndjson`/`gha` panics `unreachable!` | Fixed (clean `bail!`) |
| F166 | `codelore schema` row-type list drifted | Fixed (derives from `AnalysisName::all()`) |
| F167 | stale-code/delivery-friction wall-clock anchor | Fixed (`MAX(commits.date)` + `age_time_now`) |
| F168 | `lead-time` ORDER BY lacks tiebreaker | Fixed (`, rev ASC`) |
| F169 | Treemap breadcrumb undefined `--bg-elev-1` | Fixed (`--bg-elev`) |
| F170 | CSV emitter no formula-injection guard | Fixed (`'` prefix + force-quote) |
| F171 | `bus-factor` drops root files + not rename-aware | Fixed (`<root>` bucket + lineage) |
| F172 | Calendar heatmap `Math.min.apply` RangeError | Fixed (single-pass loop) |
| F174 | `run_coupling` recomputed 2–5×, no memoization | Fixed (per-`FactsDb` `Rc` memo) |
| F175 | SPA detail drawer no focus management | Fixed (focus enter/return + `aria-labelledby`) |
| F176 | Six SQL analyses in `output/spa.rs`, exit-5 leak | Fixed (moved to `analyses/dashboard.rs`, exit-4) — **residual: no tests/tracing → new F223/F224** |
| F178 | `query_map_collect` under-adopted | Fixed (~73% adoption) |
| F179 | Tablists lack arrow-key nav / roving tabindex | Fixed (full WAI-ARIA tabs pattern) |
| F180 | Charts expose no text alternative | Fixed (`role=img`+`aria-label` at 12 sites) |
| F181 | No `prefers-reduced-motion` CSS block | Fixed |
| F182 | Dynamic updates silent to screen readers | Fixed (`aria-live` summary) |
| F183 | Selection-listener stale-closure leak | Fixed (dedup-by-source) |
| F184 | `changes_lineage` rebuilt every analysis call | Fixed (build-once guard) |
| F185 | Clone `Fingerprint` retains unused `sequence` | Fixed (field removed) |
| F187 | `just test` diverges from CI invocation | Fixed (feature scope matches CI) |
| F189 | `vendor-duckdb-rs.sh` no retry / mutable-tag TOFU | Fixed (retry + SHA pin + stamp) |
| F190 | `explain` covers 15/32; no anti-drift test | Fixed (31 topics + enforced coverage test) |
| F192 | mi/communities/centrality no `run_*` tests | Fixed (3 integration tests) |
| F193 | `resolve_imports_at_head` builds path set twice | Fixed (built once, shared) |
| F194 | Kamei `enrich` re-materializes `changes_lineage` | Fixed (subsumed by F184 guard) |
| F195 | `deny.toml` multiple-versions, no skip-list | Fixed (explicit skip-list) |
| F196 | `release.yml` no sccache warm-cache reuse | Fixed (sccache wired) |
| F198 | Two SQL source-swap mechanisms, false-symmetry doc | Fixed (doc corrected; both intentional) |
| F199 | `Options::validate()` reports `Provenance` variant | Fixed (`InvalidOptions` + test) |

### Implemented 2026-07-01 (this session — CI-exact gate green: fmt + clippy `--all-features -D warnings` + `test --features test-support,spa` (75 binaries) + `cargo deny`)

| F-ID | Subject | Fix |
|---|---|---|
| F191 | Usage errors (`--complexity-sample`) exit 1 | Typed `CodeLoreError::InvalidOptions` (exit 2); leaked `Plan` markers dropped from the user-facing message |
| F201 | `read_blob_at` directory-path divergence | `GitCliRepo` uses `git cat-file blob` (errors on non-blob → `Ok(None)`, matching `GixRepo`); differential test `read_blob_at_returns_none_for_a_directory_path` |
| F203 | Dead `Options.commit_range` knob | Field + `Default` + two doc refs removed; cache key auto-updates via the serde derive |
| F204 | Dead `CodeLoreError::Provenance` variant | Variant removed; exit-code test re-anchored on `InvalidOptions` |
| F205 | Stale banned `Plan 1/4` markers on `compute_changed_files` | Comment rewritten to the current contract (real loc + hunks). Broader `Plan N` sweep → new F231 |
| F207 | `cycle-origins` no rev-keyed graph memo | `HashMap<rev, Rc<ImportGraph>>` shared across bisections via `graph_at_rev_cached` |
| F208 | Structural import graph rebuilt per arch analysis | Per-`FactsDb` `Rc<ImportGraph>` memo; `build_import_graph` returns the shared handle |
| F209 | `apply_grouping` row-by-row INSERT | DuckDB Appender + `flush()` |
| F210 | `clone-coupling` 4 `String` clones/edge | Borrowed `(&str, &str)` probe-map keys — zero clones |
| F211 | Config file-reads → `Analysis` not `RepoIo` | arch-rules / thresholds / group-file read failures → `RepoIo` (exit 3); parse failures stay `Analysis` (exit 4). Finishes the F153 job |
| F212 | `unwrap_or_default` masks head-rev read error | Typed `map_err` (consistent with the surrounding plumbing) |
| F213 | `pair_programming` dead `params!` lint-defeat | `use duckdb::params;` + throwaway `let _ = params![…]` removed |
| F214 | `is_bot` allocates 2 `String`s/commit | Non-allocating ASCII `contains_ignore_ascii_case`; both `is_bot` sites |
| F216 | SPA coupling Sankey/drawer read non-existent fields | Read real `shared`/`degree`; band width, "top 30" sort, and drawer label all corrected |
| F217 | Reset-zoom buttons never installed (async-boot race) | Idempotent installer re-run at end of `bootWidgets()` |
| F219 | Trends `shortPath` label collisions | Collision falls back to the unique full path; series keyed by the disambiguated label |
| F220 | Calendar `visualMap` degenerate at `min===max` | Anchor low end at 0 so the single value paints a visible band |
| F221 | Tooltip `?` in `<th>` triggers sort | Sort handler skips clicks originating from `.tooltip-trigger` |
| F222 | Hotspot zero-filter-match blank body | Inline "No paths match '…'" empty-state row |
| F223 | Six `dashboard.rs` fns no integration test | New `tests/dashboard_test.rs` drives all six over the ingested fixture |
| F224 | Six `dashboard.rs` fns lack tracing spans | `#[tracing::instrument]` on all six (matches every sibling analysis) |
| F225 | No exit-2 CLI test | `invalid_options_exit_with_code_2` (`--min-coupling` > `--max-coupling` → exit 2) |
| F226 | `build_import_graph_from_edges` HashSet iteration order | Sorted edge list before adjacency build (removes per-process nondeterminism) |
| F227 | `ANY_VALUE()` nondeterministic in bucketed/grouped ingest | `arg_max(…, ROW(m.date, -m.rowid))` (bucketed) / `arg_max(…, c.path)` (grouped) |
| F228 | hotspot-velocity window consts doc-only | SQL is now a `{recent}/{baseline}/{boundary}` template resolved from the consts (byte-identical) |

### Implemented + validated 2026-07-02 (validate-then-implement over the deferred backlog)

Deep validation of the deferred backlog: one clean win shipped, two findings refuted as intentional designs, one confirmed ready-to-execute.

| F-ID | Subject | Outcome |
|---|---|---|
| F200 | `commit_metadata` stub divergence + vacuous differential test | **Fixed** — deleted the unused `commit_metadata` trait method (both backends) + the `CommitMetadata` type + the vacuous `commit_metadata_match` test + two other dead references. **Kept** `changed_files`/`diff_hunks` (their differential tests are *real* cross-checks, not vacuous). Narrower than "delete all three oracle-only methods" — only `commit_metadata` was both divergent (gix stub vs cli-real) and vacuously tested. Gate green: fmt + clippy `--all-features -D warnings` + test (75 binaries) + deny. |
| F188 | `cut-release.sh` ruleset body omits spa-browser/dogfood | **Refuted** — the omission is *intentional + documented*. `cut-release.sh:230-232` explicitly names spa-browser/dogfood as known-excluded; `dogfood` is `continue-on-error` (correctly not a required gate); and a live-vs-hardcoded drift-detector (`:250`) guards against divergence. Adding spa-browser would be a *policy* change (make it gate release tags), not a bug fix — the maintainer's call. |
| F202 | Fan-in/out computed three inconsistent ways | **Refuted (mostly)** — god-classes fan_out *intentionally* counts external (npm/pypi/std) imports (documented as "total dependency breadth"), while crossing/instability measure *internal* coupling. That divergence is by design. The only genuine gap is crossing-vs-instability self-loop handling (a rare re-export edge case) — low value, output-changing, needs a semantics decision. Not the broad "three different numbers" defect the finding implied. |
| F229 | Vendored `libduckdb-sys` fork (duckdb-rs#786) | **Fixed (merged to main via #71 — full CI matrix green incl. `test (windows-latest)`)** — bumped `duckdb → =1.10504.0` (upstream #786 fix; the released `build.rs:66` emits `rustc-link-lib=dylib=rstrtmgr`) and removed the whole vendoring apparatus: the `[patch.crates-io]` block, `vendor/duckdb-rs/` + tracked stubs, `scripts/vendor-duckdb-rs.sh`, `patches/duckdb-rs-msvc-1940.patch`, the `.gitignore` stub-path handling, and the "Vendor patched libduckdb-sys" step at all 9 sites across ci/release/container/bench. No arrow-version conflict; a version-drift guard caught + forced `provenance::DUCKDB_VERSION` + banner test refs to `1.10504.0`. The MSVC build (the whole reason the fork existed) is confirmed green on the Windows runner. |

### Refuted findings (preserved to prevent rediscovery)

F84, F88 (silent ODB skip rationale), F95 (window filter at ingest level), F116 (Renovate/Dependabot partitioned by ecosystem — Renovate owns `cargo`, Dependabot owns `github-actions`), F123 (crossbeam 0.8.4 + num-format 0.4.4 are current releases), plus the earlier-report set: apply_grouping JOIN shape, renderHeader listener leak, parquet/SQLite backslash escape, hotspots CTE leak, color-mode aria-label, Kamei SEXP `<` vs `<=`, tree-sitter `kind_id` ABI, AI-assist false positives, NULL-conflated AI attribution, DuckDB pinning speculation, code-health weights citation, SoC inclusive thresholds. Rationale in `f1aa0e7` (PR #36) + `13fefcb` (PR #38).

---

## 4. Active Findings

**Reading the locations below.** File and line references are as-of-discovery
and several predate the module splits that broke `main.rs`, `output/csv.rs`
and `output/markdown.rs` into directories — a line number here may point
nowhere. The finding is identified by the symbol and behaviour described, not
by its coordinates; re-locate before acting. Counts and constant *values* are
a different matter — those are claims, and where a re-validation found one
wrong it has been corrected in place rather than left to mislead.

### 4.1 Carried forward from prior passes (re-validated 2026-07-01)

#### F173 — Same HEAD blobs read + tree re-walked up to 3× across complexity/clones/imports
*   **Location**: `facts/ingest/mod.rs:145,158,165` (three sequential passes); each `*_head.rs:55` independently calls `read_blob_at_head`
*   **Severity**: HIGH · **Category**: performance (redundant I/O) · **Status**: Active
*   **State on main**: Still three sequential HEAD passes each re-reading live blobs. Only `head_rev`/`live_paths` were hoisted once (SQL path-queries no longer repeated); blob reads + tree walks still happen 3×. Deepened by the newly-found per-file re-resolution cost in **F206** — even a single deduped pass keeps paying F206's per-file HEAD/commit/tree decode.
*   **Deferral blocker**: divergent extractor error contracts (clones aborts ingest via `collect::<Result>>?`; complexity/imports warn-and-skip) + the memory-regression risk of hoisting all live blobs into one map. Needs a bounded shared-blob LRU or unified error contracts first.

#### F119 — Hand-rolled CSV emitter (now 1692 LOC) instead of the `csv` crate
*   **Location**: `output/csv/` — a seven-file module since the split; no longer the single `output/csv.rs` this finding was written against
*   **Severity**: MED · **Category**: tool replacement · **Status**: Active (re-scoped)
*   **State on main**: Still hand-rolled (1692 LOC across the module, up from 1122; no `csv` dep). **Re-scope note**: no longer a clean byte-identical swap — the emitter now carries a deliberate formula-injection guard (F170) and `\n` line endings; the `csv` crate would change both. Any migration must preserve the injection guard + line-ending contract, or the swap is rejected.

#### F148 — `output/csv` + `output/markdown` per-analysis emitters, no shared row abstraction
*   **Location**: `output/csv/` (1692 LOC, 58 `write_*` fns), `output/markdown/` (1863 LOC, 58 `write_*` fns)
*   **Severity**: LOW · **Category**: copy-paste drift · **Status**: Active
*   **State on main**: Both were since split from single files into seven-file modules, which addressed the file-size symptom and not the finding: still one `write_*` fn per analysis on each side, in lockstep at 58 apiece (up from 43), with no `TabularEmit`/row trait. The parallel counts are the finding — every analysis added costs two near-identical emitters. Coupled to F119 (csv-crate) + F161 (streaming) — treat as one output-layer cluster.

#### F161 — Every emitter materializes the full `Vec<Row>` — no streaming path
*   **Location**: `output/json.rs:29`, `sarif.rs:90`, `markdown.rs` — all `rows: &[T]`
*   **Severity**: LOW · **Category**: memory architecture · **Status**: Active
*   **State on main**: All emitters still take a fully-materialized slice; no `EmitterStream`. Peak memory grows with row count; a 200k-path monorepo CSV export can spike multi-GB. SARIF stays batch (needs run-level totals); CSV/JSON/markdown are the streamable targets.

#### F177 — Three schema-version sentinels still coexist
*   **Location**: `facts/schema.rs` (`CURRENT_SCHEMA_VERSION`, now `"8"`), `cache.rs` (`CACHE_EPOCH`, now `"schema_v21"`), `schema_v1.sql` filename literal reached through `facts::schema::SCHEMA_V1`
*   **Severity**: MED · **Category**: duplicated source-of-truth · **Status**: PARTIAL
*   **State on main**: Both named sub-fixes landed — CLI `profile` now derives the schema string from `CURRENT_SCHEMA_VERSION`, and the cache sentinel was renamed to the honest `CACHE_EPOCH` (matches CLAUDE.md). But three version constants remain structurally disjoint (none derived from another), and they have drifted independently since — `CURRENT_SCHEMA_VERSION` is now `"8"` while `CACHE_EPOCH` reads `"schema_v21"`, two sentinels whose shared `schema_v` spelling implies a correspondence that does not exist. The stray `"schema_v3"` help literal is **gone** (0 occurrences), so that half of the residual is closed. Residual: unify or cross-reference the three, or rename `CACHE_EPOCH`'s value so it stops looking like a schema version.

#### F186 — Bench regression gate never runs on PRs (advisory-only weekly cron)
*   **Location**: `.github/workflows/bench.yml:3` (`schedule` + `workflow_dispatch`, no `pull_request`), `:116` (`fail-on-alert: false`)
*   **Severity**: MED · **Category**: CI coverage / design tradeoff · **Status**: Active (design decision)
*   **State on main**: Unchanged and explicitly documented as intentional post-merge advisory behavior. Kept as a design-review item, not a plain bug: a perf regression can merge unflagged until the Monday cron. Decision point — leave advisory, or add a non-gating PR-triggered bench comment.

#### F197 — `dogfood` job: per-PR `--release` build, advisory-only, separate cache
*   **Location**: `.github/workflows/ci.yml:210` (`continue-on-error: true`), `:231` (`shared-key: release-dogfood`), `:235` (`cargo build --release`)
*   **Severity**: LOW · **Category**: CI cost · **Status**: PARTIAL
*   **State on main**: The "cold" aspect is mitigated (sccache warms release objects cross-workflow), but the job is still a per-PR `--release` build, advisory-only, on a deliberately separate cache slot. Residual (deliberate bake-in): decide when to gate + whether to share the CI cache slot.

---

### 4.2 Discovery pass — 2026-07-01 (deferred remainder)

The 5-dimension fan-out logged F200–F230; 25 landed in the 2026-07-01 pass and F200 (+ F188/F202 refutations) in the 2026-07-02 pass (see the §3 "Implemented" tables). The entries below are the deferred remainder — each is a large refactor with regression surface, a dependency-migration needing CI validation, or a low-value mechanical sweep, not a quick safe change. Since then F206 (HEAD-scan blob I/O) and F230 (gix bump) have shipped — marked Fixed inline below — and F215 was closed, not fixed (see its entry); F218 is the open remainder.

#### Backend performance

##### F206 — `read_blob_at` re-resolves HEAD→commit→root-tree per file and discards the gix object cache each call
*   **Location**: `repo/gix_repo.rs:293-329` (`to_thread_local()` per call → `rev_parse_single` → `find_commit` → `commit.tree()` → `lookup_entry_by_path`); default wrapper `repo/mod.rs:99-101`
*   **Severity**: HIGH · **Category**: blob I/O / redundant recomputation · **Status**: **Fixed (v0.25.0)**
*   **Description**: Every HEAD-time blob read mints a fresh thread-local `Repository` (cold object cache), re-resolves `HEAD`, re-decodes the commit + root tree, and re-walks + re-decodes every intermediate directory tree — for *each* file. A file at depth `d` re-decodes `d` tree objects; every sibling re-decodes its parent tree again. Three HEAD passes × F live files = 3F redundant resolves. This is distinct from and **deeper than** F173 (which only dedups the blob across passes — the per-file HEAD/commit/tree re-decode remains even in one deduped pass). Dominant cost of HEAD scans on large deep-nested monorepos.
*   **Suggested improvement**: Resolve HEAD → root tree once per pass and reuse it (batch `read_blobs_at_head(paths)` walking a single cached tree), or hold one `to_thread_local()` repo with `object_cache_size` enabled across the file loop. Same bytes returned — output-neutral, faster.
*   **Outcome (v0.25.0)**: shipped as this finding's suggested improvement — `Repo::blob_reader_at(rev)` returns a `BlobReader` whose `read(path)` is byte-identical to `read_blob_at`, and `GixRepo` overrides it (`repo/gix_repo/mod.rs`) to resolve the root tree once per `rayon` worker (via the existing `map_init` idiom) and reuse a warm `gix` object cache for every file that worker subsequently reads. The differential oracle (`GitCliRepo`) uses the default per-call forwarder, so the two-backend parity is unchanged (byte-identical ingested facts proven before/after). Landed with the F173/F253 Phase-1 HEAD-scan work; the remaining three-pass blob dedup (F173) stays open.

#### Rust idioms / error handling

##### F215 — Stringly-typed `format: &str` re-matched with `unreachable!()`
*   **Location**: `codelore-cli/src/analyze.rs` — one `unreachable!("format validated by outer matches!()")` arm
*   **Severity**: LOW · **Category**: type-safety / simplification (optional) · **Status**: Closed, not fixed — the §13 cluster note is authoritative: exactly one `unreachable!` site remains, small enough that `enum Format` no longer carries its own weight; F244 absorbs any future registry-level version
*   **Description**: `--format` is validated once then re-matched in dispatch, carrying an `unreachable!("format validated…")` arm — a hand-maintained invariant a parse-once `enum Format` would make compile-time-total.
*   **Re-validated**: the finding was written against `main.rs` when it was a ~6700-line monolith and claimed ~11 such dispatchers. The monolith split dissolved most of that: **exactly one** `unreachable!` remains in the whole CLI crate. What is left is a one-site cleanup, not the cross-cutting refactor this entry was deferred as — and small enough that the `enum Format` argument no longer carries its own weight. Re-scope or close.

#### SPA / UI / UX

##### F218 — Any single layout-selector change re-renders every widget (full-dashboard cascade)
*   **Location**: `output/spa/template.html` (one `Alpine.effect` subscribing to all layout knobs → all `_codeloreRerenderers`)
*   **Severity**: MED-HIGH · **Category**: render performance · **Status**: Partially Fixed (v0.27.1)
*   **Description**: Bumping the Kamei window 30→60 (one sparkline) re-runs `d3.pack` over the whole hotspot tree, rebuilds every ECharts instance, and re-lays-out the arch graph + DSM. The code yields between rerenderers to stay responsive — treating the symptom. The scenario toggle also auto-clicks the knowledge-loss tab, double-rendering the circle-pack on the first pick.
*   **Partial fix**: Split the monolithic `Alpine.effect` into (a) a pure theme effect that reads only `store.theme.isDark` and fires `_codeloreRerenderers` with cooperative yield (F135), and (b) a separate layout/offboarding effect that reads `store.layout.*` and `store.scenario.departed` — so theme toggles no longer chain through layout/scenario subscriptions or trigger the CSS-token invalidation pass from unrelated clicks. The cross-widget selection and brush effects were already separate.
*   **Residual (open)**: layout/scenario changes still fire ALL registered rerenderers — the headline Kamei-window scenario above is unchanged. Remaining follow-on: key the rerenderer registry by the store fields each widget depends on, so a layout change re-renders only its subscribers.

#### Dependency currency (verify latest before acting — assessed offline from declared/resolved versions)

Overall hygiene is strong: `thiserror 2`, `toml 1`, `ureq 3`, `anyhow`/`serde`/`clap`/`rayon`/`time`/`percent-encoding` all current. `tree-sitter*` + `petgraph` are deliberately pinned (CLAUDE.md) — out of scope. Two items worth active tracking:

##### F230 — `gix` 0.84 → 0.85 bump
*   **Location**: `crates/codelore-lib/Cargo.toml`
*   **Severity**: LOW · **Category**: dependency currency (routine) · **Status**: **Fixed (merged via #74)**
*   **Outcome (2026-07-02)**: bumped `gix 0.84 → 0.85` (latest, published 2026-06-22) with `provenance::GIX_VERSION` + banner refs. API-compatible — the two-backend differential harness (`differential_repo_test.rs`) passed unchanged, so `GixRepo` still matches the `git`-CLI oracle. Consolidated Dependabot #68 (whose only failure was the drift guard) into #74; full CI matrix green. Related closed dep PRs: #69 (arrow 58→59 — deferred, would desync from duckdb's pinned arrow 58), #70 (duckdb group — superseded by F229).

---

## 5. Recent closure logs (2026-07-02 → 2026-07-04)

The dated sub-sections below record closed findings (Fixed / Refuted) from the
2026-07-02 → 2026-07-04 passes, kept out of §4 so *Active Findings* lists only
open work. Exception: §5.4's 2026-07-04 pass also logs two still-open own-slice
follow-ups (F244, F246), carried in the next-cycle backlog (§6).

### 5.1 Code-health composite score — design observation (2026-07-02)

#### F232 — Coupling centrality counted twice in the composite code-health score

*   **Location**: `analyses/code_health.rs` — `SHOTGUN_INSERT` (reads `coupling_centrality_v1`, writes `shotgun-surgery` biomarker) + `normalized` CTE `n_cp` term (also reads `coupling_centrality_v1` directly)
*   **Severity**: LOW · **Category**: scoring design / weight calibration · **Status**: Fixed (v0.27.1)
*   **State on main**: `coupling_centrality_v1` previously fed the composite score via two independent paths: (1) the `shotgun-surgery` biomarker (`intensity = PERCENT_RANK(ORDER BY centrality)`) flowing into `structural_risk`; and (2) directly as `n_cp = normalize(centrality)`. A high-centrality file was penalised through both at once — a double-count of the same signal.
*   **Fix**: the `n_cp` term and its centrality join were removed from the score SQL; coupling now enters the composite once, as the shotgun-surgery biomarker inside `structural_risk`. Score reweighted to `w_sr = 0.50, w_cn = 0.30, w_au = 0.20` (sum 1.0). `CACHE_EPOCH` bumped.

### 5.2 Refactoring-targets analysis — cross-analysis contract and display fidelity (2026-07-03)

#### F233 — Implicit cross-analysis contract: `refactoring-targets` consumes `code_health_biomarkers_v1` temp table as a side-effect

*   **Location**: `analyses/refactoring_targets.rs` (dominant-biomarker lookup query reads `code_health_biomarkers_v1`); `analyses/code_health.rs` (`materialize_biomarkers` creates the table as a side effect of `run_code_health`)
*   **Severity**: MED · **Category**: implicit contract / latent-robustness · **Status**: Fixed (v0.27.1)
*   **Description**: `refactoring-targets` is the first external consumer of the `code_health_biomarkers_v1` temporary table, which `run_code_health` materialises as a side effect via `materialize_biomarkers`. This elevates a private implementation detail of `code-health` into an implicit cross-analysis contract: if `code_health` dropped the table before returning, or the call order in `run_refactoring_targets` changed so the biomarker query ran before `run_code_health`, the dominant-biomarker lookup would fail at runtime with a DuckDB "table not found" error.
*   **Fix**: the `BIOMARKERS_DDL` in `analyses/code_health.rs` now carries a doc-comment documenting the external consumer and the session-scoped contract (must stay session-scoped and readable after `run_code_health` returns; columns not renamed without updating the consumer). The two `refactoring_targets` integration tests exercise the full path and guard a runtime regression.

#### F234 — `loc` display floor: files with no LOC entry show `loc = 1` (fabricated value)

*   **Location**: `analyses/refactoring_targets.rs` — `loc_by_path.get(...).unwrap_or(0).max(1)` used for both the displayed `loc` column and the priority denominator
*   **Severity**: LOW · **Category**: display fidelity / cosmetic · **Status**: Fixed (v0.27.1)
*   **Description**: A file with no non-NULL LOC entry (e.g. a file the complexity walker skipped) had its `loc` stored as `0.max(1) = 1` — a fabricated value propagated directly into the output row. The `EA_Z_FLOOR` (25) already floors the priority denominator independently, so the `.max(1)` affected only the displayed `loc`.
*   **Fix**: `refactoring-targets` now reports the true `loc` (`0` = no LOC data); the EA-Z effort floor is confined to the priority denominator (`max(loc, EA_Z_FLOOR)`). The integration-test assertion was updated accordingly.

#### F235 — `structural_risk` saturated at the ceiling on real repositories

*   **Location**: `analyses/code_health.rs` — `BIOMARKERS_INSERT` (`ranked`/MAX rollup) + `file_structural` (probabilistic-OR aggregate)
*   **Severity**: HIGH · **Category**: metric quality / scoring · **Status**: Fixed (v0.27.1)
*   **Description**: Empirical run on this repo showed **67 of 69 files at `structural_risk = 1.0000` and 100% `red` band** — the metric did not discriminate. Two compounding causes: (1) `BIOMARKERS_INSERT` ranked FUNCTIONS by `PERCENT_RANK` then took the per-file `MAX`, so any file with enough functions had one in the top percentile → intensity ≈ 1.0; (2) `file_structural` combined intensities with a probabilistic-OR (`1 − Π(1−intensity)`) × co-occurrence multiplier, which drove any file with ≥2 high smells to the `LEAST(1.0, …)` clamp. The per-row invariant tests (range/monotonicity/determinism) all passed while the distribution was degenerate.
*   **Fix**: (1) rank FILES not functions — aggregate to the file first (`MAX` per file), then `PERCENT_RANK` across files, so each smell's intensity is uniformly spread; (2) replace the probabilistic-OR with a bounded weighted sum (per-smell weights summing to 1.0, absent smells contribute 0, co-occurrence implicit). Empirical result on this repo: `structural_risk` spreads `0.01–0.96`, band split **8 red / 32 yellow / 31 green** (thresholds `0.55 / 0.28`). Added a distribution regression test (`code_health_structural_risk_discriminates`) on a purpose-built `biomarker_repo` fixture. Residual (Phase-2): small per-language cohorts (e.g. a handful of JS files) make `PERCENT_RANK` coarse — the cross-repo corpus percentile addresses this.

#### F236 — biomarker normalization inconsistency: god-class/dry used min-max while others used percentile

*   **Location**: `analyses/code_health.rs` — `materialize_biomarkers` (god-class + dry insertion)
*   **Severity**: MED · **Category**: metric quality / scoring · **Status**: Fixed (v0.27.1)
*   **Description**: After F235, three biomarkers (complex-method, large-method, shotgun-surgery) used per-file `PERCENT_RANK` but god-class and dry still used min-max `/max` — an outlier-dominated scheme that under-discriminated (one extreme god class compresses the rest toward 0). Naively switching them to `PERCENT_RANK` *among files that have the smell* was worse (a lone or tied occurrence — e.g. two files each with one clone — collapses to a 0 rank, losing the DRY signal entirely; verified on the fixture).
*   **Fix**: god-class and dry now rank their raw metric over the FULL per-language file universe (absent files contributing 0), matching complex/large's scheme exactly (same `cyclomatic IS NOT NULL AND loc IS NOT NULL` filter). A file with the smell ranks above the zero-majority (e.g. a tied duplicated pair → ~0.8, not 0 or a min-max 1.0). (shotgun-surgery keeps its own pre-existing universe — the coupled-file set from `coupling_centrality_v1`, not language-partitioned; unifying that too is a separate follow-up.) Band thresholds retuned `0.50/0.25 → 0.55/0.28` to keep the red set selective after god-class intensities rose. Added `biomarker_repo` fixture + tests: `code_health_structural_risk_discriminates`, `code_health_biomarkers_fire_distinct_smells` (closes the T2-M2 dropped-`UNION`-arm gap), and `refactoring_targets_dominant_type_varies_on_biomarker_repo` (closes the P2-T2-M1 gap). `CACHE_EPOCH → schema_v8`.

#### F237 — Phase 1/2 deep-validation audit fixes

*   **Location**: `main.rs` (explain, check gate, refactoring-targets dispatch), `analyses/code_health.rs` (module doc), `analyses/refactoring_targets.rs` (dominant-type query), `quality_gates/mod.rs`, tests
*   **Severity**: HIGH (aggregate) · **Category**: correctness / consistency / test coverage · **Status**: Fixed (v0.27.1)
*   **Description**: A five-stream adversarial audit of the Phase-1/2 changes confirmed the core computations correct (bounded, NaN-free, deterministic) but surfaced: (a) HIGH — `codelore explain code-health` printed the pre-F232 formula (`0.40/0.25/0.15` + a separate coupling term, `0.66/0.33` bands); (b) HIGH — the `check` gate `code_health_min` evaluated the hotspots inline cognitive-only proxy (floored 60), not the composite, so a red file (composite ~20) silently passed a `code_health_min = 70` gate; (c) MED — `refactoring-targets` claimed `html` support but its dispatch returned `html_not_wired`; (d) LOW — `dominant_type` reported a biomarker at intensity 0 (should be "none"); (e) LOW — the module doc overstated per-language-percentile uniformity (shotgun excepted); plus several weak/tautological tests and untested contracts (weights, band thresholds, code-health CSV columns).
*   **Fix**: (a) explain tuple updated to the current formula/weights/thresholds; (b) `code_health_min` rewired to the composite via a new `evaluate_code_health_gate(&[CodeHealthRow])` (breaking change, noted in CHANGELOG); (c) `html` wired via the generic `write_html`; (d) `WHERE intensity > 0` added to the dominant-type query; (e) module doc scoped. Tests: reframed the architecturally-invalid `structural_risk_rewards_multiple_cooccurring_smells`, hardened the silent-skip in `code_health_penalizes_churn`, removed the tautological `god_class_and_dry_are_biomarkers`, fixed the `min_by_key(loc)` tie in the ManualUp test, and added `code_health_csv_column_contract` + `code_health_band_matches_thresholds`. Residual (deferred): exact-weight assertion (needs exposing `n_cn`/`n_au`), god-class biomarker firing (needs a fan-in fixture), and churn/ownership global-vs-per-language normalization (a Phase-2 tuning consideration).

### 5.3 SPA linked-brushing — publish-side + subscriber deep validation (2026-07-03)

A post-slice deep validation of Plan 3b (four selection subscribers) ran three parallel source-level validators (subscribe correctness / publish completeness / test integrity) plus a validation-spec pass. The subscribe side was confirmed fully correct; the findings below are on the publish side and two subscriber edge cases. F239–F241 were fixed together in `0a41b1d`; F238 — initially expected to need a design decision — was also fixed, via a direct per-item `lineStyle` restyle (`dd9cfad`).

#### F238 — parallel-coords cross-widget highlight is visually inert (`emphasis.disabled`)

*   **Location**: `output/spa/widgets.js` — parallel series config (`emphasis: { disabled: true }`) + the `parallel-coords` selection subscriber
*   **Severity**: LOW · **Category**: UX / linked-brushing fidelity · **Status**: Fixed (v0.27.1)
*   **Description**: The pre-existing `parallel-coords` subscriber calls ECharts `highlight`/`downplay` on selection, but the parallel series sets `emphasis: { disabled: true }`, so those actions have NO visible effect — a file selected in another widget does not visibly stand out in the parallel-coordinates plot. The subscriber is wired but inert.
*   **Fix (`dd9cfad`)**: investigation confirmed `emphasis: { disabled: true }` is a LOAD-BEARING ECharts-6 regression workaround (the hovered polyline disappears under emphasis) — so re-enabling emphasis was NOT an option. Instead the subscriber now restyles the selected line's per-item `lineStyle` directly (bold `--color-info`, width 3, opacity 1; the rest fade to opacity 0.12; a null selection restores the default `--color-warning`), extracting the series data into `parallelData` once so it can mutate + re-apply without a rebuild. No emphasis transform, so the regression stays avoided. Setting every item each fire means no stale A→B highlight.

#### F239 — `trends` subscriber leaves a stale A→B highlight

*   **Location**: `output/spa/widgets.js` — `trends` selection subscriber
*   **Severity**: MED · **Category**: correctness / linked-brushing · **Status**: Fixed (v0.27.1)
*   **Description**: ECharts `dispatchAction({type:'highlight'})` is additive — it does not implicitly clear a prior highlight. The `trends` subscriber highlighted the selected series without downplaying first, and because the file-detail drawer is NON-modal (`.show()`, not `.showModal()`), a user can switch selection directly from file A to file B with no intervening null-clear. Result: both A's and B's trend lines stayed bold+un-blurred — a stale highlight of the previous file. (The `coupling`/`dsm` subscribers added in the slice already downplayed-first; `trends` and `parallel-coords` did not.)
*   **Fix (`0a41b1d`)**: `trends` now `downplay`s unconditionally first, then early-returns on null, then highlights only on a match — matching the coupling/dsm shape. (`parallel-coords` was validated benign here — its `emphasis.disabled` means no highlight state persists; see F238.)

#### F240 — linked-brushing publish side was asymmetric (receive-only widgets)

*   **Location**: `output/spa/widgets.js` — click handlers for the circle-pack map, coupling sankey, treemap, X-Ray sunburst
*   **Severity**: MED · **Category**: feature completeness · **Status**: Fixed (v0.27.1)
*   **Description**: "Select a file in ANY widget → highlight everywhere" needs both a publish and a subscribe half. Plan 3b completed the subscribe half but the publish half was asymmetric: only four surfaces (hotspot table, parallel-coords, KI-table, keyboard treeview) routed clicks through `_codeloreShowDetail` (which publishes `selection.set`); the map canvas, sankey, treemap, and X-Ray sunburst called `showFileDetailDrawer` DIRECTLY — opening the drawer without broadcasting. So the coupling sankey, DSM, trends, and the map canvas were effectively receive-only.
*   **Fix (`0a41b1d`)**: the four direct-drawer file-clicks now route through the guarded `_codeloreShowDetail` idiom (sankey gated to files mode — a module-depth node name is a prefix, not a file, and must not go on the file-level bus). The map click drops its redundant direct `selectedCouplingFile`/`updateCouplingArcs()` in the broadcast branch (the `hotspot-map` subscriber does it on fan-out). Validated NOT-VALID and intentionally excluded: the DSM as a publish source — its axes/cells are module-level, so a click cannot identify a single file for the file-level bus. Follow-on (`27d666b`): the map's canvas background-click now clears the shared selection (bus) instead of only its local arcs, so deselection is symmetric with the new publish-on-select.

#### F241 — coupling-sankey subscriber highlight dead in module-depth view

*   **Location**: `output/spa/widgets.js` — `coupling` selection subscriber
*   **Severity**: LOW · **Category**: linked-brushing fidelity · **Status**: Fixed (v0.27.1)
*   **Description**: The `coupling` subscriber highlighted the sankey node by the raw full path (`name: selectedPath`). In files mode (default) node names ARE full paths so it matched, but in module-depth view nodes are named by truncated `modulePathSeg` prefixes, so the highlight silently no-op'd — the sankey did not participate in cross-widget selection at non-file depths.
*   **Fix (`0a41b1d`)**: the subscriber now maps the bus path into the current node-name space (`modulePathSeg(selectedPath, userSankeyDepth)` when depth is numeric, else the full path), mirroring the DSM subscriber's module-mapping. Also recorded as a validated non-issue: a proposed DSM "empty-indices" guard is dead code — the per-index diagonal guide cells guarantee the scan always yields ≥1 index.

#### F242 — module-depth coupling-subscriber browser test is inert against the differential fixture

*   **Location**: `crates/codelore-lib/tests/spa_browser_test.rs` — Step 13 in `rendered_spa_boots_without_console_errors`
*   **Severity**: LOW · **Category**: test coverage · **Status**: Fixed (v0.27.1)
*   **Description**: The original Step 13 (inside `rendered_spa_boots_without_console_errors`) asserted the coupling subscriber highlights a selected file's `modulePathSeg(path, 2)` module prefix in module-depth sankey view. The production mapping is correct (verified by source review), but that test's only fixture — `differential_repo::build()` — has near-zero co-changes, so at depth 2 the change-coupling sankey had no cross-module links and no qualifying node; the step always SKIPPED. Net: the guard executed no assertion in CI and provided zero live regression protection, and it spun a ~3s re-render poll to no effect on every run.
*   **Fix (`373747e`, `9030159`)**: added a dedicated `coupling_repo` fixture (`test_support/mod.rs`) — three 2-segment modules (`src/alpha`, `src/beta`, `src/gamma`), with `alpha/svc.rs`↔`beta/svc.rs` co-changed across 6 commits so a `src/alpha`↔`src/beta` depth-2 edge is guaranteed under any coupling threshold, plus per-file solo churn for hotspot rows — and moved the assertion into its own test `sankey_module_depth_highlights_mapped_node` rendered from that fixture with `permissive_coupling_opts`. The inert Step 13 was removed from the smoke test. The new test FAILS (not skips) if the depth-2 sankey has no qualifying node, and asserts the captured highlight name equals the module prefix (not the raw path). Independently verified: `spa_browser_test` 9/9 (was 8), the new test exercises its assert (full-boot run, ~9.3s), `spa_integration_test` 4/4.

### 5.4 Whole-codebase architecture review + hygiene pass (2026-07-04)

A five-dimension architecture review (four parallel read-only analysts: architecture/boundaries, coding-structure/patterns, performance, SPA; plus two validation-and-spec passes) surveyed the engine + SPA for improvement leverage. Headline: the codebase is genuinely well-built — error handling exemplary (3 non-test `unwrap`s, all justified), the `Repo` two-backend abstraction + `!Send` ingest pipeline + cross-crate boundaries clean, CSV-injection closed, test quality high. The real structural debt concentrates in one place: the analysis→output **dispatch fan-out** (43 stringly-typed dispatchers × per-format arms + 43+43 tabular emitter fns). The low-risk validated wins were fixed this pass; the large refactors are logged own-slice below.

#### F243 — `html` output support un-advertised in 4 dispatchers (stringly-typed drift)

*   **Location**: `codelore-cli/src/main.rs` — `dispatch_authors`/`dispatch_top_committers`/`dispatch_knowledge_islands`/`dispatch_clone_coupling`
*   **Severity**: LOW · **Category**: correctness / user-facing message · **Status**: Fixed (v0.27.1)
*   **Description**: Each of these 4 dispatchers has a working `"html" => write_html(...)` arm, but its `unsupported_format(...)` error message advertised a format list OMITTING `html` (e.g. `"csv|json|markdown"`), so a user passing an invalid `--format` was told html isn't supported when it is. Same class as F237(c). This is a live symptom of the stringly-typed dispatch (the advertised list is a hand-maintained string parallel to the actual `match` arms — F215).
*   **Fix (`acd9568`)**: added `html` (and `sarif` where applicable) to the 4 advertised strings. Byte-identical for every success path — only the error text for an unsupported format changed. This is a symptom patch; the root cause (parallel hand-maintained format lists) is F215, logged own-slice below.

#### F231 — `Plan N` phase markers in code — now Fixed via a self-enforcing guard

*   **Status**: **Fixed (v0.27.1)** (`52c427c` + this-pass residual close) — previously Active/deferred.
*   **Fix**: rather than the deferred one-off scripted sweep, the existing `comment_hygiene_test.rs` was extended with a `no_plan_phase_markers_in_code_comments` test (a `Plan`+digit whole-comment scan, sibling to the `F<NN>` guard), then all **69** existing markers were scrubbed — the majority stripped (parenthetical/provenance tags), ~18 stale future-tense/current-claim comments rewritten to present state (verified against source: single-producer ingest, Type 1/2 clones, all output formats shipped, `gix` default + `GitCliRepo` oracle). Test + fixes landed atomically; the guard makes the convention self-enforcing forever (strictly better than a sweep that can silently regress).
*   **Residual close (this pass)**: the guard initially scanned only `.rs` comment regions, so `Plan N` markers in `.sql` DDL (`facts/schema_v1.sql`) and in user-facing string literals (`analyze.rs`'s SARIF / parquet `bail!` messages, including a multi-line-string continuation) still slipped through. The `Plan`-marker check now scans the **whole line** across `.rs` **and** `.sql` (renamed `no_plan_phase_markers_in_code`), catching comment / string-literal / DDL markers alike; the five residual markers were scrubbed to current-state wording. The `F<NN>` check deliberately stays comment-scoped — bare `F<NN>` tokens appear legitimately in test fixtures and assertion labels, a separate broader hygiene item logged as F269.

#### Other validated DO-NOW improvements landed this pass

*   **Clippy `#[allow]` justification** (`356efc9`) — 19 previously-unjustified `#[allow(clippy::…)]` sites gained a true per-site technical reason (Golden Rule #14). The tempting "consolidate casts to a workspace allow" was validated and REJECTED — it would disable the lint repo-wide.
*   **SPA listener-bus unification** (`1d645af`) — the two byte-identical registries (`selection` + `brush`) collapsed to a `makeListenerBus(arrayName)` factory (behavior-preserving; browser tests green).
*   **SPA browser-test coverage** (`e4ec986`) — the main browser test's dashboard fixture was broadened so previously-dark widget render branches (arch-trend, fusion overlays, MI tile, ownership/clones colour maps) now execute under the no-console-error / exception gate. No latent render bug surfaced.
*   **F245 — SPA `widgets.js` build-time module split** — the 4588-line single IIFE was split into seven ordered `spa/js/*.js` source modules concatenated at compile time via `concat!` into the `{{WIDGETS_JS}}` placeholder in `spa.rs`, fitting the offline-single-file constraint (no build step, one output file). Byte-identical assembly proven via `cmp`; full browser suite green; files named to match their contents. (Was logged own-slice; landed this pass.)

#### Logged own-slice (validated REAL, each warrants its own byte-identical/benchmark-gated slice)

*   **F244 — Central analysis registry / `enum Format` + `TabularRow` (root cause behind the dispatch fan-out).** Refines/absorbs **F215** (stringly-typed dispatch), **F148** (no shared tabular-row trait; 88 csv/markdown `write_*` fns), **F119** (hand-rolled CSV). Validated true scope: 43 dispatchers + ~137 match arms; a clap `ValueEnum` `Format` is the minimal first increment (deletes the hand-maintained master-list `match`, adds parse-time validation + completions) — but carries an exit-2-vs-exit-4 contract delta for invalid `--format` and does NOT alone fix the per-dispatcher advertised-string drift (needs a `supported_formats()` source of truth). `main.rs` (3283 LOC) splits into `commands/`+`dispatch/` after the enum lands. L, byte-identical-gated. Sequence AHEAD of the output-emitter cluster.
*   **F246 — SPA canvas-chart keyboard operability.** The arch-graph/DSM/module-chord/X-Ray/treemap have `role="img"`+`aria-label` but no keyboard-navigable data equivalent (only the circle-pack has its `role="tree"`). Largest remaining a11y gap; scope to the 1-2 highest-value widgets. M.
*   **Still tracked:** F173 (the remaining HEAD-scan lever — deduping the blob across the three sequential passes, benchmark-gated; F206's per-worker warm reader shipped in v0.25.0, see §4.2), F218 residual (per-widget layout-change routing — the theme-path split landed; see the finding).

#### Validated and REJECTED (do NOT act)

*   **Domain newtypes (Golden Rule #15)** — low Rust ROI: values are string-shaped from DuckDB straight into emitters with no cross-type mixup risk; the one primitive-confusion bug (path vs module-prefix, F240/F241) was JS-side. Accepted deviation.
*   **`Options` builder** — already rejected in the roadmap; its cross-field-validation value is already delivered by `Options::validate()`.

---

## 6. Next Audit Cycle

**State after the 2026-07-01 + 2026-07-02 implementation sessions (all merged to `main`):**

- **Implemented + merged (28)**: 2026-07-01 (25) — F191, F201, F203, F204, F205, F207–F214, F216, F217, F219–F228 (PR #71); 2026-07-02 (3) — **F200** (deleted the divergent+vacuous `commit_metadata` + `CommitMetadata`, kept the real `changed_files`/`diff_hunks` cross-checks; #71), **F229** (dropped the vendored `libduckdb-sys` fork; `duckdb → =1.10504.0`; #71), **F230** (`gix 0.84 → 0.85`; #74). Full CI matrix green on every merge, including `test (windows-latest)`.
- **Refuted on validation (2026-07-02)**: F188 (ruleset omission is intentional + drift-guarded), F202 (fan-out divergence is mostly by design — god-classes externals vs internal coupling).
- **Deferred — large refactor / focused pass**: F206 (HEAD-scan I/O restructure — wants a benchmark), F215 (`enum Format`), F218 residual (per-widget layout routing; the theme/layout effect split is Partially Fixed above), F231 (62-site `Plan N` scripted sweep).
- **Carried-forward Active (output/blob cluster)**: F119 (csv-crate), F148 (`TabularEmit` dedup), F161 (`EmitterStream`), F173 (HEAD blob dedup) — byte-identical-critical (F206 is the deeper lever for F173).
- **Carried-forward Partial / design**: F177 (schema sentinels), F186 (bench PR gate — design), F197 (dogfood advisory/separate-cache).
- **Fixed (v0.27.1) 2026-07-03**: F232 (coupling double-count — `n_cp` removed, score reweighted), F233 (`code_health_biomarkers_v1` cross-analysis contract — documented at the DDL), F234 (`loc` display floor — reports true value), **F235 (`structural_risk` saturation — rank files not functions + weighted sum; 67/69-at-1.0 → 8/32/31 band split)**, **F236 (biomarker normalization unified on full-universe per-file percentile; `biomarker_repo` fixture + distribution/vocabulary/dominant-type tests)**, **F237 (deep-validation audit: stale explain, check-gate composite rewiring, refactoring-targets html, dominant-type intensity>0, test hardening)**, **F239 (trends A→B stale highlight — downplay-first)**, **F240 (SPA linked-brushing publish symmetry — map/sankey/treemap/X-Ray now broadcast)**, **F241 (coupling-sankey highlight fires in module-depth view)** (all `0a41b1d`), **F238 (parallel-coords highlight made visible via a direct per-item `lineStyle` restyle — `dd9cfad`)**.

**Highest-leverage work remaining:**
1. **HEAD-scan I/O** (F173) — F206's per-worker warm reader (resolve HEAD→tree once per worker) shipped in v0.25.0; the remaining lever is deduping the blob across the three sequential HEAD passes (F173), benchmark via `ingest_capacity_sweep`. Biggest large-repo wall-clock lever.
2. **Output-emitter cluster** (F119 / F148 / F161) — csv-crate migration (preserve the F170 injection guard + `\n` endings), `TabularEmit` dedup, `EmitterStream` streaming, in one coordinated byte-identical pass.

**F242 (module-depth coupling-subscriber browser test made live — new `coupling_repo` fixture + dedicated test; `373747e`, `9030159`)** closed the last SPA linked-brushing follow-up.

**2026-07-04 architecture-review pass**: **F243** (html un-advertised in 4 dispatchers — Fixed `acd9568`) and **F231** (Plan-N markers — Fixed via self-enforcing hygiene guard `52c427c`) closed; clippy-allow justification + SPA listener-bus + browser-fixture coverage landed. New own-slice: **F244** (analysis registry / `enum Format` + `TabularRow`, absorbs F215/F148/F119) and **F246** (canvas keyboard a11y); **F245** (widgets.js module split) landed this pass.

The 2026-08-02 discovery pass logged **F249–F268** (see §7); F269 was logged this pass (below). The post-v0.26.0 deferred-backlog pass logged **F270–F272** and closed F255/F269 (see §8). The post-v0.26.0 first-run UX pass logged **F273–F283** (see §9); its 0.27.0 re-verification added **F284–F286**. Later sweeps allocated onward from here; the live next-ID marker is the one at the ledger tail.

**F247 (Active) — `run_coupling_scoped` cutoff ignores lineage/time-bucket source in `good_commits`.** The rev-parameterizable `code_health` history cutoff (`HealthScanCtx::history_cutoff`) routes coupling through `run_coupling_scoped(db, opts, "changes_at_ts")`, which overrides only the pair-source + Fisher-denominator tables. The internal `good_commits_cte(bucket, use_lineage)` still reads the opt-derived `changes_lineage`/`changes`. For the primary path (no lineage, no time-bucket) this is equivalent — the cutoff-window revset equals full-history ∩ window. But `history_cutoff` combined with `--use-canonical-lineage` yields coupling pairs keyed on pre-rename path names, and combined with `--time-bucket` aggregates buckets over full history. The **same class** applies to code-health's own churn / revs / author-fragmentation CTEs: under a cutoff `{src}` becomes the raw, non-lineage `changes_at_ts` view, so those terms also lose rename-awareness when a cutoff is combined with `--use-canonical-lineage`. Neither combination is exercised (the timeline consumer uses the primary path — cutoff without lineage/bucket) nor required by the spec; documented in the `run_coupling_scoped` and `CHANGES_AT_TS_DDL` doc comments. Fix if a future consumer needs cutoff + lineage/bucket: build `changes_at_ts` from the lineage-rewritten source and thread `changes_source` into `good_commits_cte`. Surfaced by the Task-4 review + the whole-branch review of the rev-parameterizable code-health branch.

**F248 (Fixed — v0.27.1) — no integration coverage that `health-trend`'s `arch_health` falls as the import graph decays.** The `health-trend` analysis (`analyses/health_trend.rs`) computes `arch_health` from per-rev `GraphMetrics`, and the unit tests cover the pure function (empty/acyclic/fully-tangled/clamp). But the integration test (`tests/health_trend_test.rs`) only asserts shape/ranges/oldest-first, not the spec's "degrading architecture ⇒ `arch_health` decreasing" case — because the only ≥2-commit fixture, `biomarker_repo`, is six independent Rust files with no inter-file imports, so its import graph is empty and `arch_health` is pinned at 100 across every sample. Fix: add an import-structured fixture whose later commits introduce a dependency cycle (mirror `architecture_trend`'s `trend_captures_cycle_introduction_over_time`), then assert the newest sample's `arch_health` is below an earlier sample's. Surfaced by the whole-branch review of the Repo Health Timeline (piece 2). **Fixed** with `arch_health_falls_when_a_cycle_enters_the_import_graph`, mirroring `architecture_trend`'s cycle fixture: acyclic era, `a<->b` back-edge partway through, padded both sides so the even sampler lands on each. Asserts the score is not constant, then that the final sample sits below the acyclic peak. Proven non-vacuous before being trusted — on `biomarker_repo` the same assertion fails, because that fixture yields `[94.5 × 6]` (the finding said "pinned at 100"; the value is 94.5, constant either way). Priority rose because 0.27.0 promoted `health-trend` to step 1 of the README onboarding path, making an untestable column the first number a new user is told to trust.

**F269 (Fixed — v0.27.1; see §8) — `F<NN>` finding IDs embedded in test string literals and one test filename escape the comment-hygiene guard.** The `comment_hygiene_test` `Plan`-marker check now scans whole lines (comment + string + DDL), but the `F<NN>` task-ID check stays comment-scoped by design: bare `F<NN>` tokens appear as legitimate-looking test scaffolding — regression-message prefixes (`"F29 regression: …"` in `time_bucket_test.rs`, `"F33: …"` in `cache_test.rs`, `"F34: …"` in `gix_repo_test.rs`, `"F6 regression: …"` in `diff.rs`), an `eprintln!("[F69 spike] …")` label, and a git-config `user.name` fixture (`"F34"`) — so a whole-line/string scan would false-fire on all of them. These are the same banned class as comment F-IDs under the no-task-IDs-in-code rule, only in strings; and `tests/f69_window_spike_test.rs` carries the ID in its NAME, which a content scanner structurally cannot reach. Deferred as its own sweep (rename the file + rewrite the ~8 test labels to drop the ID while keeping each regression's description), distinct from the F231 `Plan N` sweep. Surfaced while extending the hygiene guard for F231.

---

## 7. Discovery pass — 2026-08-02 (F249–F268)

A six-dimension read-only research fan-out (robustness / rigor / performance / feature-deepening /
error-handling / testing), each grounded in source and adjudicated against the tracked baseline;
the controller then verified the load-bearing items directly. Full narrative — location, failure
scenario, proposed direction, value/effort, verification status, invariant touches — in
[`2026-08-02-discovery-pass-f249-f267.md`](./2026-08-02-discovery-pass-f249-f267.md). Index:

| F-ID | Subject | Sev | Status |
|---|---|---|---|
| F249 | `ensure_ingest_witnessed` guards only 2 of ~13 ingest entry points — `analyze`, `gate`, `gate_changes`, `explain`, 8 MCP tools render confident empty reports over a blind (fetch-depth:1 / all-excluded) ingest. ✅ grep-verified; convergent (5 signals). Gotcha: `analyze`'s `--after/--before` also empties `commits` → message must branch. | HIGH | Fixed (v0.25.0) |
| F250 | `codelore explain delivery-friction` 404s on a shipped, fully-documented metric. ✅ verified | LOW | Fixed (v0.27.1) |
| F251 | `coordination-needs` / `knowledge-islands` classify `high` tier / 100% ownership off n=2–5 with no denominator field (unlike `bus_factor`/`ownership`). ✅ verified | MED | Fixed (v0.27.1) |
| F252 | `write_github_output` silently swallows the open+write `Err` (`let _ =`). ✅ verified | LOW | Fixed (v0.27.1) |
| F253 | HEAD-scan blob I/O Phase-1 (refines F173/F206): blocker smaller than tracked (blob-read handling already identical; divergence is downstream AST-parse). One warm-ODB reader per rayon worker via the existing `map_init` idiom; also fixes `architecture-trend`/`cycle-origins` (never cached, re-paid per `analyze`). ✅ verified — byte-identical ingested facts + `architecture-trend` output before/after, differential suite unaffected (`GitCliRepo` untouched). | HIGH | Fixed (v0.27.1) |
| F254 | Cache-hit path runs a full O(tracked-files) `is_worktree_dirty()` walk on every invocation, just to maybe warn — defeats the cache on the agent-loop/CI hot path. ✅ verified | MED | Fixed (v0.27.1) — TTY-gated |
| F255 | `panic = "abort"` × long-lived `codelore mcp`: one panicking `spawn_blocking` tool call SIGABRTs the server for every client. ✅ verified (profile scope). **The proposed `catch_unwind` boundary was the wrong fix — it is a no-op under `abort`.** | HIGH | Fixed (v0.27.1) — see §8 |
| F256 | Small per-language cohorts collapse biomarker intensities to near-binary → false `structural_risk` red-bands; disclose cohort `n` (refines F236 residual — verify the "corpus lens addresses this" claim first). | MED | Active |
| F257 | Repo-wide function-level hotspots via `entities × hunks × commits` (no tree-sitter reparse — columns ✅ verified present). New capability. | HIGH | Fixed (v0.27.1) |
| F258 | `first_party_import_share` wildcard misclassification (`use crate::foo::*` tagged Wildcard→excluded) + a `wildcard_import_share` row. ✅ verified (`classify` branch order). | MED-HIGH | Fixed (v0.27.1) |
| F259 | Dead `commits.committer_email` (all refs are test `INSERT`s ✅) → a `landed_by_other_pct` gatekeeper metric; must ship the no-`committer_name`-mailmap caveat. | MED | Fixed (v0.27.1) |
| F260 | `hotspot-velocity` combined-window floor lets a single-window burst out-rank steadier activity; `RECENT/BASELINE_DAYS` + `EA_Z_FLOOR` uncited & unoverridable (bypass the `constants.rs` convention). | MED | Active |
| F261 | Dead `changes.similarity` (rename %) → an `avg_rename_similarity` / low-similarity-rename signal. | LOW | Active |
| F262 | Survival analysis on hotspots (Kaplan-Meier over hot-episodes) — re-scope of the roadmap Tier-1 item; no new ingest, but needs a design pass (stateful episode extraction + KM). | — | Active (design) |
| F263 | `[new_code]` gate `run_new_code_scope` has zero test coverage (only the pure evaluator is tested). Pairs with F249. | HIGH | Fixed (v0.25.0) |
| F264 | `is_shallow()` has zero tests — the primitive behind cycles 2/3/4's top finding. Pairs with F249. | HIGH | Fixed (v0.25.0) |
| F265 | `calibrate` total-failure (0-of-N) exit path untested (only partial-failure is) — would red-flag cycle-2's G2 bug. ✅ verified: G2 was already fixed (`calibrate.rs` hard-errors on 0-of-N with `CodeLoreError::Analysis`, exit 4); this was a coverage gap, not a live bug. Regression test locks in the existing behavior. | MED | Fixed (v0.27.1) |
| F266 | Differential harness missing binary / non-ASCII / submodule probes (fixture documents its own boundary). Touches the two-backend-parity invariant. | HIGH | Active |
| F267 | MCP `hotspots` never invoked via `tools/call`; `entity-effort`/`entity-ownership` have zero behavioral coverage. | MED | Fixed (v0.27.1) |
| F268 | CI `Build test binaries` link exhausts runner disk (SIGBUS in `ld`, different binary each run) linking 100+ fat test binaries. | MED | **Fixed (#196)** — Linux-only disk-reclaim step before checkout |

**Correction logged (not a finding):** Type-3 near-miss clones is *not* a latent-data quick win —
`clones/fingerprint.rs` stores a single SHA-256 digest (zero similarity signal); MinHash+LSH needs a
shingled representation = new ingest. Re-scope the roadmap's "~100 LOC" estimate before scheduling.

**Pointer (stale, corrected):** the `calibrate_defects` temporal train/validation
positive-leakage named here was subsequently **fixed** (hardening cycle 6, H3;
see `2026-08-06-hardening-cycle-6.md` and the shipped CHANGELOG entry) — this
pointer predates that fix and no longer marks open work.

---

## 8. Post-v0.26.0 deferred-backlog pass (F270–F272)

A verification pass over the deferred backlog carried into this cycle, plus a
sweep of the newest unaudited surface (the composite GitHub Action). Every
anchor was re-read against source before acting; two of the carried items
turned out to be mis-stated by the reports that logged them.

### Closed this pass

*   **F255 / the `panic = "abort"` MCP finding — Fixed.** The decision was made
    data-first: two fat-LTO stripped release builds of the shipped shape
    (`--features spa`) measured **50,830,224 B (48.48 MiB) under `abort` vs
    53,343,872 B (50.87 MiB) under `unwind` — +2.40 MiB, +4.95 %**. The flip
    took effect (the unwind binary imports `_Unwind_RaiseException` /
    `_Unwind_DeleteException`; the abort binary carries only the
    backtrace-side unwind symbols). Crucially the fix needed **no code**: the
    tool bodies already ran `spawn_blocking(…).await.map_err(internal)?`, so
    under unwinding a panicking tool becomes an MCP error response rather than
    a SIGABRT, and `diff`'s `Worktree` destructor runs again, closing the
    leaked-`git worktree` half. The report's `catch_unwind` prescription would
    have compiled and done nothing.
*   **F269 — Fixed.** A whole-line scan of the guard's own roots found exactly
    eight bare IDs, all in string literals, plus one in a file name. Scrubbed
    the labels (keeping each regression's description), renamed the spike file
    to say what it measures, and widened the guard's task-ID check to
    whole-line — it is now symmetric with the phase-marker check instead of
    deliberately asymmetric, and additionally rejects a file stem opening with
    a task-ID segment. Verified by the widened check first failing on its own
    module doc.
*   **The MCP tool-annotation and concurrency-bound items — Fixed.** All
    eleven tools now publish `readOnlyHint`/`openWorldHint`; `delta_health` is
    declared not-read-only (throwaway worktrees) and `explain_file`
    open-world (the optional `CODELORE_LLM_*` endpoint). Tool bodies route
    through one bounded `blocking` helper instead of calling `spawn_blocking`
    directly. `tools/list` now asserts the hints, so an unannotated tool fails
    the gate.
*   **The `pair_programming` bot-filter item — Refuted as a defect, but it sat
    on a false comment.** The Rust-side filter runs per participant and is
    invisible to the SQL bot-filter guard, which is correct: the guard bans a
    canonical-level `BOOL_OR(is_bot)` collapse in SQL, and this code does not
    do that. What *was* wrong is the module doc's claim that bots are already
    filtered from `commits.canonical_author` — `append_commit` writes every
    commit unconditionally, and bot exclusion happens per analysis through
    `HUMAN_ALIASES_CTE`. The claim made the analysis's own `is_bot` checks
    read as redundant. Corrected.
*   **The MCP protocol/error-drift test-coverage item — Refuted.**
    `mcp_test.rs` already asserts an exact tool count, the full name set,
    `inputSchema` presence on every tool, and carries a dedicated
    `assert_rpc_error_code` helper. The claim that it cannot detect protocol
    or error drift does not survive reading it.

### F270 (Fixed — v0.27.1) — the composite GitHub Action has no CI coverage at all

*   **Location**: `action.yml`; no workflow under `.github/workflows/` references it
*   **Severity**: MED · **Category**: test coverage / shipped-surface risk
*   **Description**: `action.yml` is a published user-facing surface and the
    only significant part of the repo outside every automated gate — the
    hygiene guard scans `.rs`/`.sql` under `crates/`, clippy scans Rust, and
    nothing executes the action. That gap is not theoretical: a single recent
    60-line diff to it accumulated a banned task-ID marker, two doc claims
    contradicted by the CLI's own argument definitions, and a bash
    portability trap, none of which any gate could see. All four were fixed
    this pass by reading the file, not by a failing check.
*   **Suggested improvement**: one workflow job that runs the action against
    this repo with `command: analyze` and again with `command: check`,
    matrixed over `ubuntu-latest` and `macos-latest`. That exercises version
    resolution, checksum verification, extraction, and both routing branches.
    Keep it to the two commands with real branch coverage; do not matrix the
    whole input surface.
*   **Outcome**: shipped as the `action` job in `ci.yml`, exactly at the scope
    above. Two steps — `analyze` (asserting `result-path` is set and the file
    is non-empty) and `check` (the non-analyze routing branch) — over
    `ubuntu-latest` + `macos-latest`, both leaving `args` empty because that is
    the default and therefore the path most users hit first — which is also the
    path the empty-array guard protects, so the matrix confirms that guard
    holds on both runner images. It does **not** establish what the unguarded
    expansion would have done there: the guard ships ahead of this job, so the
    bare `"${arr[@]}"` form is never executed. That question is moot, not
    answered.
*   **Known coupling, documented in the job**: the Action installs a *published
    release* binary, so this job cannot test the code in the PR — it tests the
    Action's own mechanics. Its `check` step therefore runs the released binary
    against the *current* thresholds file, and a genuine verdict can surface if
    those drift apart. `self-gate` runs the same gate with a binary built from
    the PR, so the two together distinguish "the Action is broken" from "the
    release disagrees with current thresholds". Deliberately not marked
    `continue-on-error`, which would make the job decorative.

### F271 (Fixed — v0.27.3, partial) — MCP tools hand-roll JSON into a text block instead of declaring structured output

*   **Location**: `codelore-cli/src/mcp.rs` — ten of the eleven `#[tool]` bodies
    return `Result<String, ErrorData>`. `check_gates` returns
    `Result<Json<GateSummary>, ErrorData>`, which is what makes it the one tool
    with an `outputSchema`: `rmcp`'s `#[tool]` macro derives the schema from a
    `Json<T>` return type, so nothing in this repository names `output_schema`
    and a text search for it reports zero. Read the return types instead.
*   **Severity**: LOW-MED · **Category**: protocol fidelity (optional)
*   **Description**: Each tool serialises its rows itself and returns the JSON
    as text, so clients get no `outputSchema` and no structured-content block.
    An agent must parse the text and cannot validate it.
*   **Why it is not built**: rmcp supports this via `Json<T>` + `output_schema`,
    but taking it means inventing an output struct for each of eleven tools
    whose current returns are heterogeneous (bare arrays, objects, arrays with
    a trailing `{omitted, total, note}` summary, and a plain-text briefing from
    `change_context`). That is a type-surface expansion well past the value,
    and the trailing-summary shape would have to be redesigned to fit a schema.
    Logged rather than built, per the minimum-surface rule. Revisit only if a
    client actually rejects the text-block shape.
*   **Resolution — `check_gates` only, and the survey that bounded it.** The
    "eleven tools" framing hid that they are not one problem. Classified by
    what a schema would actually cost:
    *   **`check_gates`** — its output was already a struct (`GateSummary`) in
        the binary crate, where `schemars` is already a dependency. Three
        derives and a return type. Done.
    *   **`hotspots`, `code_health`, `refactoring_targets`,
        `finding_hotspot_overlap`** — return an array whose *last element is a
        different type* (the `{omitted, total, note}` disclosure). That is not
        expressible as a schema, so the wire shape would have to become an
        object: a breaking change for every current consumer.
    *   **`repo_overview`, `delta_health`, `explain_file`** — build their
        output with inline `serde_json::json!`, and the content is library row
        types. Typing them means `schemars` in `codelore-lib`, which is
        published, so the dependency lands on every library consumer.
    *   **`function_xray`** — returns *two incompatible top-level shapes*: an
        object when the file is not a Tier-1 language, a bare array otherwise.
        One schema cannot describe both without changing the wire.
    *   **`change_context`, `gate_changes`** — prose briefings. A text-only
        tool is valid MCP; there is no JSON document to describe.
*   **Backwards compatibility, verified rather than assumed**: `rmcp`'s
    `Json<T>` doc says structured content is placed in `structured_content`
    "rather than the regular `content` field", but the implementation populates
    **both** — the text block survives. Confirmed on the wire by probing a live
    server before and after: `structuredContent` absent then present, text
    block still delivered.
*   **One thing did change**: routing through `serde_json::to_value` reorders
    object keys alphabetically. Same fields, same values; disclosed in the
    CHANGELOG. Established by capturing the pre-change binary's output and
    diffing it, after two separate inferences from `Cargo.lock` and
    `cargo tree` both predicted no change and were wrong.

### F272 (Fixed — v0.27.1) — nothing enforces agreement between the six Rust-version pin sites

*   **Location**: `rust-toolchain.toml`, workspace `rust-version`, `clippy.toml`, `Containerfile` (`ARG RUST_VERSION` and the `FROM` digest), the `dtolnay/rust-toolchain` action tags, `CHANGELOG.md`
*   **Severity**: LOW · **Category**: release plumbing / drift guard
*   **Description**: `cut-release.sh` does not bump these (the doc wording that
    claimed it did was already corrected), and no test asserts they agree. All
    six currently read 1.96, so this is a guard against future drift, not a
    live defect. A bump that misses a site fails in a way that is hard to
    attribute: the workflow tag alone is a silent no-op because
    `rust-toolchain.toml` overrides it.
*   **Suggested improvement**: prefer a test over automation. Every pin lives
    at a static path, so an `include_str!` agreement test — a sibling to
    `dep_versions_drift_test.rs`, which is the established pattern for exactly
    this — needs no globbing and cannot rot.
*   **Known blind spot the test cannot cover**: `Containerfile`'s builder base
    is pinned as `rust:${RUST_VERSION}-${DEBIAN_RELEASE}@sha256:…`, and the
    digest wins over the tag. Bumping `ARG RUST_VERSION` without a matching
    digest bump silently keeps building on the old toolchain, and no textual
    agreement check can see that. Call it out in the test's failure message
    rather than pretending it is covered.
*   **Outcome**: shipped as `tests/rust_version_pins_test.rs`, following
    `dep_versions_drift_test.rs`. `rust-toolchain.toml` is the source of truth;
    the other three file pins are embedded with `include_str!` and the action
    tags are read at run time from the workflows directory, so a workflow added
    later is covered without editing the test. Agreement is checked on
    major.minor because the sites are written at different precisions on
    purpose. The digest blind spot is named in the failure message. Verified by
    injecting drift into both a file pin and a workflow tag and confirming the
    guard names both — a drift guard that cannot fail is worth nothing.

## 9. First-run UX pass (F273–F283), plus findings appended after it (F284–F294)

Validated against the shipped 0.26.0 binary, not inferred. This section is
the F-ledger and the record of the pass; `2026-08-06-first-run-ux-review.md`
carried the then-open set — F276 (since Refuted), F278 and F279 (since Fixed);
only the deferred thresholds scaffold remains live from that list.

### F273 (Fixed — v0.27.1) — the cache key carried the repo path as the user typed it

*   **Location**: `options.rs::canonical_json` (classification guard + snapshot), `cache.rs` module header and `cache_path_with_root` doc
*   **Severity**: MED · **Category**: cache correctness / performance
*   **Description**: `cache.rs` documented `repo_path` as excluded from the key.
    It was on the *included* side of the classification guard and flowed into
    `opts_hash` un-canonicalised. One repository at one HEAD with identical
    flags therefore derived a different key per spelling. Measured on a clean
    cache root: absolute, symlinked, trailing-`/.`, and `../`-relative
    spellings produced four entries in one directory, every one a miss.
*   **Sharpest detail**: the guard's own comment warns that "an absolute-path
    or per-invocation field would silently make every cache key
    machine-specific". `repo_path` is exactly that field, classified onto the
    hazardous side by the guard built to prevent it.
*   **Outcome**: normalised on the snapshot beside the other per-invocation
    selectors — `map.remove` is reserved for paths replaced by a content
    digest, and `repo_path` has none. Identity is untouched: `cache_key`
    hashes the canonical path as its first component, so the raw copy was
    redundant rather than load-bearing. No `CACHE_EPOCH` bump — the preimage
    changed, so old entries are unreachable by construction, and a bump would
    additionally orphan every `diff --base-cache` file for no gain.

### F274 (Fixed — v0.27.1) — eviction is documented as LRU and behaves as FIFO

*   **Location**: `cache.rs` (banner comment, `prune_global_cache` doc), `facts/mod.rs`, `external/store.rs`, `quality_gates/ledger.rs`, `docs/advanced-usage.md`
*   **Severity**: LOW · **Category**: documentation accuracy
*   **Description**: both pruners sort ascending by **mtime**, and a cache hit
    opens the fact store read-only — mtime and atime are byte-identical across
    a hit — so mtime is frozen at ingest-completion forever. The policy is
    FIFO by ingest time. An entry hit daily is evicted ahead of a newer one
    never reused. Five code sites plus one user-facing doc line call it LRU.
*   **Suggested improvement**: relabel; do not implement true LRU. That needs
    either a new dependency (`std` cannot set mtime, and `libc::utimensat` is
    barred by `unsafe_code = "forbid"`) or a per-hit sidecar write on the
    deliberately near-O(1) read path. Disproportionate to an eviction-order
    difference inside a 5-entry cap.
*   **Outcome**: all six sites relabelled. The banner comment now records why
    the trade is acceptable rather than only what the policy is — keys are
    HEAD-scoped, so the entry being hit is usually the most recently ingested
    one, and a wrong eviction costs one re-ingest and never correctness. The
    user-facing guide states the consequence, since it is observable.

### F275 (Fixed — v0.27.1) — emptied per-repo cache directories are never removed

*   **Location**: `cache.rs::prune_repo_cache` / `prune_global_cache`
*   **Severity**: LOW · **Category**: cache hygiene / cache-miss latency
*   **Description**: both pruners delete `.duckdb` files (and `.wal`
    companions) and nothing else — `cache.rs` contains no `remove_dir`. On the
    development machine: 7,990 per-repo directories, 6,643 of them completely
    empty. `codelore calibrate` is the dominant producer, ingesting ~99
    throwaway corpus checkouts through the default root per run.
*   **Why it is not purely cosmetic**: the bytes are ~0, but
    `prune_global_cache` walks the whole tree **twice** on every cache miss
    (stale-tmp sweep, then the `.duckdb` collection). A warm walk of the
    current tree measures ~0.34 s, ~79% of it empty directories, and it grows
    monotonically.
*   **Suggested improvement**: an age-gated sweep at the tail of
    `prune_global_cache`, gated on **total** emptiness
    (`read_dir().next().is_none()`, not "no `.duckdb`") and using
    non-recursive `fs::remove_dir` as a second net. Five sidecar families
    share that directory — gate ledger, external findings, change-sets,
    enrichment, in-flight tmp — so a `remove_dir_all` would destroy them.
*   **Outcome**: implemented as specified, and placed BEFORE the size walk
    rather than after it — `prune_global_cache` returns early when the cache is
    under cap, which is the common case, and directories accumulate whether or
    not the cap binds. The age threshold is a parameter, not the constant,
    because a directory's mtime cannot be back-dated portably: a test unable to
    lower it could only ever assert that nothing was swept. Both halves are
    pinned — a directory holding a ledger, a live entry, or only a
    subdirectory survives; a fresh empty one is left alone.

### F276 (Refuted) — `evaluate_all_gates` discards measured values it already computed

*   **Location**: `codelore-cli/src/check.rs::evaluate_all_gates`
*   **Severity**: LOW · **Category**: reusability
*   **Description**: `eval_hotspot_gates` runs the hotspot scan
    unconditionally, but only `hotspot_rows.len()` is returned, so the measured
    values behind `cognitive_max`, `hotspot_score_max` and
    `hotspot_anchored_max` are computed and thrown away. Code-health rows *are*
    returned, so `code_health_min` and `corpus_percentile_max` are already
    available.
*   **Why it matters**: this is the gate on any measured thresholds scaffold.
    Widening the return unlocks six gates cheaply. The remaining gates are a
    different problem — `evaluators.rs` skips **building the import graph at
    all** unless `max_dependency_cycles` or `max_propagation_cost` is already
    configured, so a scaffold cannot measure what it is meant to propose.
*   **Refuted — the measured values are not discarded.** Checked against
    source: every one of the three gates records its measured value through
    `make_rec`, whose `value` field is exactly that measurement, and those
    records leave the function as `ledger_records`.
    *   `cognitive_max` and `hotspot_score_max` — `make_rec` inside
        `eval_hotspot_gates`.
    *   `hotspot_anchored_max` — `make_rec` in `evaluate_all_gates`, with the
        value folded as the max anchored score across rows, plus an explicit
        `"skipped"` record when no calibration anchor is active.
*   **The rows are not discarded either.** `eval_hotspot_gates` returns the
    full `Vec<HotspotRow>`, not a length, and `evaluate_all_gates` reuses those
    rows for the anchored gate — its own comment says so. Only the outer return
    narrows to `hotspot_count`, and that count has a consumer: the
    `check: PASS (N files evaluated)` line.
*   **Nothing consumes what widening would expose.** The stated motivation is a
    "measured thresholds scaffold" that does not exist. Widening a return for
    an unbuilt caller is the speculative generality the repository's own rules
    forbid, and the ledger records already are the durable home for measured
    values — which is what a scaffold would read.
*   **Residual, deliberately not acted on**: none. If a thresholds scaffold is
    ever built, it should read `GateRunRecord`s rather than re-plumb the gate
    functions' return types.

### F277 (Fixed — v0.27.1) — the cache canonicalisation test pinned an invariance that could not fail

*   **Location**: `codelore-lib/tests/cache_test.rs`
*   **Severity**: MED · **Category**: test quality
*   **Description**: the test passed one `Options::default()` (whose
    `repo_path` is `"."`) to both `cache_key` calls and varied only the free
    `repo_path` argument. Production always passes `&opts.repo_path`, so the
    two co-vary — the test asserted something that could not break, and was
    structurally blind to F273 sitting directly under it.
*   **Outcome**: varies both, compares two spellings that differ textually on
    every platform (comparing against the tempdir path is vacuous on systems
    where it is already canonical), and asserts they differ *before* asserting
    the keys match. Verified to fail against the pre-fix behaviour.

### F278 (Fixed — v0.27.3) — the hygiene guard's ID vocabulary is `F`-plus-digits only

*   **Location**: `codelore-lib/tests/comment_hygiene_test.rs::is_task_id`
*   **Severity**: LOW · **Category**: guard coverage
*   **Description**: `is_task_id` matches a token of `F` followed by 1–3
    digits. `T8:` and `(Task 13)` are both live in the tree and both invisible
    to it; the `T8:` instance had reached published `--help` output.
*   **Constraint that makes this non-trivial** (validated, not assumed): a
    naive `T<N>` rule collides with the domain vocabulary. `T1`/`T2`/`T3` are
    the clone-type names (`clone_coupling.rs`: "1.0 for T1+T2 exact matches"),
    and every ISO-8601 timestamp in the test fixtures contains `T00:`/`T10:`.
    A usable rule has to be anchored — e.g. `T<digits>:` opening a comment or
    doc line — rather than a bare token match. Widening the vocabulary without
    that anchor produces false positives on correct code.
*   **The prescribed anchor was insufficient** (measured, not assumed): a
    census of every standalone `T`-plus-digits token in scope returned 13 real
    IDs across three series and 12 domain-vocabulary hits. Only 5 of the 13
    are written as `T<digits>:`; the rest appear as a parenthesised aside, a
    following word, an em-dash continuation, and one inside a string literal.
    Anchoring on the colon would have missed most of them.
*   **Rule that holds**: flag a standalone `T`-plus-digits token whose
    preceding byte is not alphanumeric, `_`, or `}`. The brace and digit
    exclusions drop every ISO-8601 timestamp — the separator is always glued
    to a literal digit or a format placeholder's brace — without an exemption
    list that would rot as fixtures change. The clone-type names are a
    three-entry allowlist, whose stated cost is that a future task numbered
    1-3 would not be caught. `Task <N>` reuses the existing `Plan <N>` matcher
    rather than adding a parallel one.
*   **Also fixed**: all 15 live violations, closing F279's remainder. One was
    user-facing and not in that finding's list — `codelore explain
    knowledge-islands` printed a task ID in its **Citation** field, beside real
    sources like "DORA 2018 Accelerate"; it now reads "Bird et al. 2011
    risk-author". Another was stale as well as banned: a test claimed its
    integration coverage was still to come, when 34 differential tests and two
    per-backend files had long since been written.
*   **Durable half**: a self-test pins both directions on the shapes that
    actually occur here — four ID spellings must flag, four domain-vocabulary
    shapes must not. The guard scans its own file, so the rules are documented
    by shape rather than by example; writing a literal ID as an illustration
    fails the gate, which is the guard behaving correctly and was observed
    twice while writing it.

### F279 (Fixed — v0.27.2 + v0.27.3) — a ticket ID shipped in user-facing help

*   **Location**: `args.rs` (fixed); `analyze.rs`, `explain.rs`, `options.rs`, `clone_coupling.rs` (remaining)
*   **Severity**: LOW · **Category**: convention violation
*   **Description**: `codelore analyze --help` printed `T8: An author is
    considered "departed"…`. The project forbids ticket IDs in code, and this
    one reached the published binary.
*   **Outcome**: the help-text instance is removed. Instances remaining in
    library doc comments and inline comments are not user-facing but are the
    same violation; clearing them is gated on F278's anchored rule, so they are
    fixed and guarded together rather than piecemeal.
*   **Closed (v0.27.3)**: the remainder is cleared with F278's rule, which landed in that release. Re-validated: all five files this finding named carry no ID markers. The stamp read "+ Unreleased" for two releases after the work shipped — the exact rot `ledger_stamp_test` exists to catch, and invisible to it because a compound stamp matches neither spelling it counts. The
    sweep found one more user-facing instance than this finding listed —
    `codelore explain knowledge-islands` printed a task ID in its **Citation**
    field, which the original `--help` search did not reach because it looked
    at argument help rather than every published string.

### F280 (Fixed — v0.27.1) — a vacuous gate pass wrote `result` without `violations`

*   **Location**: `check.rs`, `gate.rs`
*   **Severity**: LOW · **Category**: CI contract
*   **Description**: both vacuous-pass paths wrote `result=pass` to
    `$GITHUB_OUTPUT` and stopped; all five other exit paths across the two
    commands write `violations` too. A workflow reading `outputs.violations`
    received an empty string rather than a count.
*   **Outcome**: both write `violations=0`, guarded by a test shown to fail
    against the previous behaviour.

### F281 (Fixed — v0.27.1) — `check --help` rendered `auto- discovered`

*   **Location**: `args.rs` `thresholds_file` doc comment
*   **Severity**: LOW · **Category**: help-text rendering
*   **Description**: the doc comment split `auto-discovered` across two lines;
    clap re-joins doc lines with a space, so both `check --help` and
    `gate --help` printed the hyphen and the remainder separated.

### F282 (Fixed — v0.27.1) — a filtered-to-empty analysis reports success and explains nothing

*   **Location**: `analyze.rs` (`dispatch!` macro, `run_streaming_dispatch`, the footer's unused `Footer.rows`)
*   **Severity**: MED · **Category**: honest-absence convention
*   **Description**: the README's own first command carries `--min-revs 5`, the
    documented default. On a young repository nothing clears it, so the run
    prints one CSV header row, exits 0, writes nothing to stderr, and under a
    TTY frames it with `Status: ✓ ready` and `✓ hotspots completed`. `gha` and
    `ndjson` emit **zero bytes**, which in CI reads as "no findings".
    `ensure_ingest_witnessed` correctly does not fire — ingest saw everything;
    the emptiness is introduced downstream by the predicate.
*   **Scope note**: the convention already exists ~28 times (24 of 57 markdown
    emitters carry an `rows.is_empty()` branch, 13 of which print a message,
    plus the HTML empty state) and **zero** times across the CSV emitters —
    the default path.
*   **Implementation constraints** (validated): the `dispatch!` macro binds
    `rows` at three sites and covers 185 of 189 streaming pairs, but
    `macro_rules!` hygiene means `opts` is unreachable from the macro body and
    absent from `EmitCtx`, so the message cannot be composed there. It belongs
    in `analyze()` beside the existing empty-window `tracing::warn!`, which is
    the same shape of advisory. `CommunitiesResult` has no `len()`. The footer
    is TTY-gated, so a fix hung off it dies exactly where silence is most
    dangerous. Message should state the filters in effect, not assert a cause:
    `min_revs` is read by 40 of 57 analyses, and the rest return zero rows for
    unrelated reasons.
*   **Outcome**: the count escapes the macro (three bind sites, one per rule
    plus the HTML arm) and the advisory is emitted in `analyze()` where `opts`
    is in scope, beside the empty-window warning it mirrors. `Footer.rows` is
    populated, closing the deferral the comment described rather than working
    around it — the premise had gone stale when the arms were folded into the
    macro. Not TTY-gated, which is both the point and what makes it assertable
    from an integration test. Wording says "options in effect", not "filters":
    testing `release-cadence`, which never reads `min_revs`, showed the
    stronger phrasing overstated it for the analyses that ignore the knob.
    `CommunitiesResult` gained `len`/`is_empty` as the one dispatched result
    that is not a `Vec`.

### F283 (Refuted as a metric defect; log noise Fixed — v0.27.1) — the partial-tree warning fires on a grammar quirk, not on lost structure

*   **Location**: `codelore-lib/src/complexity`, vendored `codelore-rca` grammars
*   **Severity**: MED (unverified impact) · **Category**: metric correctness
*   **Description**: every `codelore check` run logs `complexity: parse errors
    in <file> — metrics computed on a partial tree` for roughly a dozen source
    files, `crates/codelore-cli/src/main.rs` among them. Those files'
    complexity feeds `code-health`, the hotspot score, and this repository's
    own self-gate, so a partial parse understates the metric wherever it hits.
*   **Cause, located**: every affected Rust file contains the token `&raw`.
    `&raw const` / `&raw mut` is Rust 2024's raw-borrow operator, so the pinned
    tree-sitter grammar commits to a raw borrow on seeing `&raw`, then errors
    when the next token is neither `const` nor `mut`. The codebase has a
    dozen locals named `raw` (`&raw` passed to a parse helper). Correlation is
    exact: 12 of 12 warned-about `.rs` files contain the token; sampled files
    that do not contain it never warn.
*   **Impact, measured — none.** Renaming the identifier so the token
    disappears flips `has_error()` from true to false, and every metric is
    byte-identical either way: cognitive, cyclomatic, nexits, nargs and the
    space count all match exactly across four sampled files, including
    `main.rs` (cognitive 63.00, cyclomatic 132.00, 24 spaces on both sides).
    Tree-sitter's error recovery confines the ERROR node to the `&raw`
    expression without swallowing any enclosing structure, so `code-health`,
    the hotspot score and the self-gate read the same numbers they would on a
    clean parse.
*   **What is left is log noise**, not a correctness defect: 14 WARN lines on
    every `check` run, which trains a reader to skim past warnings that do
    matter. The honest fix is not to touch the metrics but to stop crying wolf
    — either demote this to `debug!`, or keep `warn!` only where the error
    region actually overlaps a measured entity.
*   **Outcome**: demoted to `debug!`. The narrower option was checked and
    discarded: the `&raw` sites sit inside the functions being measured, so
    gating on overlap would fire anyway. Default runs are now silent; `-v` (or
    `RUST_LOG`) still surfaces all 14 for anyone investigating a genuinely
    suspect file, verified both ways.
*   **Note for whoever revisits the JS side**: two `.js` files warn as well
    (`00_setup_boot.js`, `90_toggles_utils.js`) and cannot share this cause.
    They were not investigated; assume nothing from the Rust result.

### F284 (Fixed — v0.27.1) — the zero-row notice prescribed a remedy most analyses cannot use

*   **Location**: `codelore-cli/src/analyze.rs`, the F282 notice
*   **Severity**: LOW-MED · **Category**: honest-absence convention
*   **Description**: the notice closed with "relax the thresholds (e.g.
    `--min-revs 1`)". Measured, `min_revs` is a genuine filter in **17** of the
    modules under `analyses/`; **23** name it only inside a
    `tracing::instrument` span and **23** never mention it. The suggestion is
    therefore a dead end for most analyses. On `defect-validation` it was worse
    than useless: the analysis prints the correct instruction to build a
    calibration artifact, and the notice immediately advised lowering a
    threshold that file does not reference at all.
*   **Root cause of the error**: the "40 of the analyses read `min_revs`"
    figure in F282's own comment counted `opts.min_revs` occurrences without
    excluding the span-field idiom `fields(min_revs = opts.min_revs)`, which
    23 modules carry purely for tracing. The number was measured, but with the
    wrong predicate — a reminder that a count is only as good as what it
    counts.
*   **Outcome**: the remedy clause is removed; the notice states the analysis,
    the zero and the options that were set. Deliberately no per-analysis
    "does this filter on min_revs?" lookup: that knowledge exists nowhere
    derivable at runtime, and hardcoding a 17-name list would rot the moment an
    analysis is added — the exact parallel-knowledge trap the project's
    conventions forbid. The options summary already carries `min-revs=<n>`, so
    where it is the cause the number sits beside the zero, and §12 of the
    advanced-usage guide covers the header-only case.
*   **Rejected alternative**: pointing at `codelore explain <analysis>` instead.
    Checked before proposing — `explain` returns citation, formula and source,
    and discloses no filter thresholds, so the pointer would have been the same
    class of false promise it was meant to replace.

### F285 (Fixed — v0.27.1) — `check` recommended a gate key the README never explained

*   **Location**: `main.rs::vacuous_pass_notice` ↔ `README.md`
*   **Severity**: LOW · **Category**: documentation coherence
*   **Description**: the vacuous-pass notice names three starter keys.
    README occurrences were `code_health_min` 1, `max_dependency_cycles` 1,
    `max_red_effort_pct` **0** — and the missing one is the subtlest of the
    three, so it was the one a reader could not look up where the CLI had just
    sent them. It was documented only at `advanced-usage.md`'s
    `#### max_red_effort_pct quality gate`.
*   **Outcome**: added to the README's gate example with a one-line definition.
    General rule worth keeping: every key a CLI message names should resolve
    where that message points.

### F286 (Fixed — v0.27.1) — the onboarding path handed off past its own continuation

*   **Location**: `README.md`, end of "Your first 5 minutes"
*   **Severity**: LOW · **Category**: documentation flow
*   **Description**: the section closed by sending the reader to the
    1,700-line advanced guide — four lines above "Tracking health over time",
    the section added specifically to answer the question a reader has at that
    point. The pointer competed with its own continuation.
*   **Outcome**: the handoff names the next section first and the reference
    guide second.

### F287 (Fixed — v0.27.2) — the documented way to use the published Action references a ref that does not exist

*   **Location**: `README.md`, `docs/github-action.md` (14 occurrences); no `v1`
    ref on origin; `scripts/cut-release.sh` and `release.yml` create none
*   **Severity**: HIGH · **Category**: published-surface availability
*   **Description**: every documented invocation is
    `uses: emrecdr/codelore@v1`, including both examples under "Versioning" —
    so the exact-pin example pins the *binary* version while still routing the
    *action* reference through `@v1`. No `v1` tag or branch exists, and nothing
    in the release process creates or moves one. A workflow copied from the
    docs fails at the step with `Unable to resolve action
    emrecdr/codelore@v1`; the action never runs.
*   **Why two audit cycles missed it**: both audited what `action.yml`
    *contains*, and CI exercises it as `uses: ./`. The reference form a
    consumer actually types is exercised nowhere. F270 closed "the Action has
    no CI coverage" by running the action; nobody asked whether it could be
    reached. A local path proves the mechanics and says nothing about
    availability.
*   **Coupling to F-H1 (the version injection)**: these constrain each other's
    order. The injection is less urgent than it appears — a `@v1` that does not
    resolve cannot be exploited — but creating `v1` publishes whatever it
    points at to every third-party consumer at once, so it must not be created
    on a release carrying the vulnerable `action.yml`. Fix order is therefore
    forced: release the fix first, then create the ref.
*   **Open decision, not yet a prescription**: it is not established whether
    `v1` was deliberately withheld until the Action stabilised. If so the
    defect is the documentation promising a ref that was never published, and
    the fix is to document exact tags (`@vX.Y.Z`) instead of creating `v1`.
    Both are one-line-per-occurrence changes; they differ in what is promised,
    which is the maintainer's call.
*   **Whichever is chosen, the durable half is the same**: nothing today keeps
    the documented reference and the published refs in agreement. A guard that
    resolves every `uses: emrecdr/codelore@<ref>` in the docs against the
    repository's actual refs would have caught this on the day the docs were
    written, and would catch a floating `v1` that stops being moved.
*   **Resolution — publish `v1` and keep it moving.** The tag was created at
    `v0.27.1` (verified: `action.yml` is byte-identical between that tag and
    `main`, and carries the injection hardening, so the ordering constraint
    above is satisfied — nothing vulnerable was published). `cut-release.sh`
    now moves it onto each release inside the existing ruleset window, and the
    documented-ref guard ships alongside.
*   **Why the move had to go inside the ruleset window**: `protect-release-tags`
    matches `refs/tags/v*`, which includes `v1`, and enforces
    `non_fast_forward`. Re-pointing an existing `v1` is a non-fast-forward
    update and is rejected while enforcement is active; `deletion` is blocked
    too, so delete-and-recreate is not an escape hatch. A naive "just retag in
    a release step" automation would have failed on the second release, not the
    first — the worst time to discover it.
*   **`v1` is a constant, not a derived major**: it versions the Action's
    *interface*, independent of the crate version, the way `actions/checkout@v4`
    tracks no product version. codelore is `0.x`, so deriving the major from
    `VERSION` would yield `v0` and contradict every documented example. The
    docs' "Following SemVer. Major-version pin recommended" phrasing conflated
    the two and has been rewritten to separate the Action pin from the binary
    `version:` pin — the old "pin to a specific release for reproducibility"
    example pinned only the binary while the Action itself still floated.

### F288 (Fixed — v0.27.2) — `workflow_dispatch` on `release.yml` is documented as a test run but publishes for real

*   **Location**: `.github/workflows/release.yml` — header comment ("Manual
    workflow_dispatch (test runs)"), `release` and `homebrew-publish` jobs
*   **Severity**: MEDIUM · **Category**: outward-facing side effect / misleading affordance
*   **Description**: the workflow accepts `workflow_dispatch` and its header
    presents that as the way to test-run the pipeline. No job carries an `if:`
    guard. Exactly one *step* is guarded — `crates-publish`'s publish step,
    on `github.ref_type == 'tag'` — so crates.io is safe. `release` and
    `homebrew-publish` are not. A manual run therefore creates a real GitHub
    Release tagged `manual-<timestamp>` (the `plan` job's non-tag fallback),
    uploads five binaries to it, and pushes a regenerated formula pointing at
    that release to `emrecdr/homebrew-codelore`. `brew install codelore` would
    then resolve to a throwaway build.
*   **Why it has not fired**: nobody has taken the header at its word. The
    affordance is documented but unused, so the cost has stayed theoretical —
    which is also why it survives review: the guard that exists on
    `crates-publish` reads as evidence the case was handled.
*   **Discovered**: while looking for a way to exercise the Build L3
    attestation split without cutting a tag. The migration is structurally
    verified and guarded, but genuinely unexercised until the next release,
    and this is why.
*   **Resolution**: all three publishing jobs (`release`, `homebrew-publish`,
    `crates-publish`) now carry `if: github.ref_type == 'tag'`, so
    `workflow_dispatch` runs `plan` → `build` → `attest` and stops. That is the
    dry run the header already claimed, and it makes the Build L3 attestation
    path exercisable without cutting a tag. `crates-publish` keeps its
    step-level condition as well: it also covers the unconfigured-token case,
    and a permanent publish is worth guarding twice.
*   **Why each job is guarded rather than just `release`**: skipping `release`
    would likely cascade through `needs:`, but that couples an outward-facing
    safety property to dependency-graph semantics — a later edit to a `needs:`
    list would silently re-enable publishing. Each job asserts its own
    precondition instead.
*   **Residual**: a dry run still writes real attestations for the throwaway
    archives. They are digest-bound and harmless, and signing is the part most
    worth exercising, so this is accepted rather than suppressed.

### F289 (Fixed — v0.27.2) — publishing workflows triggered on the Action's floating major tag

*   **Location**: `.github/workflows/release.yml`, `.github/workflows/container.yml`
    (`on: push: tags`), coupled to `scripts/cut-release.sh::ACTION_MAJOR_TAG`
*   **Severity**: HIGH · **Category**: cross-file coupling / published-surface breakage
*   **Description**: both workflows triggered on `tags: ['v*']`. F287's fix
    added a `v1` tag that `cut-release.sh` re-points on every release — and
    `v1` is a `v*` tag. Pushing it ran the full release pipeline a second
    time and published a GitHub Release named `v1`. GitHub's
    `releases/latest` then returned `v1`; the Action's own `version: latest`
    resolution rejected it against `^v[0-9]+\.[0-9]+\.[0-9]+...$` and exited
    1, so **every consumer of the published Action failed**. The Homebrew tap
    was regenerated as "codelore 1" pointing at that release. crates.io was
    untouched — the idempotent `publish_if_absent` probe saw the versions
    already live and skipped, which is the only reason the irreversible
    surface survived.
*   **How it was introduced**: by the F287 fix itself, in this repository,
    and caught by the release cut that followed minutes later — the release
    commit's own CI went red on the `action` jobs, and `cut-release.sh`
    aborted before tagging. The abort ordering (CI gate strictly before the
    tag dance) is what kept it recoverable.
*   **Why it was missed**: the `v1` design was validated against
    `protect-release-tags`, whose condition is `refs/tags/v*` — the
    non-fast-forward constraint was found and handled. The *workflow trigger*
    uses the same `v*` pattern in a different file and was never checked. One
    `v*` was reasoned about carefully; the other was not looked at.
*   **Resolution**: both workflows trigger on `v*.*.*`, which matches every
    real release tag including pre-release suffixes (`v1.0.0-rc.1`,
    `v0.1.0-alpha.2`) and no bare major. The bogus `v1` Release was deleted
    (the *tag* kept, so `uses: @v1` still resolves) and the tap restored to
    0.27.1.
*   **Durable half**: a guard reads `ACTION_MAJOR_TAG` out of `cut-release.sh`
    and asserts no workflow tag-trigger glob matches it, tying the two files
    together. Verified discriminating: restoring `v*` fails it with the exact
    diagnosis. The general lesson is the one the guard encodes — two settings
    that are individually reasonable and destructive only in combination need
    a check that spans both files, because no reviewer reading either file
    alone can see the hazard.

### F290 (Fixed — v0.27.3) — the two remaining MCP tools that returned unbounded violation lists

*   **Location**: `codelore-cli/src/mcp.rs` — `check_gates` (JSON `violations`),
    `render_gate_changes` (text violation loop)
*   **Severity**: MEDIUM · **Category**: agent-context budget
*   **Description**: the cap-and-disclose regime reached every list tool except
    these two, which emitted one row per violation with no bound. A wide
    refactor against a tight gate is whole-population output into the context
    window.
*   **What the audit's framing missed** (checked against source, not taken on
    the report's word): the two tools do not share an output shape, so a single
    prescription does not fit. `check_gates` returns a JSON struct — the
    `serialize_capped_rows` helper serializes a *slice*, so it does not apply to
    a struct field. `gate_changes` returns rendered text, where that helper does
    not apply at all. And in `render_gate_changes` two of the three loops were
    *already* capped (`GATE_FINDINGS_ROWS`, `GATE_DELTA_TABLE_ROWS`); only the
    violation loop was not. The finding was one missing constant plus one
    missing struct-field cap, not "unbounded arrays" as a class.
*   **Resolution**: `check_gates` takes a `limit` (default 50, the file's
    `resolve_row_cap` convention) and truncates `violations`, with
    `violation_count` measured *before* truncation so it stays the true total —
    the verdict and the number an agent reports are invariant under the cap, so
    lowering `limit` can never resemble fixing violations. `gate_changes` gains
    `GATE_VIOLATION_ROWS`, the third render-only cap beside its two siblings,
    with the matching `(+n more violations)` tail.
*   **Deliberately not changed**: the `codelore gate` JSON document still
    carries every row. `GATE_FINDINGS_ROWS`'s own comment records that as a
    design decision (spec §6) — that document is a file artifact, whereas an
    MCP response is context-window budget. Applying the render cap there would
    have contradicted a documented invariant.
*   **Durable half**: a test asserts `violation_count` is identical across a
    capped and an uncapped call while the row counts differ. Verified
    discriminating — removing the `truncate` fails it.

### F291 (Fixed — v0.27.3) — a `limit` schema description contradicted the handler for three cycles

*   **Location**: `codelore-cli/src/mcp.rs::RefactoringTargetsParams::limit`
*   **Severity**: LOW · **Category**: agent-facing contract accuracy
*   **Description**: the doc comment — which becomes the JSON-Schema
    description an agent reads — said "Maximum rows to return (default: all)"
    while the handler resolved an absent limit to 50 via `resolve_row_cap`. An
    agent had no reason to pass `limit` and no reason to suspect the list was
    cut. The trailing disclosure object did fire, so the output was not a lie;
    the *contract* was.
*   **Why three cycles**: it was reported and fixed as prose each time. Nothing
    compared the advertised default against the resolved one, so the next
    parameter added could reintroduce it.
*   **Durable half**: the `tools/list` smoke test now walks every tool's
    `inputSchema`, and for any `limit` property asserts the description neither
    promises "all" nor omits the real default. It runs against the schema the
    agent receives over the wire rather than the source text, because that
    string is the contract. Verified discriminating — restoring the old wording
    fails it by name.

### F292 (Fixed — v0.27.3) — a container tag from the `v1` incident survives in the registry

*   **Location**: `ghcr.io/emrecdr/codelore:v1` (registry state, not source)
*   **Severity**: LOW · **Category**: published-surface debris
*   **Description**: the `v1` tag push that caused F289 triggered
    `container.yml` as well as `release.yml`. Its `type=ref,event=tag` rule
    minted a container tag literally named `v1`, which is still published. It
    resolves to `sha256:0e753ae9…` — an image distinct from both `v0.27.1`
    (`d491a776…`) and `v0.27.2` (`2d7677b1…`) — and it will never update
    again, because `container.yml` no longer triggers on that ref.
*   **Why it matters despite no references**: nothing in the repository points
    at it (the docs use `:latest` and `:vX.Y.Z`), but the Action is documented
    as `uses: emrecdr/codelore@v1`, so `docker pull ghcr.io/…:v1` is a natural
    guess. A tag shaped like a floating major that is frozen forever is worse
    than one that does not exist: the failure is silent.
*   **Full incident inventory** (all four tag rules traced, not assumed):
    `v1` — present, stale, the subject of this finding. `sha-243a85b` —
    collateral: the incident rebuilt the `v0.27.1` commit and moved that
    sha-tag off the genuine release image onto the rebuild, so it now names an
    image that was never published as a release. `latest` — self-healed when
    `v0.27.2` shipped (verified equal to `v0.27.2`'s digest). The two
    `type=semver` rules did not fire, because `v1` is not a semver string.
*   **The cause is already closed**: `container.yml` triggers on `v*.*.*` and
    `tag_trigger_pattern_test` asserts no workflow tag-trigger glob matches the
    Action's major tag. This finding is residue only — it cannot recur.
*   **Deliberately NOT guarded further**: a test that queries the registry for
    stray tags was considered and rejected. It would duplicate a guard that
    already prevents the cause, and it would make a unit test depend on network
    reachability and registry auth — a flaky check for an event that the
    source-level guard makes impossible.
*   **Remediation required a token scope this project's tooling does not
    carry**: GitHub Packages exposes version deletion, not tag deletion, so
    removing `:v1` also removed `sha-243a85b` — acceptable, since both are
    incident artifacts and the genuine `v0.27.1` image is a separate version
    that is unaffected. Deleted from the package UI by the maintainer.
*   **Why the version was hard to find by digest**: `:v1` is a multi-arch
    OCI *index*, and the package UI lists its four child manifests (amd64,
    arm64, and two attestation entries) as separate rows. Searching the page
    for the index digest matches nothing; the index is the parent row, and
    deleting it removes the children with it.
*   **Verified after deletion**: `:v1` and `:sha-243a85b` both return 404,
    while `latest`, `v0.27.2`, `0.27.2`, `0.27`, `v0.27.1`, `0.27.1`,
    `v0.27.0` and `v0.26.0` all still resolve to their original digests. The
    tag count fell by exactly two, so nothing else was caught in the delete.

### F293 (Fixed — v0.27.3) — a failed sqlite export gave no hint about its prerequisites

*   **Location**: `codelore-lib/src/output/sqlite.rs`
*   **Severity**: LOW · **Category**: actionable diagnostics
*   **Description**: the emitter ran `INSTALL sqlite; LOAD sqlite;` inside the
    same batch as the ATTACH and the ten table copies, and mapped any failure
    to `format!("sqlite: {e}")`. `INSTALL` fetches the extension over the
    network on first use and caches it under DuckDB's home directory, so an
    air-gapped or locked-down host failed with a bare DuckDB error labelled
    "sqlite" — indistinguishable from a bug in the export itself.
*   **Resolution**: `INSTALL`/`LOAD` is issued as its own statement so the hint
    attaches to the one network- and filesystem-dependent step, rather than to
    every sqlite error and without pattern-matching DuckDB's error text, which
    would rot. The hint names both prerequisites, the cache location, and the
    two ways out.
*   **Wording corrected by evidence**: the first draft said only "needs network
    access". Reproducing the failure showed the same arm is reached by an
    unwritable cache directory — a permission error, no network involved — so
    the hint names both causes.
*   **Durable half**: a `#[cfg(unix)]` test induces the failure by pointing
    DuckDB's own `home_directory` at an unwritable path, so it needs neither
    network isolation nor a mutation of the process environment. Verified
    discriminating — stripping the hint fails it on the bare error. The
    workspace's `unsafe_code = "forbid"` rejected the first attempt, which set
    `HOME` via `std::env::set_var`; the DuckDB setting is both safe and better
    targeted.

### F294 (Refuted) — `calibrate-defects` mining ingest has no truncation witness

*   **Location**: `codelore-cli/src/calibrate_defects.rs` (carried as cycle-6
    through cycle-8 "M14")
*   **Severity**: LOW · **Category**: guard coverage
*   **Claim**: every other ingest site calls `ensure_ingest_witnessed`; the
    mining ingest does not, so a truncated checkout could calibrate against
    incomplete history.
*   **Refuted — the guard cannot fire here.** `ensure_ingest_witnessed` errors
    only when HEAD resolves *and* `commit_count() == 0`. The scenario it was
    written for is a depth-1 fetch whose tip is a merge, which the **default**
    merge filter drops, leaving zero rows. `calibrate-defects` sets
    `include_merges: true`, so that same commit is ingested rather than
    filtered. Verified empirically: a `--depth 1` clone of a merge-tipped
    repository arrives as a *root* commit (git flattens it — its parent list is
    one token), so the walk yields one commit, never zero. Adding the call
    would be error handling for a state this tool's option set cannot reach,
    which the project's own rules forbid.
*   **What actually bounds the risk** — and it is not the ingest: the tuning
    floor in `defect_calibration::validate` keeps the default weights and
    records the reason (`MIN_LINKED_DEFECTS`, `MIN_IMPLICATED_FILES`) when the
    mined evidence is thin. A shallow checkout therefore produces an artifact
    whose weights are untuned *and say so*, not one that is silently wrong. The
    original finding's parenthetical already noted this; it is the whole answer
    rather than a partial one.
*   **Residual, deliberately not acted on**: the run still spends the full
    mining pipeline before the floor reports thin evidence, so the reason lands
    in the artifact rather than at the point the run was already doomed.
    Surfacing it earlier needs a "minimum commits worth mining" threshold that
    does not exist and would have to be invented — a new tunable for a
    cosmetic gain. Recorded here so a fourth cycle does not re-report the
    witness as missing.

## 10. Cycle-10 surface rotation (F295–F296)

The code had not moved since the prior anchor, so the pass rotated onto the
least-recently-audited surface instead of auditing a delta: the SPA widget
JavaScript, untouched by deep review for five cycles. Both findings below
came out of that rotation, which is the argument for doing it.

### F295 (Fixed — v0.27.4) — stored XSS in the architecture graph and matrix tooltips

*   **Location**: `codelore-lib/src/output/spa/js/40_architecture.js` — the
    force-graph and DSM `tooltip.formatter` bodies
*   **Severity**: HIGH · **Category**: injection / output escaping
*   **Defect**: both formatters concatenated module paths straight into their
    markup — the node name, both edge endpoints, and both axis labels — with
    no escaping. An ECharts *function* formatter's return value is inserted as
    markup, so the token filtering that protects `{b}`/`{c}` templates never
    applied; that these tooltips emit `<br/>` and `<strong>` is the same
    property a payload uses. `<` and `>` are legal in path names on Linux and
    macOS and git tracks them verbatim.
*   **Reproduced, not inferred**: a fixture repository containing a directory
    named with an `<img … onerror=…>` payload, analysed by the *released*
    binary, emitted that payload unaltered into the dashboard's JSON block.
*   **Why the existing defence did not cover it**: the emitter rewrites `</` to
    `<\/` in that block. That is transport-level — it prevents `</script>`
    breakout and has no jurisdiction over what happens after `JSON.parse`,
    where the string carries its metacharacters intact into the next
    concatenation. A payload containing no `</` passes untouched.
*   **Scope, bounded by checking rather than asserted**: only the two tooltips.
    The graph's node labels render as canvas text, and all ten load-time
    `innerHTML` assignments in the file interpolate static strings, theme
    tokens, or generated element IDs — read individually — so there was no
    no-hover path.
*   **Fix**: the five values wrapped in the `escapeHtml` helper every sibling
    widget already used. This file held zero calls against four to fifteen in
    each sibling: the one file that never adopted the convention, not one that
    opted out. Verified by rebuilding and re-emitting against the same fixture.
*   **Exposure**: needs a repository whose paths an attacker controls plus a
    viewer — a hosted scan, or the Action running on a fork pull request. A
    dashboard of one's own repository was latent. Present in released builds
    through v0.27.3.
*   **Class closure**: third appearance of "row data reaches an HTML sink
    unescaped" (an `onclick` attribute two cycles earlier), so the fix ships
    with a guard rather than a fourth point fix. The guard's own first matcher
    reported **zero** violations against the real pre-fix file — it required a
    `+` before the accessor and allowed one member segment — and would have
    merged in the same commit as the fix, passing on the exact defect it was
    written for. Only a negative control caught it. Recorded because the
    near-miss is the reusable part.

### F296 (Fixed — v0.27.4) — a `|` in a git tag corrupts the release-cadence table

*   **Location**: `codelore-lib/src/output/markdown/delivery.rs`
*   **Severity**: LOW · **Category**: output escaping
*   **Defect**: the tag column was written raw while every other cell in the
    same emitter — author, path, metric, caveat — already went through
    `escape_md_cell`. `git check-ref-format` permits `|` in a tag name, and one
    there broke column alignment for any consumer rendering the table.
*   **Fix**: route the tag through the same helper. Markdown is not evaluated,
    so this only ever corrupted layout.

### Deferred from this cycle

*   **A Content-Security-Policy for the dashboard** — researched and
    deliberately not shipped. The SPA is a single self-contained file with
    large inline scripts, so a policy would need `script-src 'unsafe-inline'`,
    which permits inline event handlers and would have given F295's payload no
    protection while reading, in review, like a mitigation. A hash-based policy
    is viable — the template carries zero inline `on*=` attributes, so
    `'unsafe-hashes'` is not required — but needs a per-emit digest of each
    inline block including the interpolated data. That is an emitter change and
    a separate decision. The generalisation worth keeping: **a control that
    cannot block the finding that motivated it is not defence in depth, it is
    decoration** — and the version that is easy to add is exactly the version
    that does not work.

## 11. Cycle-11 audit of the guard cycle 10 asked for (F297–F299)

The delta was small, so the pass adversarially tested the *guard* added by
F295 rather than re-reading the code it protects. Its report reached the
right finding by the wrong route: the mechanism it named was not the
mechanism, and the fix it prescribed closes none of the gap. Both were
corrected by compiling and running the guard, which the audit pass could
not do. The lesson is recorded in F297 because it generalises past this
finding — an audit that cannot execute what it audits will produce
plausible mechanisms, and plausible mechanisms produce fixes that pass
review and change nothing.

### F297 (Fixed — v0.28.0) — the escaping guard could not see the markup its widgets most often build

*   **Location**: `codelore-lib/tests/spa_escaping_test.rs` — `HTML_MARKERS`,
    `RAW_STRING_ACCESSORS`, `is_escaped`
*   **Severity**: MEDIUM · **Category**: test reach / regression detection
*   **Defect**: the guard examines a statement only if it matches one of ten
    markup substrings, and none of them are the tags the widgets build rows
    from. A statement assembling `<h4>`, `<dl>`, `<dt>`, `<dd>`, `<tr>` or
    `<td>` was not markup at all, so *every* accessor in it went unchecked.
    Two narrower defects compounded it: accessors matched by dotted prefix, so
    `.author` does not occur in `main_author` — a payload field rendered today
    — though it does cover `.entity_a`/`.entity_b`, where the base name comes
    first; and a value counted as escaped only when `escapeHtml(` sat directly
    before it, so the second operand of `escapeHtml(a || b)` read as bare.
*   **Scope, measured rather than asserted**: four of the five author sinks in
    the dashboard were invisible. Three failed the markup test; only
    `partnerAuthor` failed for the accessor reason the cycle report named.
    All five were correctly escaped, so nothing was exploitable — what was
    missing was detection of a regression.
*   **Why author names are the sharper vector**: a path needs `<`, legal only
    on some filesystems. `git commit --author` rejects a name beginning with
    `<` outright (`fatal: empty ident name`), but accepts a **quote**
    verbatim in both name and email — an attribute-context breakout needing no
    angle brackets — and author identity is rendered into
    `data-primary-author="…"`. Safe today only because `escapeHtml` covers
    `"` and `'`.
*   **Fix**: markup recognised as an opening tag *inside a string literal*
    (which separates `'<td>'` from a `j < n` comparison and from the literal
    `'<anonymous>'`, both observed false positives); escaping judged by
    walking back to the innermost unclosed `(`, so one call covers every
    operand within it; `_author`/`_path`/`_name` added for qualifier-first
    compounds. All three were needed — the accessor change alone catches
    nothing.
*   **Verified**: with both `main_author` sinks unescaped, the previous guard
    passes and the new one fails naming both lines; the corpus reports zero
    violations either way; the tree was restored byte-identical after each
    run.
*   **Deliberately still out of reach**: a repository string parked in a local
    by one statement and rendered by the next (`partnerAuthor`, `rowAuthor`).
    Every design that reaches them was measured and turns the guard red on a
    clean tree, flagging a comment and a truthiness test. This is the
    across-statement limit the guard's own doc states, now stated explicitly
    rather than by implication.

### F298 (Fixed — v0.28.0) — `preceding` could panic on a multi-byte terminator

*   **Location**: `codelore-lib/tests/spa_escaping_test.rs` — `preceding`
*   **Severity**: LOW · **Category**: correctness (test infrastructure)
*   **Defect**: the identifier-chain start was located with
    `rfind(…).map_or(0, |i| i + 1)`, stepping one byte past a terminator that
    may be several bytes wide, then slicing there. The widgets contain `—`
    and `·`; such a terminator would split mid-character and panic the guard
    rather than report on the file.
*   **Reachability**: none in the current corpus — a probe over every accessor
    match found zero non-ASCII terminators — so it was latent.
*   **Fix**: step by the terminator's own `len_utf8`. Fixed inline rather than
    merely recorded, because widening markup detection (F297) increases the
    number of statements walked, and therefore its exposure.

### F299 (Fixed — v0.28.0) — the comment-hygiene guard could not see hyphen-joined phase markers

*   **Location**: `codelore-lib/tests/comment_hygiene_test.rs` — `line_has_keyword_number`, `line_has_plan_marker`
*   **Severity**: LOW · **Category**: test reach / convention enforcement
*   **Defect**: the guard forbids audit and phase markers in source and enforced
    it two ways — bare `F`/`T`-plus-digits tokens, and a keyword followed by
    spaces then a number. One audit pass joined its name to its number with a
    **hyphen**, which fell between both rules: the token scans split at the
    hyphen into a bare word and a bare digit, neither of which is an ID, and
    the keyword scan stopped at a separator it did not expect.
*   **Scope**: four markers in `analyses/coupling.rs`, `analyses/soc.rs` and
    `output/csv/coupling.rs` — inside the scanned roots, across every cycle
    this guard has run, while it reported clean.
*   **Found by**: validating F279's closure claim. That claim proved *correct*
    for its own scope — all five files it names are clean — but the check that
    established it surfaced a marker family F279 never listed. A finding
    verified rather than assumed is what turned up the one next to it.
*   **Fix**: the keyword rule accepts a single `-`/`_` joiner, and the pass
    name joins `Plan`/`Task` in a named keyword list. The four comments were
    rewritten to state the code-maat compatibility contract they describe
    without the marker — the explanations were correct, only the prefix was
    history.
*   **Verified**: restoring one marker fails the new guard naming its file and
    line, where the previous rule passed; the self-test exercises the joined
    shape for every keyword, so a joiner handled for one and not the others
    cannot pass.
*   **Generalisation, third instance this cycle**: F297 (SPA escaping), F298,
    and now this one are all *a guard narrower than the class it polices,
    reporting clean for the wrong reason*. The common cause is that each rule
    was written from the instances in front of it rather than from the shape
    of the class, and the gap is only ever visible from outside the rule. The
    cheap standing check: for any guard, name a member of its class it would
    not catch — if that is easy, the rule is an instance list.

### Deferred from this cycle

*   **A Content-Security-Policy for the dashboard**, again. The cycle-11
    report re-proposed the `script-src 'unsafe-inline'` policy that the
    cycle-10 note above had already researched and rejected, without citing
    that rejection — the failure mode this ledger exists to prevent. The
    rejection stands. One axis it had not weighed is worth keeping: it scored
    the policy on blocking *execution*, and a `default-src 'none'` policy with
    no `connect-src` and no remote `img-src` also constrains *exfiltration*.
    That is an argument for a policy, not for that policy. **The open decision
    is the hash-based form or nothing.**

## 12. Turning the cycle-11 generalisation on this repository's own guards (F300–F303)

F299 closed with a standing check: *for any guard, name a member of the class
it polices that it would not catch — if that is easy, the rule is an instance
list rather than a rule.* That was written as a lesson. This section is what
happened when it was run as a procedure against all thirteen guards in the
tree. **Six** carry instance lists. The first pass probed four of them —
two confirmed, one refuted, one scoped to a single file — and reported that
as complete. It was not: `doc_analysis_count_test` and
`workflow_signing_isolation_test` were never looked at. An enumeration
reported as exhaustive without being counted is the same defect this
section is about, committed while writing it up, and F303 below is what the
missing two produced.

The refutation matters as much as the confirmations: a check that only ever
confirms is not a check.

### F300 (Fixed — v0.28.0) — the escaping guard did not cover function names

*   **Location**: `codelore-lib/tests/spa_escaping_test.rs` — `RAW_STRING_ACCESSORS`
*   **Severity**: LOW · **Category**: test reach / regression detection
*   **Defect**: `function` was absent from the accessor list. A function name
    is parsed out of the analysed repository's source, so it carries whatever
    that source put in an identifier position — the same provenance as a path,
    and rendered in the same drawer (`12_drawer.js:368` in the X-ray table,
    `:509` in the function list).
*   **Scope**: both sites are correctly escaped, so nothing was exploitable.
    What was missing was detection of a regression.
*   **Fix**: `.function` added; verified by unescaping one site and confirming
    the previous guard passes where the new one fails naming its line.
*   **Also corrected — a doc claim that was not true**: the module doc said the
    accessor list "is derived from the JSON payload's string fields rather than
    from an exemption list, so it does not rot as widgets change." It is
    hand-curated and names 18 of roughly 70 `String` field names across the
    analyses. The curation is defensible — most of the remainder are computed
    values (a band, a verdict, a trend, a date, a revision hash) that cannot
    carry a metacharacter — but "derived, therefore does not rot" described a
    property the code did not have, and that description is why the gap was
    not looked for sooner.
*   **Knowingly left uncovered**: the function-coupling endpoints `a`/`b`.
    Prefix matching makes a single-letter accessor unusable — `.a` would
    swallow `.author`, `.added` and `.arch_band` — so they are unguarded should
    they ever be rendered rather than used as lookup keys, as they are today.

### F301 (Fixed — v0.28.0) — the ledger-stamp guard could not see a compound stamp

*   **Location**: `codelore-lib/tests/ledger_stamp_test.rs` — `UNRELEASED_MARKS`
*   **Severity**: LOW · **Category**: test reach / ledger integrity
*   **Defect**: the guard matched two exact spellings of an unreleased stamp.
    A compound stamp naming both a shipped release and pending work matched
    neither.
*   **Not hypothetical**: F279 carried exactly that shape and claimed
    unreleased work for two releases after it shipped — invisible to the guard
    written to catch precisely that claim. The stamp was corrected by hand
    before this guard was; fixing the row without fixing the rule is the
    pattern the guard's own module doc warns about.
*   **Fix**: the rule now asks whether a stamp *line* — a `### F…` heading or a
    `**Status**:` bullet — mentions the unreleased state, rather than matching
    spellings. Scoping to stamp lines keeps prose discussing the section from
    counting; the self-test pins both directions.

### F302 (Fixed — v0.28.0) — the release publish gate reads one file and three markers

*   **Location**: `codelore-lib/tests/release_publish_gate_test.rs`
*   **Severity**: MED · **Category**: CI safety / guard scope
*   **Defect**: the guard's own doc states it is "a *detector*, not a list of
    job names… A publishing job added later is therefore covered without anyone
    remembering to update a list here." Both halves of that are narrower than
    claimed: it reads `release.yml` and nothing else, and its
    `PUBLICATION_MARKERS` names three mechanisms — `action-gh-release`,
    `cargo publish`, `git push`. Publishing a container image matches none of
    them.
*   **What is outside it**: `container.yml` pushes to ghcr via
    `docker/build-push-action` with `push: true`, and runs on
    `workflow_dispatch` as well as `v*.*.*` tags. Its `latest` tag is correctly
    gated on `github.ref_type == 'tag'`, but `type=sha,prefix=sha-` is not, so
    a manual dispatch publishes a real `sha-<short>` tag to the public
    registry. `attest-digest.yml` likewise holds `packages: write`.
*   **Filed as a decision, then decided** (§13): whether a dispatch-time
    `sha-` tag was a hazard or a deliberate traceability affordance was a
    design call, and the precedent argued for looking — this guard exists
    because `release.yml`'s `workflow_dispatch` was documented as a dry run
    and published a real GitHub Release and a Homebrew formula.
*   **Resolution**: publishing was made a property of the ref rather than the
    trigger. Every job in `container.yml` now requires a tag ref, so
    dispatching from a tag still publishes — the retry path for a publish
    that failed after the tag was pushed — while dispatching from a branch
    skips. The guard's own scope is unchanged and remains narrow: it still
    reads one file and three markers, and a publishing workflow added
    elsewhere is still outside it. That half is open.

### F303 (Fixed — v0.28.0) — the signing-isolation guard recognised one of three ways to grant a scope

*   **Location**: `codelore-lib/tests/workflow_signing_isolation_test.rs` — `parse`
*   **Severity**: MED · **Category**: CI safety / guard reach
*   **Defect**: the guard keeps the release pipeline at SLSA Build L3 by
    refusing to let any job running repository-authored code hold
    `id-token`/`attestations`. It found those scopes by reading a
    `permissions:` block line by line, and GitHub accepts two further
    spellings of the same grant: `permissions: write-all`, which grants every
    scope including the signing ones while naming neither, and the flow map
    `permissions: {id-token: write}`, which names them but not on lines of
    their own. Both parsed to zero scopes.
*   **Why `write-all` is the sharp one**: it is the first thing anyone reaches
    for when an attestation step fails for want of a permission. The blind
    spot therefore sat exactly where the pressure to use it is highest, and
    taking it would have dropped the pipeline to L2 while changing no visible
    output — which is verbatim the regression the guard's own module doc
    calls "silent and attractive".
*   **Scope**: no workflow uses either form, so nothing was ungated. What was
    missing was the tripwire.
*   **Fix**: a `permissions:` line carrying its value inline is parsed —
    `write-all` expands to the signing scopes, a flow map contributes its
    keys, and `read-all` contributes nothing.
*   **Verified end to end rather than at the parser**: rewriting
    `container.yml`'s build job to `permissions: write-all` fails the guard
    naming the job and both implied scopes, where it previously reported
    nothing; the workflow was restored byte-identical. `read-all` is pinned
    as granting nothing signable, so the safe shorthand does not start
    failing every workflow that uses it.
*   **Correctly handled already, and worth recording**: the guard reads
    workflow-level `permissions:` and flags a top-level grant that every job
    would inherit. That half of the inheritance question was never the gap.

### Probed and refuted

*   **Widening the comment-hygiene phase-keyword list.** `Tier`, `Phase`,
    `Step` and `Day` all appear followed by digits (150, 6, 83 and 7
    occurrences), which looks like the same class F299 closed. It is not:
    they name Tier-1 languages, algorithm phases, algorithm steps, and a
    fixture's day-by-day timeline — **current contract, not development
    history**. A keyword list cannot draw that distinction, so widening would
    fail correct comments. Recorded because the next person to run this check
    will find the same 150 hits and needs to know they were looked at.

    A later attempt at the general rule put a number on it: *capitalised word
    joined to a number* produces **924 hits over the scanned roots, every one
    a false positive* — `Tier1` ×85, `Tier-1` ×64, `Sha2` ×39, `UTF-8` ×26,
    `Step 1..6` ×~80, `Type 1/2/3` ×~40, SQL keywords (`ELSE 0`, `LIMIT 1`),
    academic citations (Tornhill 2011, Coleman 1994), conference years
    (ICSE/MSR/FSE 2xxx), and dates. The banned class is semantic — development
    history versus current contract — and only a closed keyword vocabulary can
    express it.

*   **`doc_analysis_count_test`'s `docs/`-only scope.** Its two lists are
    documented *exclusions*, not an enumeration of the policed class, so it is
    a different shape from the others here. The obvious class member outside
    its scope is a stale analysis count in a tracked file above `docs/`. There
    is none: `README.md` states no count, and the counts that do exist outside
    the scan are in `CHANGELOG.md`, where historical numbers are correct by
    design. No live gap.

## 13. Backlog decisions, researched (F304 onward)

Every item that had been sitting as "open, user's call" was researched against
current tooling and standards and decided. Three produced work; four are
deferred with a stated reason rather than left ambiguous; one is closed.
Deferral here means *decided not to do now, for this reason* — not
undecided.

### F304 (Fixed — v0.28.0) — three workflow steps substituted a ref into a shell

*   **Location**: `ci.yml` (PR diff summary), `release.yml` (tag resolution)
*   **Severity**: MED · **Category**: injection / CI
*   **Defect**: `github.base_ref`, `github.ref_name` and `github.ref_type`
    were substituted directly into `run:` blocks. An expression is
    substituted into the script *text* before bash parses it, so a shell
    metacharacter in the value is executed rather than quoted.
*   **Why the vector is real here**: `git check-ref-format` permits far more
    than it appears to, and this repository already carries F296 — a `|` in a
    tag name corrupting a Markdown table. The same permissiveness that
    produced a broken table produces a shell injection when the value reaches
    a `run:` block instead of a formatter.
*   **Fix**: all three pass through `env:` and are read as quoted shell
    variables. `zizmor` template-injection findings 3 → 0.
*   **Caught by the tree's own guard, mid-fix**: the first draft explained the
    change by writing the expression syntax literally in a comment.
    `workflow_expression_test` failed it — GitHub's evaluator reads `#`
    comments, so that would have made both files unloadable and skipped every
    step in them. Worth recording because the guard caught a defect in the
    commit that was hardening the same files.

**F302 — decided and closed: publishing is a ref property, not a trigger
property.** (Stated here rather than under a second `F302` heading: the
first draft of this section repeated the ID as a heading, leaving the ledger
asserting the same finding was both `Active` and closed. A findings ledger
whose IDs are not unique cannot be read by ID, which is the only way anyone
reads it.)

The container workflow's tag rules are all release-shaped except `type=sha`,
which matches any ref, so a dispatch from a branch built, pushed, tagged and
attested a genuine publicly-pullable image from unreleased code.

A build-only dry run was considered and rejected: it breaks the
digest → merge → attest chain and would need the three-job pipeline
restructured, which cannot be exercised from a working copy. Every job now
requires a tag ref. **Dispatching from a tag still publishes** — that is the
retry path when a publish fails after the tag is pushed — and dispatching
from a branch skips. The condition is repeated per job rather than inherited
through `needs:`, on the same reasoning `release_publish_gate_test` already
applies to its own gate.

### Deferred, with reasons

*   **crates.io Trusted Publishing — deferred, and not for effort.** It is GA,
    with an official `rust-lang/crates-io-auth-action`, and `zizmor`
    independently recommends it. It also requires `id-token: write` on the job
    that runs `cargo publish` — and `cargo publish` builds the crate, which
    executes `codelore-lib/build.rs`. Repository-authored code holding an OIDC
    token can request *any* audience, sigstore included, which forges the
    provenance this pipeline is built to make unforgeable. Minting the token
    in a separate job does not help: job outputs are not masked, and the
    action revokes the token when its own job ends. `cargo publish
    --no-verify` would avoid executing repository code and make adoption
    viable, at the cost of publish-time verification and a new distinction the
    isolation guard would have to learn. **Adopting it as written would trade
    Build L3 for the removal of one long-lived secret. Not a drop-in, and the
    trade is the wrong way round.**
*   **`zizmor` — adopt, in two stages; stage one landed.** It is the de facto
    standard for Actions static analysis. Run against this tree it reports 65
    findings: 34 `unpinned-uses`, 15 `artipacked`, 4 `excessive-permissions`,
    3 `template-injection`, 2 `dangerous-triggers`, and one
    `use-trusted-publishing`. The template-injection three are fixed above.
    Gating CI on it needs a **policy decision that is genuinely the
    maintainer's**: 34 of the findings are `actions/*` and
    `dtolnay/rust-toolchain` referenced by tag, and the latter *must not* be
    SHA-pinned — its tag names the toolchain, not the action version. Adopting
    the gate means either SHA-pinning the `actions/*` set or configuring the
    audit to accept them, and that choice belongs to whoever owns the
    supply-chain policy. The `dangerous-triggers` pair is
    `pull_request_target` + `workflow_run` in the Dependabot workflow, already
    mitigated by never checking out PR code and documented as such.
*   **Rust 1.97.1 — deferred to the next release cut, by the documented
    convention.** Stable is 1.97.1 (2026-07-16); the pin is 1.96.0. The bump
    is coordinated across four pin sites and a CHANGELOG entry at cut time,
    and a toolchain bump lands new clippy lints against a `-D warnings` gate —
    a mid-cycle bump would leave that work unreleased and the sites liable to
    drift. Due, not urgent.
*   **A hash-based CSP for the dashboard — still the open recommendation, and
    the one item where a fact was worth re-checking.** The ledger's claim that
    the template carries zero inline `on*=` handlers is **correct** — verified
    directly, which matters because a first grep suggested ten and the pattern
    was at fault, not the claim. So `'unsafe-hashes'` is genuinely not
    required and the hash form is viable. It remains an emitter change: a
    per-emit digest of each inline block, including the interpolated data.
    With sink escaping now guarded on three axes, the marginal value is the
    exfiltration half alone, which is real but no longer urgent.

### Closed

*   **F215 — closed, not fixed.** One `unreachable!` remains in the CLI, in
    `analyze.rs`, carrying a message that states its own invariant. The
    `enum Format` refactor was proposed when the same pattern appeared in
    roughly eleven dispatchers; at one site it no longer carries its own
    weight.

### F305 (Fixed — v0.28.0) — the bot-filter guard matched one spelling of a rule SQL gives four

*   **Location**: `codelore-lib/tests/bot_filter_hygiene_test.rs`
*   **Severity**: LOW · **Category**: test reach / correctness
*   **Defect**: the guard forbids collapsing bot-ness per canonical identity
    outside `query.rs`, and looked for two exact literals —
    `BOOL_OR(is_bot)` and `HAVING NOT BOOL_OR` — case-sensitively. SQL is
    case-insensitive and indifferent to whitespace, so `bool_or(is_bot)`,
    `BOOL_OR( is_bot )` and `BOOL_OR(a.is_bot)` are the same query and the
    same misclassification, and all three were invisible.
*   **Found by**: the cycle-12 audit, against §12's own census — which had
    missed this guard entirely. The census enumerated guards carrying
    instance lists by looking for a `&[&str]` const; this guard holds its
    instances as inline literals in a `contains()` call, so the search found
    one *syntactic shape* of instance list rather than the class. Third
    instance of the same defect this cycle, and the first one inside the
    procedure built to detect it.
*   **Scope**: the tree is clean — the only occurrences are `query.rs`'s
    explanatory doc comment, which is the documented exemption. What was
    missing was detection.
*   **Fix**: matched against a normalised copy (lowercased, whitespace
    removed) with the column qualifier stripped, so the guard tests the
    query's meaning rather than its spelling.
*   **Verified**: a comment reading `bool_or( a.is_bot )` — lowercase,
    spaced and qualified at once — now fails the guard naming its file and
    line, where the previous matcher saw nothing; the self-test pins all four
    spellings and three non-collapses.
*   **Refuted while checking, and recorded**: cycle-6's M17 also claimed
    `pair_programming.rs` keeps a Rust-side bot filter the guard cannot see.
    That file now carries an explicit rationale — it reads `commits` directly
    rather than through `HUMAN_ALIASES_CTE`, so its `is_bot` checks are the
    only thing keeping bots out of the pair counts. A documented design
    decision, not an oversight.

### F306 (Fixed — v0.28.0) — zizmor adopted, and the seven findings it still reports

*   **Location**: `.github/zizmor.yml`, `ci.yml`
*   **Severity**: MED · **Category**: CI coverage / supply chain
*   **Why now**: `zizmor` is the de facto standard auditor for GitHub
    Actions, and running it against this tree found three live
    template-injection sites (F304) that nothing here would have caught. That
    is the argument for adopting it: not that the tool is popular, but that
    it found something on first contact.
*   **Configured, not silenced**: exactly one audit is configured.
    `unpinned-uses` is given the policy this repository *already* enforces in
    `workflow_action_pin_test` — SHA for third-party actions, tag permitted
    for `actions/*` and `dtolnay/rust-toolchain`. Two gates disagreeing about
    pinning would be worse than either alone, because a contributor would be
    told to pin by one and told it is fine by the other. That drops the count
    from 111 findings / 44 high to 74 / 7 without suppressing a single
    finding.
*   **On the `actions/*` divergence**: OpenSSF Scorecard asks for first-party
    actions to be SHA-pinned too, and the tj-actions and reviewdog
    compromises are why. The exemption is kept because a compromise inside
    GitHub's own namespace is a compromise of the platform running the job,
    which a pinned SHA does not survive either — and because the repository
    made this call explicitly, with that reasoning recorded in the guard.
    Overriding a documented, guarded policy on a general principle, when the
    stricter half is already enforced, is not an improvement.
*   **Blocking, not advisory** — the first draft of this job was advisory
    during bake-in, on `dogfood`'s pattern. Running it proved that wrong: with
    findings outstanding the check is red on every pull request, and a
    permanently red check teaches people to ignore red checks. That is worse
    than not running the tool. So the seven were resolved instead, and the
    gate is real:
    *   **`excessive-permissions` on `release.yml` — fixed.** The
        workflow-level default was `contents: write`, inherited by `plan`,
        `crates-publish` and `homebrew-publish`. None of them write to this
        repository: `plan` reads the ref, `crates-publish` authenticates to
        crates.io with a token, and `homebrew-publish` checks out the tap with
        its own deploy key and pushes there. The default is now `contents:
        read`; `release`, which creates the GitHub Release, already declared
        its own write.
    *   **`dangerous-triggers` ×2 and `excessive-permissions` ×3 on the
        Dependabot auto-merge workflow — written exceptions.** The triggers
        are what that pattern is, mitigated the documented way: neither stage
        checks out or executes pull-request code. The three write scopes are
        the capability itself — approving, merging, and re-dispatching CI —
        and cannot be narrowed without removing it.
    *   **`cache-poisoning` — written exception.** The audit rates its own
        confidence Low, and a cache entry is written under the ref that
        produced it, so poisoning what a tag-triggered release build restores
        needs push access to the repository.
*   **Exceptions live on the line that raises them**, as
    `# zizmor: ignore[rule]` comments with the reasoning beside them, rather
    than as line numbers in a config file that drift the moment the file is
    edited.
*   **Gated at `high`.** The 16 remaining `low` findings are `artipacked` —
    `actions/checkout` persisting credentials — and deserve their own pass
    rather than a blocking gate adopted in the same commit as the tool.

### F307 (Fixed — v0.28.0) — the bot-filter matcher read one line at a time, and the codebase wraps long SQL

*   **Location**: `codelore-lib/tests/bot_filter_hygiene_test.rs`
*   **Severity**: LOW · **Category**: test reach / correctness
*   **Defect**: F305 fixed the spelling axis — case, whitespace, table
    qualifier — but normalised each line independently. A call wrapped across
    lines is never assembled, so its argument is never seen. Confirmed by
    planting `BOOL_OR(` with `a.is_bot` on the next line inside a SQL string
    literal: the guard passed.
*   **Not a hypothetical layout**: it is the house style in the very directory
    the guard scans. `analyses/ownership.rs` writes `SUM(` on one line, its
    argument on the next, and the closing paren on a third. A `BOOL_OR(` whose
    argument grew long enough to wrap would be written the same way by the
    same convention.
*   **Fix**: normalise the whole file once and map matches back to a file line
    by counting newlines in the prefix — the technique `spa_escaping_test`
    already uses to report a file line from a statement offset.
*   **A bug found while fixing it, worth recording because it is the same
    class**: the first implementation indexed its offset table per *character*
    while `match_indices` returns *byte* offsets, so the table drifted on any
    file containing a multi-byte character — and these files contain them in
    their prose. The guard detected the right thing and named the wrong line
    (`soc.rs:142: _bot` instead of `soc.rs:141: BOOL_OR(`). A guard that names
    the wrong line is a guard people stop trusting. Caught because the
    regression test read the reported location rather than only the pass/fail.
*   **Verified**: wrapped SQL now fails naming the construct's own line; all
    four single-line spellings still caught; the tree is clean either way.

### The Trusted Publishing deferral, corrected

Cycle 13 tested the premise of §13's deferral and it does not hold as
stated. The claim was that Trusted Publishing needs `id-token` on a job that
runs `cargo publish`, which builds the crate and therefore executes
`build.rs`. Only the *verification* step builds, and it is switchable:

| command | `build.rs` executed |
|---|---|
| `cargo package --no-verify` | no |
| `cargo package` | yes |

Reproduced here independently on `cargo 1.97.1` (the cycle used 1.95.0), with
the control that matters — a "no" with no corresponding "yes" would only mean
the probe was broken.

So the architecture is compatible: the build job keeps verification and runs
repository code with no token, and a publish job holding `id-token` runs
`cargo publish --no-verify`, executing none. **The deferral stands, but the
reason was wrong.** The real blockers are that Trusted Publishing must be
configured per-crate on crates.io — an action outside this repository — and
that switching the workflow before that configuration exists breaks the next
release. Sequencing, not incompatibility. Recorded so the next cycle does not
re-derive a resolved argument.

### F308 (Active) — the comment-hygiene guard cannot see manifests, and phase markers live there

Found while validating cycle 19, whose §3 concerns a scanner that is blind to
one syntactic form. The same shape appears one layer over: the comment-hygiene
guard forbids phase markers and audit IDs, and enforces it across `.rs`/`.sql`
under `crates/codelore-(lib|cli)/(src|tests)`. `Cargo.toml` is not in that set —
neither by extension nor by path, since manifests sit at crate root rather than
under `src`/`tests`.

Manifests carry prose comments of exactly the kind the guard exists to police,
and so does the markdown this project publishes. A census across every manifest
and the published markdown found seven marker sites, all since fixed by hand:

| file | marker |
|---|---|
| `Cargo.toml` | a spec-section reference, plus three version numbers, on the PGO profile |
| `crates/codelore-lib/Cargo.toml` | a plan-number reference on the tree-sitter grammar block |
| `crates/codelore-cli/Cargo.toml` | a finding ID plus a tier/day marker on `clap_complete` |
| `crates/codelore-rca/Cargo.toml` | a spec-section reference on the tree-sitter core pin |
| `crates/codelore-rca/UPSTREAM.md` | a section *titled* after a plan step, a second plan-step reference, and a spec-section reference |

An eighth, on the `diff`-support dependency block, was removed in the cycle-19
landing because that comment had to be rewritten anyway — it still named a
dependency the manifest no longer declares.

The instances are closed; the mechanism is not, which is why this stays Active.
The `UPSTREAM.md` row is the one that matters most: `codelore-rca`'s manifest
sets `readme = "UPSTREAM.md"`, so those three markers were on the crate's
crates.io page. The doc guards do not reach it either — `documented_action_ref_test`
scans `README.md` plus `docs/**`, `doc_analysis_count_test` scans `docs/**`
alone, and no test in the workspace names `UPSTREAM.md`. The axis the original
finding missed is therefore not extension-or-path but **published versus
internal**: the one markdown file this workspace publishes outside `docs/` is
the one file no convention guard reads.

*   **Impact**: documentation-only. Nothing is misbuilt; the markers rot exactly
    as the guard's own module doc describes, and mean nothing to a reader
    without the report they came from.
*   **Class**: this is the guard-narrower-than-its-class defect the SPA
    escaping guard, the bot-filter guard, and the ledger-stamp guard each
    exhibited — a guard reports clean because the rule is narrower than the
    convention it claims to enforce, and the gap is only visible from outside
    the rule.
*   **Fix**: scope alone is not enough, and writing the finding scope-first
    nearly hid that. Extending the walk to `Cargo.toml` catches the plan-number
    marker and **not** the other one, because the matcher cannot express it
    either:
    - **Scope** — add manifests as scan targets.
    - **Vocabulary** — `PHASE_KEYWORDS` carries `Plan`, `Task`, `DEEP`; the
      surviving marker spells its phase with two words that are in neither the
      keyword list nor any other predicate.
    - **ID shape** — the finding ID is hyphenated with a letter between the
      prefix and the digits, so the token scan splits it into a bare
      single-letter token and a letter-led token, and the whole-token ID rule
      rejects both on length and on prefix.

    - **Surface** — derive the scanned set from the `readme` targets of all
      four manifests plus `docs/**`, so a newly published file is covered by
      construction rather than by someone remembering to add it.

    The guard's self-test convention applies — pin the widened matcher against
    each of the shapes the previous rule missed, calling the guard's own matcher
    rather than a copy, so the extension is proved rather than asserted.
    `tests/bot_filter_hygiene_test.rs` is the worked example to copy: its
    self-test exercises the matcher the guard actually runs.
*   **Caveat for the fix**: the exclusion must be drawn by **provenance, not by
    crate**. `codelore-rca` is hands-off as a vendored MPL fork, and its
    manifest carries grammar-pinning comments referencing upstream issue
    numbers that must keep passing. But `UPSTREAM.md` does not exist upstream at
    all — it is codelore-authored fork-provenance documentation — so a
    crate-wide exclusion would permanently exempt the most visible surface in
    the workspace. Exempt upstream-derived text, not the crate that contains it.

Recorded rather than fixed inline, per the standing rule that latent findings
spotted during unrelated work land as findings.

### F309 (Fixed — Unreleased) — the vendored fork's public surface still describes languages it no longer parses

Found in the same validation pass as F308. Originally deferred whole; cycle 20
established that its two halves do not share a cost, and the cheap half has
since landed.

*   **`src/lib.rs` — FIXED.** The crate rustdoc's "Supported Languages" list
    named eleven languages against five parsed. The split is four never-present
    (C#, CSS, Go, HTML — `UPSTREAM.md` has no hit for any of them) plus two
    removed (C++, and the Firefox-internal JavaScript dialect), so this was
    upstream residue compounded by this project's own excisions rather than
    either alone. It was live on docs.rs, and it directed bug reports to a
    project that cannot act on them. The block now names the five parsed
    languages, states which upstream entries do not apply and why, and points at
    this repository; `UPSTREAM.md` records the divergence.
*   **`src/spaces.rs` — FIXED.** `SpaceKind::Namespace` (documented as "A
    `C/C++` namespace") and `SpaceKind::Struct` were unconstructible. Their sole
    producer was `impl Getter for CppCode`, removed with the grammar excision —
    `StructSpecifier => SpaceKind::Struct` and `NamespaceDefinition =>
    SpaceKind::Namespace`. Confirmed by an adversarial pass that closed every
    other construction route: `SpaceKind` derives `Serialize` but **not**
    `Deserialize`, so no runtime value can be built from data; it has no
    `FromPrimitive`, no `From`/`TryFrom`/`FromStr`; `Default` resolves to
    `Unknown`; no macro expansion emits it; and the union over all six `Getter`
    impls is `{Unknown, Function, Class, Unit, Interface, Trait, Impl}`.

    Four dead match arms remain, not two — the earlier phrasing named only the
    `Display` impl:

    | file | variant |
    |---|---|
    | `codelore-rca/src/spaces.rs:54` | `Struct` |
    | `codelore-rca/src/spaces.rs:58` | `Namespace` |
    | `codelore-lib/src/complexity/mod.rs:56` | `Struct` |
    | `codelore-lib/src/complexity/mod.rs:60` | `Namespace` |

    The two in the product are the ones worth knowing about: `space_kind_str`
    carries arms for values its own dependency can no longer hand it.

*   **Impact**: dead public API. Nothing miscomputed — the unreachable variants
    cost two match arms each and a line of `Display`.
*   **Resolution**: both variants and all four arms removed together, and
    confirmed the way the deferral said it must be — by deleting them and
    building, not by re-asserting the reachability argument. `cargo check
    --workspace --all-targets --all-features` is clean. This is a breaking
    change to `codelore-rca`'s published API and lands unreleased, the same
    shape as the grammar excision that orphaned the variants in the first place.
*   **What made the removal takeable.** Two costs assumed large were measured,
    and both were smaller than the original wording implied:
    - **Blast radius is one crate.** `codelore-rca` has exactly one reverse
      dependency on crates.io, and it is `codelore-lib` — this same workspace.
      No external consumer is protected by waiting. The caveat is that
      `SpaceKind` is re-exported from the crate root, so an unknown direct user
      could construct either variant today.
    - **Deprecation is not a middle path here.** `#[deprecated]` is a minor
      change under the Cargo semver rules and would normally be the graceful
      option. But the `deprecated` lint fires on *pattern* matches,
      `codelore-lib` carries two such arms, and CI runs `-D warnings`. Marking
      the variants would break the build, and the only way through is
      `#[allow(deprecated)]`, which is masking rather than fixing. The choice is
      removal or documentation, with nothing in between.
*   **Still open, tracked at [F310]**: pairing the language list with the
    supported-extension set so the two cannot disagree again. The removal
    closes the dead-variant half; nothing yet stops the *next* divergence
    between what the crate claims and what it parses.

### F310 (Active) — the supported-language set and the grammar pins are hand-copied across many sites, and the one guard-shaped mechanism this repo favours is not applied to either

Surfaced by a cleanup review of the stale-claim fix. That fix repaired seven
prose assertions by hand and shipped no mechanism, which is one altitude too low
for a repo carrying fifteen convention-scanning guard tests.

**The language set** is written out by hand in at least eight places — the
`profile` command's pinned-third-party line, an MCP tool description, the
complexity module's rustdoc, the `UPSTREAM.md` grammar table, `codelore-rca`'s
crate rustdoc, the advanced-usage docs, and two separate lists in the workspace
`README.md`. The count is a floor on purpose: it was first written as six, and
a later pass found the two README sites — the highest-traffic surface of the
set. A hand-written census of hand-written facts reproduces the defect it
documents, so the guard below should enumerate the sites rather than a person.
None is compiler-checked and none is tested. Two were stale before the fix; a third (`codelore-rca`'s rustdoc) was
still stale until [F309] closed it.

**The grammar pins** are written out in four places — `codelore-rca`'s manifest,
`codelore-lib`'s manifest (declared to exist "for parser-ABI compat", so the two
*must* agree), `provenance::grammar_pins()`, and the `UPSTREAM.md` table. All
four agree today and nothing checks that they continue to.

*   **Why this is worse than ordinary duplication**: a grammar-version mismatch
    does not fail loudly. The node-ID tables in the generated enums are keyed to
    exact grammar versions, so a partial bump yields *silently wrong complexity
    metrics*. And `grammar_pins()` is serialised into every provenance manifest,
    so the same drift ships a provenance receipt that misreports what produced
    the numbers.
*   **The asymmetry that makes the case**: in the profile command's own output
    line, the `gix` and DuckDB versions come from constants that
    `dep_versions_drift_test.rs` checks against `Cargo.lock`, and the analysis
    count and format list on adjacent lines both derive from code. The
    tree-sitter version and the language list on that same line are the only
    hand-written facts in the function — and they are the ones that went stale.
*   **The fix that matches the defect — do this one first**: a pin-agreement
    guard on the `rust_version_pins_test.rs` template. That test guards the same
    shape (one fact, several independent pin sites, no single source of truth)
    and carries the anti-vacuity assertion such a guard needs. It should assert
    the two manifests name the same grammar set at the same versions, and that
    the published table has exactly one row per declared grammar. A self-test
    must reject an *extra* table row naming a dropped grammar, since that is the
    defect that actually occurred. The ordering here matters and was originally
    inverted: what shipped was **set** drift — a published table naming four
    grammars the manifest no longer declares — not version drift.
*   **The cheaper adjacent fix**: extend `dep_versions_drift_test.rs` to
    value-check `grammar_pins()` against the lockfile. The existing test file
    already has the helper; the current provenance test asserts only that the
    keys are present and non-empty, explicitly declining to check values. Worth
    doing — but on its own it guards agreement among sites that already agree,
    a drift that has not occurred here, so taking it first would close the cheap
    half and leave the half that actually bit.
*   **On deriving the language list**: `Tier1Language` is already imported in
    `codelore-cli`, so no new dependency is needed — but it exposes no `ALL`, and
    its `as_str` deliberately collapses `Tsx` onto the TypeScript label (pinned
    by a test), so deriving needs a display name distinct from the grouping
    label. That is a public-API addition and a user-visible output change, not a
    cleanup, which is why it is recorded here rather than done in passing.

### F311 (Fixed — Unreleased) — three extension tables claim to mirror each other and do not

Found while assessing [F310]. `Tier1Language`, `CloneLanguage` and
`ImportLanguage` each map a file extension to a language, and the latter two
carry comments stating they match the first. Two real divergences:

| | `.pyi` | extension case |
|---|---|---|
| complexity | accepted | lowercased before matching |
| imports | accepted | matched raw |
| clones | **not accepted** | matched raw |

*   **Effect**: a `.pyi` stub is complexity-scanned and import-scanned but never
    clone-scanned; a file named with an upper-case extension is
    complexity-scanned but skipped by both clones and imports. Both failures are
    silent — each table returns `None` and its pass moves on.
*   **Impact**: low in practice, since upper-case source extensions are rare and
    `.pyi` stubs carry little clonable body. It is recorded because the comments
    assert an equivalence that does not hold, which is the same defect class as
    [F310] one layer down: a claim of correspondence with nothing checking it.
*   **Fix**: one shared extension→language mapping, or a test asserting the three
    tables accept identical extension sets. Pre-existing; not introduced by the
    work that found it.

### F312 (Active) — `codelore-rca` compiles a dispatch layer the product cannot reach

Found by an efficiency review of the unreleased range, which asked whether that
range's dependency work reduced any build cost and concluded it did not — by the
commits' own accurate admission, the removals changed nothing about the build.
The class that *does* carry cost is the one the grammar excision closed,
referenced-but-unreachable, and the fork still contains a live specimen of it.

The entire consumed surface of `codelore-rca` is a single import: `complexity/mod.rs`
takes `FuncSpace`, six parser types, `ParserTrait`, `SpaceKind` and `metrics`.
Nothing else in the workspace names the crate. The `Callback` dispatch machinery
— `action::<T>` in `macros.rs`, and every `Callback` impl behind it in `spaces`,
`comment_rm`, `output/dump`, `count`, `function`, `find`, `ast` and `ops` — has
no call site outside the crate, so those impls are dead as produced values.
`concurrent_files.rs` is reachable only from its own `mod` and `pub use` lines.

*   **Impact**: build time only; nothing miscomputes. Confirmed in `Cargo.lock`
    that three crates have `codelore-rca` as their sole reverse dependency and
    would leave the graph with those modules — `crossbeam` (and `crossbeam-queue`
    beneath it), `termcolor`, and `num-format`. `walkdir` and `globset` would
    **not**: `codelore-lib` declares both directly.
*   **Honest magnitude**: four small pure-Rust crates plus roughly 2,400 lines of
    source, on every build and every CI leg. Seconds, not the minutes the grammar
    excision bought. Recorded because it is strictly more than the zero that range
    delivered, and it earns the same argument the range made for its own removals:
    a vendored crate's manifest and module tree should describe what the fork
    actually needs.
*   **Why not fixed here**: removing modules and the `Callback` surface is a
    breaking change to a published crate — the same shape, and the same
    version-bump requirement, as the grammar excision. It also needs a build to
    verify, which the pass that found it could not afford.
*   **Relationship to [F309]**: strictly larger and better evidenced. F309 covers
    one orphaned `SpaceKind` variant and the crate rustdoc; this covers the
    dispatch layer both of those sit inside. If the fork's divergence budget is
    opened for one, it should be opened for both in the same cut.

### F313 (Active) — `tempfile` is declared twice in `codelore-cli`

`codelore-cli` declares `tempfile = "3"` in `[dependencies]` and again in
`[dev-dependencies]`. Normal dependencies are already available to test and bench
targets, so the second declaration changes nothing — the same
declaration-that-changes-nothing class the range removed four lines above it when
it dropped `dirs`.

*   **Impact**: none at build time; manifest hygiene only.
*   **Why not fixed here**: the range's own standard for a removal is *delete the
    declaration and build*, not *search for the name* — that distinction is the
    stated lesson of the `dirs` and `num-traits` entries. The pass that found this
    was at 98% disk on a shared cargo target and could not meet that standard.
    Applying a weaker one, in a review of the range that set it, would be the
    wrong trade. Pre-existing; not introduced by the work that found it.

### F314 (Fixed — Unreleased) — `unsafe_code = "forbid"` does not cover the crate the docs say it covers

`CLAUDE.md` states the invariant as **`workspace.lints.rust: unsafe_code =
"forbid"`** — "zero `unsafe` blocks; CI rejects additions." The first clause is
true. The second is false for one crate in three.

`codelore-lib` and `codelore-cli` opt in with `[lints] workspace = true`.
`codelore-rca` carries a deliberately empty `[lints]` table, commented
"Don't apply workspace lints to vendored MPL files — keeps upstream-merge
friction low." Cargo's `[lints]` inheritance is all-or-nothing per crate, so
declining the workspace clippy block also declines `unsafe_code = "forbid"`.
The crate that wraps the tree-sitter grammars is the one where the lint does
not run.

*   **Impact**: none today. A search across `crates/` finds zero executable
    `unsafe`; the three textual hits are doc comments explaining why
    `unsafe { env::set_var }` was avoided. The defect is that a guarantee is
    asserted more broadly than it holds, so `unsafe` added to `codelore-rca`
    would pass CI in silence.
*   **Class**: guard narrower than its claimed class — the same shape as [F308],
    the bot-filter guard and the SPA escaping guard. The rule reports clean
    because its scope is smaller than the claim made for it, and the gap is
    invisible from inside the rule.
*   **Why the policy is not the problem**: declining workspace *clippy* lints on
    vendored MPL code is correct and should stay. `unsafe_code` is a different
    kind of rule and was dropped along with the style lints only because
    inheritance is per-crate and all-or-nothing.
*   **Resolution**: the crate now carries `[lints.rust]` with
    `unsafe_code = "forbid"` directly, which restores the invariant without
    inheriting the clippy block — the merge-friction policy is unchanged and
    still documented on the line above it. Confirmed inert by building:
    `cargo check -p codelore-rca --all-features` is clean, as expected for a
    tree with no executable `unsafe`.
*   **Consequence worth knowing**: `forbid` cannot be locally overridden, so
    `unsafe` arriving from upstream in a future sync becomes a hard error rather
    than a warning. That is the intent, but it will surface mid-merge rather
    than at review time, and the honest response then is to port the code rather
    than to downgrade the lint.

### F315 (Active) — scan coverage is disclosed but not gated

The HEAD complexity scan now tallies eligible-but-skipped files and warns below a
90% floor. The `degraded` verdict still does not consume it. `eval_code_health_gate`
computes `degraded = code_health.is_empty() && head_has_scorable_source(repo, opts)`,
and that witness ends in `paths.iter().any(...)` — a boolean. A scan covering a
minority of the repository is not empty, so the gate still reports `passed`.

*   **Why this was not fixed in the same change**: the honest denominator is not
    obvious, and getting it wrong produces a sentinel nobody can trust. A HEAD-tree
    denominator is wrong: `query_live_paths` derives from `changes ⋈ commits`, so
    under `--after`/`--before` the scan legitimately attempts fewer files than the
    tree holds and a tree-based ratio fires spuriously. The correct denominator is
    the scan's own eligible count — which is exactly what the new `ScanCoverage`
    carries, and why threading it is the right shape rather than recomputing a
    ratio at gate time.
*   **Fix**: persist `ScanCoverage` where it survives a cache hit — `provenance`,
    not `IngestStats`, since a cached run never re-executes ingest — and have
    `eval_code_health_gate` record `degraded` when the stored ratio is below the
    floor. `fail_on_degraded` already defaults true, so wiring it is what converts
    disclosure into enforcement.
*   **Scope note, since F316 landed**: `ScanCoverage` is no longer complexity-only.
    All three HEAD passes now produce one, so the question this finding asks —
    what does the gate do with a thin scan — has three answers to thread, not
    one. `clones` is the sharpest of them: `disallow_clone_type_1` passes on
    zero, so a thin clones scan currently reads as an *improvement* rather than
    a degradation. Whatever shape the persistence takes should carry all three
    rather than special-casing complexity.

*   **Anti-vacuity requirement**: the self-test must reject a *partial* scan, not
    just an empty one. A test that only pins the empty case would pass against the
    current code and prove nothing about the change.

### F316 (Fixed — Unreleased) — the clones and imports HEAD passes share the silent-skip shape

`ingest_complexity_at_head` now classifies its skips; the sibling passes do not.
`clones_head.rs` and `imports_head.rs` both `warn!` per file and return `None`,
keeping no tally. `clones` feeds `disallow_clone_type_1`, which is literally
`COUNT(DISTINCT clone_group_id)` — a thin scan lowers the count and the gate reads
it as improvement. `imports` feeds `max_dependency_cycles` and the
architecture-violation gates.

*   **Fix**: lift `ScanOutcome`/`ScanCoverage` out of `complexity_head.rs` into the
    shared ingest module and apply to all three. Deliberately *not* done pre-emptively
    — one consumer is not yet a shared abstraction, and this repo's convention is that
    three similar lines beat a premature one. The second consumer is the point at
    which lifting it is justified.

**Fixed as described.** `ScanOutcome` and `ScanCoverage` now live in
`facts/ingest/coverage.rs`, generic over the payload so the three passes can
carry three different result types through the same accounting, and
`warn_if_degraded` takes the pass name and its fact table so the message says
both what went thin and what reads it. The classification is a faithful move —
outcomes still split on the per-file log level each pass already used, so the
`debug!` cases stay out of the denominator.

**One case needed care in the opposite direction, and the finding does not
mention it.** A file read and parsed successfully that declares no imports —
most files in most repositories — previously returned the same `None` as a
failed blob read. Routing that to `NotCounted` would have been the obvious
reading of "produced no row", and it would have been wrong: the file *was*
covered. It would also have shrunk the denominator, making coverage read
**better** the more import-free files a repository holds — reproducing one
level up the exact blindness this accounting exists to remove. It is now
`Scored` with an empty payload, and the drain filters empties one stage later
than the classifier, which is what lets the same code answer "what did we
write" and "what did we cover" honestly at once. Two tests pin it: one for the
empty-payload semantic, one asserting the tally stays payload-agnostic.

No ingested fact changes — the rows written to all three tables are identical
before and after, confirmed by the cache suite's whole-fact-store digest.

### F317 (Refuted) — `codelore gate` can pass a newly-introduced dependency cycle

`change_set.rs` builds the projected import graph from the working tree. Three
adjacent lines treat failure three different ways: the file read propagates with
`map_err(...)?`, the size cap `continue`s silently, and an `extract_imports`
failure `warn!`s and `continue`s. A file whose imports fail to extract contributes
no edges, so `cyclic_paths` sees no cycle and the gate passes.

*   **Impact**: this is the agent-loop gate — the surface whose whole purpose is to
    catch a regression before it lands.
*   **Fix**: the asymmetry is the bug. If an unreadable file is fatal, an
    unparseable one that silently removes edges from a cycle check should be at
    least disclosed. The `ParseOutcome::Skipped(REASON_*)` vocabulary already in
    this file is the mechanism; the import path just does not use it.

**Refuted on validation. No code change made.** Each of the three failure paths
was traced, and two of them cannot fire:

*   **`extract_imports` returns `Err`** — this is the mechanism the finding
    names, and it is not reachable per-file. The function's own `# Errors`
    section says so: it errors *only* if tree-sitter rejects the language
    assignment, "a static-config bug that would fail every file under that
    language". A malformed file never takes this path.
*   **`parser.parse()` returns `None`** (the `Ok(Vec::new())` guard) — proven
    unreachable by experiment. tree-sitter returns `Some` even for a byte string
    of binary junk; `None` requires an unset language or a timeout/cancellation,
    and `set_language` succeeded on the line above while no timeout or
    cancellation flag is configured anywhere in the crate.
*   **tree-sitter error recovery silently dropping imports** — a fourth
    mechanism the finding does not name, hypothesised during validation and
    then disproved. For `use a::b;` / a syntax error / `use c::d;`, extraction
    returns **both** targets, identical to the same file without the error.
    Error recovery does not cost import edges.

That leaves the AST size cap as the one reachable path — and it is **already
disclosed to the user**, which is precisely what the finding asks for. The chain
is complete and was traced end to end: a changed file over the cap is skipped by
`parse_worktree_file` as `ParseOutcome::Skipped(REASON_SIZE_LIMIT)`, lands in
`skip_reasons`, becomes a `FileDelta` carrying that reason, and `assemble_findings`
turns it into a user-visible `unparseable` finding reading "could not be
re-parsed for the projection: file exceeds the AST size limit." The gate does not
pass quietly; it names the exact file whose import edges were dropped.

The disclosure covers every reachable case because the two gates agree:
`ImportLanguage::from_path` and `Tier1Language::from_path` map the identical
extension set (`rs`, `py|pyi`, `java`, `js|jsx|mjs|cjs`, `ts`, `tsx`) against the
same `DEFAULT_MAX_AST_FILE_BYTES`, so any file the cycle projection caps is also
one the health projection caps and reports. (One asymmetry between them is real
but points the other way — see F321.)

Two further points the finding's framing gets wrong. The behaviour is **not
gate-specific**: `populate_imports_at_head` applies the same cap, so
`--analysis dependency-cycles` cannot see those cycles either — it is a uniform
property of the import graph, not a defect in the projection. And the cap is not
an oversight: its constant documents the measurements behind it (Linux's
`block.c` ~195 KB and V8's largest hand-written `.cc` ~1.5 MB, against minified
bundles at 5–50 MB and `sqlite3.c` at ~9 MB), and without it tree-sitter hits
stack-overflow or OOM on deeply-nested generated code. Excluding generated and
vendored files is the cap's purpose, and the project already classified a
size-cap skip as routine rather than lost coverage when it split `ScanOutcome`.

The comment above the loop was also checked against its own claim and is
accurate: it says the size cap "gates extraction" and that a per-file
*extraction error* is logged — both true. Adding a `debug!` for the size-cap
skip was considered and rejected as improving adjacent code to no end.

### F318 (Fixed — Unreleased) — a schema-wrong SARIF document silently deletes an engine's findings

`sarif_parse.rs` errors correctly on a missing `version`/`runs` and on a non-array
`runs`. But a run whose `results` key is missing or not an array hits a bare
`continue` with no warning — and the engine was already registered by then, so it
flows into the seed-empty-batches path where `replace_engine(engine, &[])` deletes
that engine's stored rows. That deletion is intentional for a clean re-scan, which
is precisely what makes a malformed document indistinguishable from a legitimate
one that found nothing.

*   **Impact**: silent data loss, then a green gate — `max_findings_in_hot_files`
    reports `skipped` on zero findings and `fail_on_skipped` defaults false.
*   **Fix**: treat a missing/invalid `results` as an error rather than a skip, or at
    minimum refuse to seed an empty batch for a run that did not parse. Untested
    today; the test should plant a valid-JSON/wrong-schema document and assert the
    prior rows survive.

**Correction applied when fixing this.** The finding groups "missing or not an
array" together and the proposed fix errors on both. Erroring on a *missing*
`results` would be wrong: SARIF 2.1.0 marks the property optional, so a run that
found nothing may legitimately omit it, and rejecting that would break the very
clean-re-scan path whose delete semantics make this finding dangerous. The three
cases are now distinct — absent and `null` are a clean scan, `[]` is the same
scan spelled explicitly, and present-but-non-array is a hard error naming the
type it found. Two of the three tests exist as anti-vacuity controls, because a
parser that simply rejected every zero-finding run would satisfy the
malformed-input test while silently breaking clean re-scans.

### F319 (Fixed — Unreleased) — 21 analysis-only options key the ingest cache, forcing a full re-ingest to change a threshold

`cache.rs` folds `Options::canonical_json()` into the cache key, and
`canonical_json` deliberately admits every field not on its drop-list. Most of
those fields never reach the ingest. Changing `--min-revs 5` to `--min-revs 10`
re-walks the entire history and burns one of the five per-repo cache slots —
and threshold sweeping is the natural way to use this tool.

**The classification, verified from the direction that matters.** A negative
grep ("none of these appear in the ingest path") is weak evidence and was in
fact unreliable here — a shell-variable expansion made it return clean for
everything. The positive enumeration is the one to trust: every `opts.<field>`
token appearing anywhere under `repo/`, `facts/`, `kamei/`, `identity/`,
`imports/`, `clones/` and `paths_filter.rs`.

That yields exactly **13** tokens, of which **12 are code**:

    after, before, exclude_patterns, group_file, head_only_ingest,
    include_ignored, include_merges, min_clone_node_count, repo_path,
    strict_grouping, track_rewrites, use_canonical_lineage

The thirteenth, `time_bucket`, appears only inside a doc comment at
`facts/ingest/grouping.rs:15`; the table it names, `changes_bucketed`, is a
session TEMP table built at analysis time. It is analysis-only. Note that an
`-o`-style token extraction cannot distinguish code from comments, so this one
has to be read rather than counted — it is the single discrepancy between the
two methods and the reason the enumeration must be inspected, not just sized.

Everything else — `min_revs`, `min_shared_revs`, `min_coupling_pct`,
`max_coupling_pct`, `max_changeset_size`, `fisher_significance`,
`message_regex`, `age_time_now`, `min_soc`, `code_maat_compat`,
`fdr_correction`, `window_days`, `knowledge_model`, `rework_window_days`,
`release_tag_glob`, `departed_threshold_days`, `min_clone_shared_revs`,
`clone_similarity_floor`, `clone_skip_same_dir`, `complexity_sample`, and
`time_bucket` — resolves to `analyses/` or `quality_gates/`.

*   **Resolution**: `Options::ingest_cache_json` now keys the cache, deriving
    itself by *subtracting* the analysis-only list from `canonical_json` so the
    two cannot drift in shape. `CACHE_EPOCH` moved to `schema_v19` to discard
    entries written under the old classification — correct facts, unreachable
    key, and they would otherwise hold slots forever.
*   **The trap this hit, worth recording**: the obvious implementation is to
    narrow `canonical_json` in place. That would have been a correctness
    regression. `change_set::report_key` keys the agent-loop report cache on it
    and its docstring explicitly relies on it covering `min_revs` and the clone
    thresholds — narrowing it would have let two gate runs at different
    thresholds share a report entry and serve a wrong verdict. The split had to
    *add* a function, not edit one. Checking a function's callers before
    narrowing it is the general form.
*   **The proof, and its own near-miss**: an equivalence test ingests the same
    repository twice — defaults, then all 21 knobs moved simultaneously — and
    asserts a per-table digest of every row of every table is identical, with
    dynamic table discovery so a new table joins without anyone remembering.
    Its anti-vacuity control had to be corrected during development: the first
    control flipped `include_merges`, which changes nothing on a linear fixture
    (`tiny_repo`'s merge lives in `differential_repo`), so it proved nothing
    about the digest's sensitivity. A control that cannot fail is decoration.
*   **Adjacent, lower value**: `calibration_digest` / `defect_calibration_digest`
    also key the cache, while `options.rs` asserts "the corpus lens is additive
    output only — it must never split the ingest cache." Path-independence is
    tested; content-independence is not.

### F320 (Fixed — Unreleased) — `code-health` walks the working tree on every run, and on a dirty tree that contradicts what `check` documents

`run_code_health` — the entry point behind `codelore check`, the SPA,
`factors`, `delta-health` and `refactoring-targets` — builds its context from
`HealthScanCtx::head()`, which sets `clone_source: CloneSource::WorkingTree`.
That triggers a full `WalkDir` + tree-sitter parse of every Tier-1 file at
*analysis* time, on every invocation, including a cache hit.

The cheap path already exists and is already wired: `CloneSource::Head` reads
`SELECT path, COUNT(*) FROM clones GROUP BY path` from the table the ingest
populates from HEAD blobs. Both key on path — `clones.rs` sets
`entity: member.path.clone()` — so the two are structurally comparable.

**Two problems, one cause.**

*   **Cost.** On a clean tree the walk recomputes what the cached `clones`
    table already holds. `run_clones_memoised` bounds it to once per process,
    not once per analysis, but it is still a full parse per invocation of the
    flagship analysis and everything routing through it.
*   **Semantics.** `CloneSource::WorkingTree`'s own doc warrants itself with
    "On a clean tree this equals HEAD" — a justification that holds only in the
    case where the walk is *also* redundant. `check` does not require a clean
    tree, and `advanced-usage` §"quality gates" states the three surfaces are
    non-overlapping: **`check` gates the committed tree at HEAD**, `gate` gates
    the uncommitted working tree, `diff` gates a rev range. On a dirty tree
    `check`'s DRY biomarker reads the working tree while the command claims to
    describe HEAD.

**Why this is not implemented here.** The one-line fix — `run_code_health`
uses `CloneSource::Head` — is byte-identical on a clean tree and buys the perf
win outright. But it *changes* dirty-tree behaviour: `check` would stop
counting duplication you had written and not yet committed. That is arguably
correct (it is what the documented split says, and `gate` is the surface for
uncommitted work) and arguably a regression (someone using `check` as a
pre-commit check loses a signal). That is a product decision, not a
refactor, and it should be made deliberately rather than arrive inside a
performance change.

*   **Option A — align with the documented contract.** `run_code_health` uses
    `CloneSource::Head`. One line. Byte-identical on clean trees; on dirty
    trees `check` becomes HEAD-faithful as documented. Requires a CHANGELOG
    note that `check` no longer sees uncommitted duplication, and a pointer to
    `codelore gate` for that use.
*   **Option B — keep the semantics, take only the perf.** Select `Head`
    when the worktree is clean and `WorkingTree` when dirty. Zero behaviour
    change. Costs more: `run_code_health(db, opts)` has no `Repo` and cannot
    ask, so the cleanliness signal must be threaded through the signature or
    `Options` across every call site.
*   **Not affected either way**: the gate baseline (`change_set.rs`, already
    `Head`) and the gate projection (already `WorkingTree` and deliberately so).
    `health_trend.rs` and `defect_calibration/validate.rs` also pin
    `WorkingTree`; whether a *historical* rev should be scored against today's
    working tree is a separate question this finding does not address.

**Resolved as Option A, after the blast radius was measured rather than
assumed.** The finding calls this a one-line fix and it is, but the line is not
where it says: rather than overriding `clone_source` inside `run_code_health`,
the default in `HealthScanCtx::head()` moves to `CloneSource::Head`. That is the
actual defect — a constructor whose name and doc comment both say HEAD was
setting the one field that read the working tree, while every other field
resolved to a fact-store table.

Changing a default that feeds eleven production call sites needed checking, and
it held: **every caller that deliberately wants working-tree clones already
states it explicitly**, and all three build the context as a full struct literal
with no `..head()` spread, so none of them observe the default at all. Those are
the gate projection in `change_set.rs` (which must see uncommitted duplication,
and whose baseline already pinned `Head` so the two do not cancel), plus
`health_trend.rs` and `defect_calibration/validate.rs` — the latter two with
`include_clones: false`, making their clone source moot.

Two call sites were contradicting their own labels and are fixed by the same
line. `check`, per the documented split. And `calibrate-defects`, whose call
reads `.context("HEAD code-health scan")` under an `eprintln!` announcing
"scanning HEAD code-health" while `repo_path` points at the user's real repo —
its output feeds a committed calibration artifact, so a dirty tree could bake
uncommitted edits into shipped quantiles. That instance is not in the finding
and is the more consequential of the two.

`codelore diff` was investigated as a suspected third instance and cleared:
`analyze_at_rev` sets `repo_path` to a throwaway worktree checked out at the
rev, so the working tree *is* the rev and both clone sources describe the same
content. The "code health at rev" call was never reading the user's tree.

The equivalence ships as a test rather than a claim — one scan per clone source
over a fixture whose worktree matches HEAD, comparing path, score, band and
cognitive row by row, with an anti-vacuity guard on the row count. A second test
pins the default, because the first compares two explicit contexts and would
keep passing if the default silently reverted.


### F321 (Fixed — Unreleased) — an uppercase source extension gets complexity but no imports

`Tier1Language::from_path` lowercases before matching
(`ext.to_ascii_lowercase()`); `ImportLanguage::from_path` matches the raw
extension. The two arms are otherwise identical (`rs`, `py|pyi`, `java`,
`js|jsx|mjs|cjs`, `ts`, `tsx`), so the sets agree for every lowercase path and
diverge for an uppercase or mixed-case one: `Foo.RS` is Tier-1 — it is scanned
for complexity, scored for code health, and fingerprinted for clones — while the
import pass returns `None` and skips it silently.

Found while validating F317, which needed the two gates to cover the same file
set. They do for the case that mattered there, and this is the direction they
do not.

*   **Impact**: such a file is a node with no outgoing edges in the import
    graph, so it is invisible to `dependency-cycles`, `architecture-violations`,
    `instability` and the propagation-cost family — which read as *this file
    imports nothing*, not *this file was not examined*. Unlike the size cap
    (F317), nothing discloses it. Case-sensitivity is the trigger, so the
    exposure is filesystem- and project-dependent: a case-insensitive
    filesystem (macOS default, Windows) makes an uppercase-extension file
    ordinary, and it survives into a case-sensitive CI checkout unchanged.
    Rare in practice for the Tier-1 languages, none of which conventionally
    uppercase their extensions.
*   **Fix**: lowercase in `ImportLanguage::from_path` to match its twin. Note
    the two also differ in how they take the extension — `Path::extension()`
    versus `rsplit('.')` — which disagree on a dotfile like `.rs` (the former
    yields `None`, the latter `"rs"`); align deliberately rather than by
    accident, and cover both with a test.
*   **Not urgent**: no evidence of a real repository hitting it, and the
    corrective is small. Recorded rather than fixed inline, per the standing
    rule on latent bugs found during unrelated work.
*   **Resolution (with F311)**: all three dispatchers now fold case over
    `Path::extension` semantics — the dotfile divergence resolved
    deliberately toward std (`.rs` has no extension, so no language) — and
    clones accepts `.pyi`, whose stub fingerprints fall below the node-count
    floor and so cost no clone noise. A parity test in `imports::language`
    probes all three over mixed-case, dotfile, and non-Tier-1 names, with
    positive pins so it cannot rot into all-`None` agreement. `CACHE_EPOCH`
    moved to `schema_v20`: entries ingested under case-sensitive dispatch
    hold correct-but-incomplete `clones`/`imports` tables for repositories
    carrying such files.

### F322 (Fixed — Unreleased) — three MCP tools scored with default smell weights regardless of the server's `--defect-calibration`

`codelore mcp` resolves the defect-calibration artifact once at startup and
stores it on `CodeLoreServer`, and the tools whose contracts promise
calibration awareness thread it: `check_gates`, `explain_file`,
`change_context`, `gate_changes`. The other seven handlers built their
`Options` with `..Options::default()` — harmless for the four whose analyses
never read the artifact (`repo_overview`, `hotspots`, `delta_health`,
`function_xray`: only the code-health pass consumes the weights), and a live
divergence for the three that embed code-health scores: `code_health`,
`refactoring_targets`, `finding_hotspot_overlap`.

*   **Impact**: within one server session, `code_health` and `check_gates`
    answered with two different weight regimes; the MCP `code_health` result
    silently diverged from `codelore analyze --analysis code-health
    --defect-calibration` on identical repository state. An agent using the
    triage tool and the verdict tool together compared incomparable numbers.
*   **Fix (shipped)**: the three handlers thread `defect_calibration` and
    `allow_foreign_calibration` exactly as `check_gates` does; the two
    memoized ones fold the artifact's content identity into their memo keys
    the way `explain_file` already did, so a regenerated artifact cannot
    serve a stale score without moving HEAD. The four artifact-blind tools
    are deliberately unchanged — threading the field there would only
    invalidate their memos on artifact regeneration for no observable
    difference.
*   **Proof**: the regression test runs two servers over one worsened
    fixture (default weights against a value-permuted artifact), asserting
    the file set agrees and at least one structural risk moves; probed
    against the unfixed handlers it fails with the intended message.

Found by an MCP options audit, which also surfaced the sibling gap recorded
as F323.


### F323 (Fixed — Unreleased) — MCP has no corpus-lens calibration surface at all

`CodeLoreServer` carries `defect_calibration` but no `calibration` field, and
no MCP flag exists to supply one: every MCP tool result that consults corpus
percentiles reads the embedded world artifact only, while the CLI accepts
`--calibration` everywhere. A team with a custom corpus artifact gets CLI/MCP
divergence on every percentile-annotated surface.

*   **Fix**: a `--calibration` startup flag on `codelore mcp`, threaded like
    `--defect-calibration` now is (F322), including the memo-key fragment.
*   **Small and mechanical**, but it widens the MCP flag surface — recorded
    for a deliberate pass rather than fixed alongside F322.
*   **Resolution**: `codelore mcp --calibration <path>`, mirroring
    `--defect-calibration` end to end — startup fail-fast load, threading
    into every lens-consuming tool, memo-key content fragments for the
    memoized ones (`gate_changes` needs none: its report cache keys through
    `Options::canonical_json`, which already folds the digest). Pinned by a
    two-server lens-divergence test and a malformed-artifact startup
    rejection test.

### F324 (Fixed — Unreleased) — two at-rev scans still paid the cold per-blob path

`ingest_complexity_at_rev` kept `map_init(|| ())` + per-file `read_blob_at`
(per-call rev resolution + root-tree decode, cold cache) while its sibling
import scan in `architecture_trend` had already adopted the warm per-worker
reader and named the cold path "the worse offender". Cost: once per file per
sampled revision on every `health-trend`/`architecture-trend` timeline and
every defect-calibration validation pass. `effort_exposure`'s window-start
baseline had the serial version: one cold read per red file, all at one rev,
in two loops. All three sites now hoist a warm reader; byte-identical by the
reader's own equivalence test.


### F325 (Fixed — Unreleased) — delivery-friction aggregated raw paths

The one path-aggregating analysis in its cohort with no lineage opt-in: its
complexity axis routed through `grouped_complexity::source_table`, its churn
axis read `FROM changes` by raw path, so a rename split revisions/lead-time/
WIP history at the rename point. Now `materialize_if_needed` +
`lineage::rewrite`, the `stale-code` shape. Output moves only where flagged
files carry renames.


### F326 (Fixed — Unreleased) — the two backends disagreed on `is_shallow`

`GixRepo` reads the grafts file; `GitCliRepo` silently inherited the trait's
`false` default despite having a cheap check available
(`git rev-parse --is-shallow-repository`, correct for linked worktrees). It
was also the one hint method with zero differential coverage. Both fixed: the
override mirrors gix, and the differential suite gains a `--depth=1`-clone
probe (via `file://`, since git ignores `--depth` on plain local paths) that
is self-proving — the old default answers `false` and fails the equality.


### F327 (Fixed — Unreleased) — twelve `tempfile` test modules broke the bare test invocation

`tempfile` is optional behind `test-support`, yet twelve unit-test modules
used it under bare `#[cfg(test)]` — so `cargo test -p codelore-lib` failed to
compile before running anything (the documented symptom blamed only
`options::tests`; the census found twelve). All now carry
`#[cfg(all(test, feature = "test-support"))]` like the two modules that had
it right. In the same change `paths_filter` — load-bearing for the cache key
— gained its first direct tests: ignore-source precedence, negation,
ancestor rules, the `--include-ignored`-must-not-neuter-`--exclude` boundary,
and `is_git_metadata` first-component semantics.


### F328 (Refuted) — "churn analyses silently ignore `--time-bucket`"

The lead: `churn.rs` carries a private copy of the lineage dispatcher that
lacks the `time_bucket` branch the shared `lineage::source_table` has.
Refuted as a correctness bug: the CLI hard-rejects `--time-bucket` for any
analysis outside coupling/soc/hotspots/code-health (`analyze.rs`), so the
missing branch is unreachable. What survives is the DRY hazard — a private
duplicate of a shared dispatcher is exactly how the next such flag diverges
silently. Unifying churn onto `crate::analyses::lineage` remains a valid
non-correctness cleanup.


### F329 (Refuted) — "stale-code silently ignores `--min-revs`"

The lead: `run_stale_code` records `min_revs` in its tracing span but never
binds it, while sibling `code-age` binds it. Refuted: the documented contract
(alive at HEAD ∧ untouched ≥ 12 months ∧ `max(cognitive) ≤ 5`) never
promises a revision floor — and semantically must not have one, since rarely
touched files are exactly the analysis's subject. Three other analyses carry
`min_revs` in their spans unused as blanket telemetry convention; not a lie,
a uniform field.


### F330 (Active) — the knowledge-shares guard ignores the options it was built under

`is_knowledge_shares_built` is a bare bool: the first caller's `opts`
(window, lineage source) bake the temp tables, and later callers with
different opts silently reuse them. Divergent per-analysis opts inside one
dashboard build is a live pattern (`delivery_metrics` clones opts with
`include_merges = true`). Needs validation of whether any current caller
pair actually diverges on shares-affecting fields; the fix shape is the
same as the lineage guard (key the guard on the inputs).


### F331 (Active) — refactoring-targets leans on a biomarker-table side effect that call order barely protects

`run_refactoring_targets` reads `code_health_biomarkers_v1` expecting the
HEAD pass that its own `run_code_health` call materialised — but
`health_trend`'s sampled passes `CREATE OR REPLACE` the same table with
historical data, and in the SPA build the trend loop runs first. Correct
today only because refactoring-targets re-materialises. Any future
code-health memo that early-returns on a hit hands it the last trend
sample's biomarkers. The memo (a real SPA-render win: five full passes per
render today) must re-assert the temp table, or refactoring-targets must
materialise explicitly.


### F332 (Active) — hotspots scores unsupported-language files as perfectly healthy

`joined` LEFT JOINs complexity and `COALESCE(fc.cognitive, 0)`, so a
non-Tier-1 file enters the ranking with cognitive 0 → `pr_cx = 0`, score 0,
`cognitive_health = 100`. The most-churned Go file in a polyglot repo ranks
below every trivial Rust file and reads as a verdict, not a coverage gap —
while `code-health`'s INNER JOIN silently *drops* the same file: two
opposite silent semantics side by side. The `mi_rank` CTE already models the
care needed ("files without an `mi` MUST NOT skew the distribution"); the
cognitive rank never got it. Needs a semantics decision
(exclude-with-disclosure recommended) before code.


### F333 (Fixed — Unreleased) — files past the AST byte cap are invisible everywhere

Over-cap skips return `ScanOutcome::NotCounted` (excluded from the coverage
denominator by design), log at `debug!` under a default `warn` filter, and
the cap appears nowhere in the docs. A bundle-heavy repository therefore
reports 100% clone coverage over an empty `clones` table — the
reads-as-improvement failure mode the coverage sentinel exists to prevent,
reintroduced one classification to the left. Cheap fixes: a disclosed
skipped-oversize count in the aggregate line, and a documentation paragraph;
a bytes-per-line minified-file heuristic would close the sub-cap hole
without touching the vendored parser.

**Resolution (disclosure half)**: oversize skips are now their own tallied
outcome — still outside the loss ratio — and each pass warns once when they
outnumber the scanned files, with the predicate tested directly. The cap and
both aggregate warnings are documented in the user guide. The bytes-per-line
sub-cap heuristic remains open as the (b) half; it changes which files are
scanned and deserves its own pass.


### F334 (Active) — `diff_hunks` is a required trait method with zero production callers

Hunks attach inline during the walk (both backends), so `Repo::diff_hunks`
is dead surface that every future backend must still implement and the
differential suite still pays to cross-check. Adjacent divergence:
`GitCliRepo::walk_commits` always emits `hunks: []` while `GixRepo`
populates them, and no differential assertion compares the field — an
unenforced corner of the parity guarantee. Needs a remove-or-repurpose
decision; if removed, the differential hunk test retires with it, and if
kept, the field comparison joins the gate.


### F335 (Active) — the ignored-flag warning table trails the flag surface

`ignored_flag_warnings` hand-lists 7 analysis-scoped flags; 9 more
(the coupling and clone families) warn nothing when set on an unrelated
analysis. Sibling paper cut: `CalibrateDefectsArgs::window_days` shares a
name with `Options::window_days` under different ranges and semantics.
Sibling structural gap: nothing ties `Gates`/`DiffGates` fields to
evaluator branches or `RatchetMetrics` — a gate added to config but not the
evaluator (or ratchet) fails nothing. One enforcement test per pair is the
cheap form.

### F336 (Active) — `dependabot-auto-merge` grants both jobs the union of their scopes, and its comment claims the union is irreducible

The workflow-level block grants `contents: write`, `pull-requests: write`
and `actions: write` to both jobs, with a comment stating both jobs need
them and the grants "cannot be narrowed". Source says otherwise:
`mark-eligible` only creates a label and labels the PR — it never merges,
pushes, or dispatches, so it uses neither `contents: write` nor
`actions: write`; only `merge-on-green` exercises all three. The over-grant
sits on the `pull_request_target` job — exactly the trigger whose token
privilege the header mitigates by never checking out PR code; per-job
blocks would turn that mitigation from documented discipline into a token
that cannot do the dangerous thing at all. Narrowing is untestable until
the next Dependabot PR fires (labeling runs only under the live token),
which is why this is recorded rather than fixed inline.

### F337 (Fixed — Unreleased) — the lineage rename map was applied by name, with no time bound

`materialize_path_lineage` date-guards chain construction against recycled
filenames, but `materialize_changes_lineage` applied the finished map with
`LEFT JOIN path_lineage ON old_path = c.path` — string key only. A name
retired by a rename and later reused by an unrelated file had the new
file's rows rewritten onto the old file's canonical target: the new file
vanished from every path-aggregating analysis and the target inflated with
a stranger's history. Default-on (`use_canonical_lineage`), and persisted:
Kamei enrichment runs at ingest through this view, so cached fact stores
carried the conflated attribution. Found independently by two second-wave
auditors; the fix models retirement *epochs* (a name can be retired more
than once), bounds the join to the half-open window between consecutive
retirements, and excludes `copied` rows from seeding or extending chains.
`CACHE_EPOCH` → `schema_v21`. The recycled-name regression test fails on
the unfixed join; see `docs/reports/2026-09-02-deep-analysis-second-wave.md`.

### F338 (Fixed — Unreleased) — the `--time-bucket` gate covered the named analysis but not the composite fan-out

`supports_time_bucket()` is enforced against `--analysis` only; `--format
spa` / `step-summary` then run ~30 further analyses with the same
`Options`. Their `lineage::source_table` resolves to `changes_bucketed`,
whose `rev` is a date-truncated string, so commit-keyed joins return zero
rows — and the composite's degradation wrappers catch `Err`, not empty, so
the widgets silently blank (with the bucketed `knowledge_shares` build
poisoning its once-guard for the rest of the run). The combination is now
rejected at the CLI boundary (exit 2); the regression test names a
bucket-aware analysis so the pre-existing per-analysis gate cannot mask a
regression. Remaining bucketed-table hygiene (no build-once guard;
lexicographic `MAX(change_type)` beside chronological `arg_max`) is
recorded in the second-wave report and lands separately.

### F339 (Fixed — Unreleased) — three analyses summed overlapping complexity rows into per-file SLOC

`complexity_metrics` rows overlap by construction (`collect_entities`
pushes the root unit space, then recurses into every child).
`effort_exposure::fetch_sloc_map`, `code_familiarity`'s `sloc_per_path`,
and the knowledge DOE `head_sloc` all took `SUM(sloc)` per path — a ~1-3x
inflation that does not cancel out of shares, tilting every ratio toward
function-dense files; `effort_exposure`'s comment asserted the opposite
rationale, while `code_health`'s file aggregation one module away already
used the correct `MAX`. All three now take the unit row via `MAX`. The
seeded regression test fails with the exact doubled value on the summing
form, and the intended output shift was measured before/after on this
repository (delta recorded in the fix PR).

### F340 (Fixed — Unreleased) — the gix diff cap sat ~512x below the git default it claimed to match

`MAX_DIFF_BLOB_BYTES` was 1 MiB under a comment claiming parity with
git's `core.bigFileThreshold` default (512 MiB), so every text file past
1 MiB entered `changes` with zero `loc_added`/`loc_deleted` on the
production backend while `git log --numstat` counted its lines — churn,
hotspot-velocity, code-health's churn term, and the Kamei size features
silently drained on exactly the files most likely to be large. The
divergence was reproduced against real git during the second-wave audit
and is invisible to the differential suite's aggregate drift band. The
cap now matches git's actual threshold, oversized blobs are rejected via
an object-header size probe before their bytes load, and a differential
test generates a multi-megabyte text file at test time pinning both
backends to identical, nonzero per-file counts.

### F341 (Fixed — Unreleased) — six chronology tiebreaks violated or ignored the rowid convention

The ingest documents that gix walks reverse-chronologically (smaller
`rowid` = newer commit) and defines the tiebreak idiom; six query sites
diverged: coordination-needs' author-interleave LAG window visited
same-second commits newest-first inside an ascending scan (flipping
`prev` and corrupting the interleave count at ties), cycle-origins and
architecture-trend inverted their historical walks the same way, the
Kamei sparkline's last-N picked the OLDER of a same-second pair, and two
newest-first lookups — `window_start_rev` (shared with the `[new_code]`
gate) and the SARIF evidence chain — tiebroke on SHA lex order, which the
ingest comments call topologically meaningless. All six now follow the
documented rowid direction; deterministic before and after, just no
longer chronologically wrong at ties. The seeded regression uses two
same-second commits whose SHAs sort AGAINST chronology, so the old
tiebreaks fail it by construction.

### F342 (Fixed — Unreleased) — knowledge signals counted dead and renamed-away paths as live

Two liveness holes in the knowledge family. `knowledge_shares` excluded
deletion *events* (`change_type != 'deleted'`) but never deleted *paths*,
so a long-deleted file kept its pre-deletion contributions and flowed
into every consumer — coordination-needs emitted an output row per dead
file and code-familiarity counted authors who only ever touched deleted
files; the reviewer-credit query could re-introduce a dead path even
after the authored rows were filtered. Separately, the knowledge
prevalence tile's denominator (`count_live_files`) ran over raw `changes`,
where a renamed-away source path's most recent own event is its
pre-rename row — live forever — while the numerator was lineage-aware:
unlike populations in one ratio. The materializer now drops dead paths at
both stages, and the denominator reads the same lineage-aware source as
its numerator (raw paths on both sides when lineage is off). Seeded
regressions pin a dead path staying out of shares beside a live control
and the renamed-away fold under lineage against the lineage-off
population. The ingest-side `query_live_paths` shares the renamed-away
shape but only wastes blob lookups (correctly bucketed `NotCounted`) and
is deliberately unchanged.

### F343 (Fixed — Unreleased) — `--group-file` silently zeroed the clone- and import-joining analyses

Grouping rewrites `changes.path` and builds a grouped complexity rollup
(whose own documentation explains why the rollup is necessary), but
`clones` and `imports` keep raw file paths. `clone-coupling` matched raw
clone-pair paths against a coupling map keyed on grouped paths and
`crossing` joined raw import edges against grouped coupling — zero keys
matched, so both returned an empty result that read as a clean bill. The
combination is now rejected at the CLI boundary (exit 2), the same gate
shape as the pre-existing `function-hotspots` rejection; one regression
test drives both analyses and fails with the gate stashed. A grouped
`imports` rollup (group→group edges) remains a defensible future
alternative; a grouped `clones` rollup is not meaningful.

### F344 (Fixed — Unreleased) — historical liveness carried renamed-away paths as phantom nodes

`live_paths_at`'s date-anchored rule treated a renamed-away source as
live forever (a rename writes no deletion row for its source), so every
post-rename sample point carried the same file under both names —
inflating node counts and diluting propagation cost across
`architecture-trend`, `health-trend`, and `cycle-origins`' bisection.
Fixed with an era-bounded rename exclusion, deliberately NOT the
lineage-fold the second-wave report suggested: the returned names feed
`Repo::blob_reader_at(rev)` and must be the names that exist in that
era's tree — folding to canonical names would make every pre-rename blob
read miss and be silently skipped. A recycled name returns to life on
its own newer rows; the unit test walks one name through all three
epochs.

### F345 (Active) — health-trend's per-file series breaks across renames, and the cheap fix is wrong

`health_trend::top_hotspot_paths` ranks over raw `changes` (revisions
split across a renamed file's two names, so a genuinely hot file can
fall below the top-N cap), and the per-sample series keys era paths
against that HEAD-name set — a file renamed mid-history appears as two
disjoint series or drops out entirely. The obvious fix (route the
ranking through `lineage::rewrite`) is wrong for the same reason F344's
was: pre-rename samples carry era names that would no longer match the
canonical-name set. A correct fix needs the ranking canonical AND an
era-to-canonical mapping applied at membership/series-key time (the
epoch-bounded `path_lineage` now carries enough information to build
one). Designed fix, not a two-line patch — recorded rather than
half-fixed.

### F346 (Fixed — Unreleased) — `changes_bucketed` rebuilt on every call and collapsed `change_type` lexicographically

The bucketed table had no build-once guard (its sibling `changes_lineage`
gained one under F184 for exactly this cost), so every bucket-aware
analysis in a run re-ran the full `changes ⋈ commits` scan; and its
`change_type` collapsed with lexicographic `MAX` one line away from the
chronological `arg_max` on `rename_from` — a file modified then deleted
inside one bucket read 'modified' and stayed live under every downstream
deletion rule. The new guard keys on `(bucket unit, lineage)` so a
different bucketing still rebuilds, and `apply_grouping`'s swap
invalidates it alongside the lineage guard; `change_type` now takes the
chronologically last event. `apply_grouping`'s own `MAX(change_type)` is
deliberately untouched — that collapse is within one commit, where no
chronological order exists. Seeded tests pin the modify-then-delete
collapse and the guard's same-key/different-key behavior via a sentinel
row; both fail on the unfixed shapes.

### F347 (Fixed — Unreleased) — the SARIF error band was unreachable, and the rule metadata undersold the findings

The hotspot `security-severity` proxy divided by 10 a health value the
hotspots analysis bounds to [60, 100], capping severity at 4.0: the
`error` level (≥ 7.0) was dead code, `warning` fired only at exactly
4.0, and a structurally healthy hotspot emitted 0.0 — which GitHub maps
to "no severity". The emitter's own band tests could only reach the
bands with out-of-range fixture health values, which is how the dead
branch hid. The divisor now spans the real range, floored at 0.1, with
in-range band tests and a floor test. All five rule surfaces also gained
the consumer-read metadata that was missing (`fullDescription`, `help`
with the markdown form GitHub prefers, `defaultConfiguration.level`,
hotspot `precision`, and `properties.problem.severity` — the correct
severity channel for non-security rules). Remaining from the same
research, deliberately not in this change: self-truncation above the
5,000-results display cap with disclosure.

### F348 (Fixed — Unreleased) — attestations never reached release assets, and the Action's checksum check failed open

The signing pipeline was complete and correct — matrix-attested archives,
enforced L3 permission split — but bundles went only to the GitHub
attestations API, while asset-based verifiers (OpenSSF Scorecard's
Signed-Releases check among them) inspect release assets for
`*.sigstore.json` and never consult the API: releases scored as unsigned
forever. Each bundle now publishes as an `<archive>.sigstore.json` asset.
The trusted signer stays first-party-actions-only — the signing-isolation
guard rejected this change's first draft for adding a rename shell step
beside the token, exactly its designed enforcement — so the per-archive
rename lives in the token-free release job, behind a zero-staged-bundles
cardinality floor. Separately, `action.yml`'s SHA256SUMS verification
skipped on ANY fetch failure (absence indistinguishable from a 5xx, a
proxy, or an adversary dropping the manifest); it now branches on HTTP
status — 404 (pre-manifest release) warns, everything else and a
manifest lacking the archive's entry refuses to run an unverified
binary. Runtime proof of the bundle wiring lands with the next `v*` tag.

### F349 (Fixed — Unreleased) — an ambient key silently redirected the advisory layer, and repository text reached the prompt unfenced

`resolve()` selected the hosted Anthropic dialect whenever
`ANTHROPIC_API_KEY` was present with no explicit provider — a credential
commonly exported for unrelated tooling silently redirected fact sheets
(paths, author identities, function names, scores) to a hosted endpoint
while the documentation promised local-first behavior; the same table
that promised it documented the inference four lines later. The hosted
dialect now requires an explicit `CODELORE_LLM_PROVIDER=anthropic`;
key-without-provider errors naming the fix. A bearer token over plain
http is loopback-only, parsed as real addresses. Separately, the prompt
embedded the fact sheet unfenced and unescaped — git permits newlines in
paths and near-arbitrary author names, so hostile repository content
could forge additional fact lines (the probe renders the exact shape: an
injected value becomes a second `score = 99` line) or spell
directive-looking text; the sheet is now fenced with a
data-not-instructions rule and control characters are escaped, with
`PROMPT_VERSION` bumped so cached narratives recompute. The grounded
stamp was never forgeable — numeric ground truth is collected from typed
values before rendering — and that property is now pinned by a contract
test rather than being an unstated implementation accident.

### F350 (Fixed — Unreleased) — a drifted direct arrow dependency made every provenance stamp name the wrong generation

A direct `arrow` dependency sat a major ahead of the one `duckdb` pins,
putting two arrow generations in the build graph (~330 lockfile lines of
duplicate family), while `ARROW_RUNTIME_VERSION` — stamped into every
provenance sidecar and the fact store's provenance table — described the
duckdb-pinned generation the facade's re-exports no longer resolved to.
The drift guard's first-match lockfile lookup landed on the pinned copy
and passed. The ledger records the original arrow bump PR being closed
precisely to avoid this desync; it landed later anyway. Nothing ever
used the direct dependency (zero `append_record_batch` call sites,
parquet rides DuckDB `COPY`, the facade's twenty type re-exports had no
consumers), so it is removed along with the arrow-typed appender
feature: one generation remains, the stamp is true, and the lockfile
diff is pure removals. `locked_version` now panics on duplicate entries
naming the versions, with a `should_panic` matcher self-test — probed on
a synthetic string, because cargo regenerates the real lockfile before
tests read it and silently scrubs an appended fake entry.

### F351 (Fixed — Unreleased) — `--output -` created a literal file named `-`, and diff's report write was not atomic

No `-`-as-stdout handling existed anywhere in the CLI, while the
README's flagship CI recipe piped `codelore diff … --output - >>
"$GITHUB_STEP_SUMMARY"` — the markdown went into a file named `-` in the
working tree, the step summary stayed empty, and the junk file surfaced
as an untracked change to the next gate run. `-` now streams to stdout
for every streaming format in both `analyze` and `diff` (the documented
recipes work as written — their pre-fix droppings, a literal `-` plus
`-.provenance.json`, were still sitting in this repository's own
working tree from test runs). The path-based formats reject `-` up
front. Routing `diff` through the shared output helper also replaced
its raw truncate-on-create with atomic publication, so a failing run no
longer destroys the previous good report.

### F352 (Fixed — Unreleased) — exit codes drifted from the documented contract on three surfaces

Every `codelore diff` failure exited 1 — a typo'd rev range, a missing
git binary, and a real gate violation were indistinguishable to CI,
against the design intent the user guide states. Unsupported
format×analysis combinations returned four different codes depending on
which path caught them (sarif 4 before ingest; ndjson/html 2 after it;
parquet 1 after it — the gate verdict's code for a flag typo), and
`schema`/`explain` reported unknown names with the analysis-crash code
while the parser used 2 one flag over. Diff's six untyped failures are
now typed (malformed range and identical base/head exit 2; unresolvable
revs and git failures exit 3); streaming format×analysis combinations
validate before the pre-flight from `supported_formats` — the same
derived table the dispatch reads, so gate and arms cannot drift — all at
exit 2 including parquet's subset; unknown names exit 2 everywhere.
Three contract tests pin the table, probed one filter at a time —
`cargo test` silently errors on multiple positional filters, which made
an earlier probe read as vacuously green.

### F353 (Fixed — Unreleased) — text-mode verdicts split across stdout and stderr, against the documented contract

`check`/`gate` printed PASS and WARNING lines (and the shallow-checkout
notice) to stdout in text mode while FAIL went to stderr — `codelore
check > log` captured pass but lost fail, `2>/dev/null` the reverse —
and SARIF mode printed no PASS line at all despite the documented
promise of a verdict line regardless of format; only the JSON gate path
was correct and test-pinned. Every verdict and warning line now goes to
stderr in every mode, and a contract test pins the channel across text
and SARIF (using a trivially-passing `max_dependency_cycles` gate —
`code_health_min` degrades to FAIL on a rowless scratch repo, a fixture
behavior worth knowing).

### F354 (Fixed — Unreleased) — the MCP server half-validated its startup and hard-coded its cache root

A typo'd repo path in a client config produced a healthy-looking server
that failed on every tool call — while both calibration artifacts were
fail-fast validated three lines away. The repository is now opened and
HEAD resolved before serving, with the fix named in the error.
Alongside it, the server gains the `--cache-dir`/`--temp-dir` overrides
every other fact-store-touching subcommand already had: one resolved
`cache_root` field replaces eleven per-tool default lookups (an override
cannot miss a tool), and the spill override threads through a new
`base_options()` seam into every tool's `Options` — the seam the
second-wave structural audit recommended, scoped so calibration
artifacts stay per-handler and regenerated artifacts still never
invalidate the memo of a tool that does not read them. Startup refusal
and cache-dir placement are both test-pinned; the startup probe removed
the validation block and watched the test fail.

### F355 (Fixed — Unreleased) — `--calibration` missing on `gate` and `explain` while their MCP twins carried it

F323 fixed the MCP-missing direction of the calibration asymmetry and
left the CLI-missing one open: `gate_changes` and `explain_file` thread
the corpus artifact while the CLI subcommands built `Options` without
it. `codelore explain <path>` and the MCP `explain_file` printed
different corpus percentiles for the same file at the same HEAD under a
custom corpus — the number the advisory narrative's citation check
grounds against — and the `gate` half fragmented the report cache into
two entries for identical work. Both subcommands now accept and thread
the flag exactly as `--defect-calibration` already did; the parity test
drives `explain` against a ramp corpus that must move the printed lens,
and the probe (threading stashed, flag accepted-but-ignored) fails on
identical outputs — F323's exact silent-ignore shape. `diff` is
deliberately untouched: it has no calibration-consuming MCP twin
(`delta_health` is artifact-blind by design).

### F356 (Fixed — Unreleased) — dashboard: unreadable circle-pack lenses, boot-loop blank page, and an unclosable drawer

Four SPA defects in one batch, each verified in a real browser. The
circle-pack's metric literal omitted `ai_pct`, `mi`, and `mi_rank`, so
switching the size/color lens to any of the three silently rendered the
default metric — the selector claimed one thing and drew another. A
throwing panel in the boot loop aborted every panel after it, blanking
the rest of the dashboard with no signal; each panel now boots inside a
try/catch that leaves an in-place failure note and lets its siblings
continue. The detail drawer advertised `role="dialog"` but ignored
Escape; it closes now. And the tour plus the injected click targets
were mouse-only (`tabindex` missing), unreachable by keyboard. The hash
contract also gained the two architecture-view keys the SPA already
wrote but refused to read back (closed-enum validated on read,
emitted only when non-default). Browser tests pin the metric wiring via
the build-hierarchy hook and the Escape-close path.

### F357 (Active) — `--format step-summary --output -` still creates a literal `-` file

F351 taught `emit_to_output_or_stdout` the `-` = stdout convention and
gated the three file-path formats (parquet, sqlite, spa) behind an
explicit rejection — but the step-summary dispatch takes neither path:
it writes the raw `--output` value through `atomic_publish` with no dash
filter and no rejection, so `--format step-summary --output -`
reproduces F351's exact failure shape — a junk file named `-` plus a
`✓ step-summary written to -` confirmation. Either route the dispatch
through the shared dash filter or add step-summary to the rejection
list; the default stdout path (no `--output`) is unaffected, and no
documented recipe exercises the combination. Surfaced by the 2026-09-02
docs-currency audit while verifying F351's coverage.

### F358 (Active) — `codelore diff`'s SARIF emitter still carries the pre-F347 severity shape

F347 aligned the analyze/check emitter (`output/sarif.rs`): hotspot
`security-severity` became `max(0.1, (100 − cognitive_health) / 4)`
with band-derived levels. The CLI's own diff emitter
(`diff_output.rs`) was not touched: rank-entrant hotspot results still
compute `(100 − cognitive_health) / 10` clamped to [0, 10],
score-increase results carry a hardcoded `warning` level and no
`security-severity` at all, and diff-mode CODELORE-CLONE emits a fixed
`note` level with no severity. A Code Scanning instance consuming both
surfaces ranks the same file differently depending on which command
produced the finding. The user guide's SARIF rule table is scoped to
the analyze/check emitters until the two share one severity mapping.
Surfaced by the 2026-09-02 docs-currency audit.

### F359 (Fixed — Unreleased) — three silent-pass holes closed, and the new cross-backend gate caught a real oracle divergence

Three guard classes, one immediate payoff. (1) The spa-browser CI job
now exports `CODELORE_REQUIRE_BROWSER` and every Chrome-skip site
asserts it is unset before skipping, so a broken Chrome install fails
the only JS-executing job instead of producing a silent two-minute
green; probe: a sabotaged launcher fails the suite with the guard's
message under the env var and still skips cleanly without it. (2)
`check`'s gate evaluator exhaustively destructures the thresholds
struct — every field annotated as gate, policy, or modifier — so an
unclassified new threshold is a compile error at the evaluator; probe:
a synthetic field fails the build with E0027 at the destructure. (3)
The per-table fact-store digest moved to `test_support`, and a new
differential test ingests the fixture through BOTH `Repo` backends
requiring byte-identical digests per table — the promise CLAUDE.md
states but no test ever checked end-to-end (`hunks` excluded by name
for the recorded emits-none divergence, with an assert that deletes
the exclusion the day it stops being true). First run caught
`GitCliRepo` trimming the trailing newline git stores in every commit
message (`trim_end_matches` on the `%B` field) while `GixRepo`'s
`message_raw()` keeps it — all fifty fixture rows diverged; the oracle
now keeps `%B` byte-exact, and the probe is the trim itself restored.
Production walker untouched; no cache impact.

### F360 (Fixed — Unreleased) — assorted hardening: two reachable panics, a self-defeating pruner, and a silent no-op lens

Four fixes from the verified-hardening list. (1) `format_history`
byte-sliced the hand-editable ledger's `head_sha` at 12 bytes —
a multi-byte character straddling the boundary panicked the `check`
run; now a char-boundary prefix, with the two provenance-safe
truncation siblings (`delivery.rs` short rev, `calibrate` vintage)
moved to the same idiom. Probe: the restored slice panics the new
multi-byte test with the exact "byte index 12 is not a char boundary"
message. (2) All fifteen `partial_cmp(..).unwrap_or(Equal)` float
sorts became `total_cmp` — the house pattern in seven other files;
Equal-on-NaN is intransitive and modern sorts may detect and panic.
(3) `prune_global_cache` gained a keep-path: it runs between the
ingest renaming the just-written entry into place and re-opening it,
so a still-binding byte cap deleted the file the run was about to
read. Probe: without the keep-path the zero-cap test loses the entry.
(4) `CalibrationArtifact::validate` rejects the fully empty artifact
(no `languages` pools AND no `repo_metrics` pools) — it passed every
structural check while making every lookup report "not in corpus"
with no signal; `calibrate` already refused to write one, now a
hand-built one cannot be read. First cut rejected `languages: []`
alone and CI caught three tests legitimately exercising pools-only
artifacts — the architecture percentiles need no per-language pools,
so that shape stays valid and the two no-pools test stand-ins now
carry the language entry their real-world counterparts have. Probe:
with the check removed, the rejection test's `expect_err` fails on
`Ok`; the same test pins the pools-only acceptance each run.

### F361 (Fixed — Unreleased) — the MCP startup guard's repo errors never reached the exit-code mapping

The `codelore mcp` startup fail-fast raised both failures (repository
open, HEAD resolution) through `anyhow::anyhow!("... {e} ...")`.
Interpolating `{e}` renders the error into a string; it does not survive
as a source. `main`'s `e.chain().find_map(downcast_ref::<CodeLoreError>())`
therefore found nothing and `map_or(1, exit_code)` returned 1, while
spec §6.6 assigns 3 to a repository error. Behaviour and message were
correct throughout — only the number was wrong, on precisely the
misconfiguration path the guard was added to serve.

Both sites now use `anyhow::Error::new(e).context(...)`, the construction
`preflight_and_open_repo` already uses and which keeps the variant in the
chain (`Error::context` is inherent on `anyhow::Error`, so no trait
import is involved). Probe: restoring the interpolating form makes the
test report `Some(1)`.

Why it survived review and a full CI matrix: the covering test asserted
a bare `!status.success()`, which accepts any nonzero status — including
the 101 a panic yields under this workspace's unwind strategy, and a
signal kill, whose code is `None`. It now pins `Some(3)`. This is the
concrete argument for the exit-code assertion sweep: tightening one
assertion exposed a live contract violation, so that work is not
hygiene.

A sweep of every `anyhow!` interpolation in the CLI found only these two
sites. The two remaining ones wrap `rmcp` transport failures, which carry
no `CodeLoreError` to preserve, so their fallback to 1 is correct.

### F362 (Fixed — Unreleased) — `--format step-summary --output -` wrote a file named `-`

The dash gate in `analyze.rs` rejects `-` for `parquet | sqlite | spa`,
formats that cannot stream at all. `step-summary` streams to stdout by
default, so the dash is meaningful for it — but `run_step_summary_dispatch`
passed `args.output` straight into `atomic_publish` without the filter its
sibling `emit_to_output_or_stdout` applies to the identical
`Option<&Path>`, so the dash took the file branch and created `./-` while
the caller's redirect captured nothing.

Neither documented recipe reaches this on its own: `docs/advanced-usage.md`
uses bare streaming for step-summary, and the README's `--output -` example
is on the `diff` path, which routes through the filtering emitter. The
defect needs the two idioms crossed, which the docs invite by teaching the
dash as the conventional spelling of stdout in one place and default
streaming in another. Recorded because the inconsistency, not the recipe,
is the bug: two output-routing paths in one file disagreed about what `-`
means.

Fixed by applying the same filter. Probe: removing it fails the new test,
which pins both that the summary reaches stdout and that no `-` file is
left in the working directory. No test previously exercised step-summary's
output routing at all — the format's only prior appearance in the CLI suite
was the time-bucket rejection.

### F363 (Active) — the advisory layer's request timeout is a hardcoded constant with no operator override

`enrichment/client.rs::build_agent` gives the shared `ureq` agent a single
`timeout_global` of `REQUEST_TIMEOUT_SECS`, a `pub const` fixed at 120,
documented as covering connect through body read. There are no retries,
so that constant is the entire budget a generation gets. Every other knob
the chat client reads is an environment variable resolved through
`LlmEnv` — provider, base URL, API key, model — and the timeout alone is
not; there is no CLI flag for it either. `advanced-usage` §8 describes it
only as "a single bounded timeout and no retries", naming no way to move
it, which is accurate precisely because no way exists.

The ceiling is not theoretical. An independent evaluation of the advisory
layer — 612 generations across six repositories through an
OpenAI-compatible gateway, published against the narrative-receipts issue
with its raw per-run records — recorded a maximum request of 117.7s
against the 120s bound, 98% of the budget, on a model whose mean was
23.6s. Its authors patched the constant and rebuilt before they could
evaluate a slower model at all, and disclosed that patch as a protocol
deviation. The failure mode this predicts for an ordinary user is sharp:
`--llm` against a reasoning model, a cold local runtime, or a loaded
gateway aborts the run, and the only remedy in a released binary is
editing source and recompiling. `MAX_TOKENS` already bounds a runaway
generation independently, so the timeout is not what protects against
unbounded output — it only decides how slow a *legitimate* response may be.

Left open rather than fixed inline, deliberately. The fix introduces a
new public environment variable, which is API surface the project has to
live with, and the shape is a design decision rather than a correction:
whether one `CODELORE_LLM_TIMEOUT_SECS` is the right granularity when
`timeout_global` bundles connect and read; whether an unparseable, zero,
or absurd value is rejected at the `Options::validate` boundary in the
style of the other cross-field rules or clamped silently; and whether the
default should move at the same time, given that the one measurement in
evidence sat at 98% of it. Naming that ceiling in the documentation is
worth doing regardless of which shape wins, since today a user meeting it
has no way to learn it exists.

### F364 (Fixed — Unreleased) — PR-mode SARIF kept the pre-correction severity scale

`diff_output.rs` computed `((100 - cognitive_health) / 10).clamp(0, 10)`
with a hardcoded `"level": "warning"`, while `output/sarif.rs` had moved to
`((100 - health) / 4).max(0.1)` with the level derived from it. Same rows
(`HotspotRow`), same repository, two grades — decided by whether `analyze`
or `diff` wrote the file.

The consequence is one-directional and lands on the worst surface.
`cognitive_health` is bounded to [60, 100], so a tenth-scale severity never
exceeds 4.0: the `error` band was unreachable, and a structurally healthy
file emitted 0.0, which GitHub renders as "no severity". The under-reporting
surface is the one that posts to pull requests.

The derivation moved into `output/sarif.rs` as `health_grade`, returning
severity and level as one value so they cannot disagree, called by both
emitters. This follows an existing seam rather than creating one:
`diff_output.rs` already imports `diff_finding_hash` and
`primary_location_line_hash` from that module. `score_increased` results
are deliberately untouched — `ScoreDelta` carries no health field, so a
grade would have to be invented.

Probe: restoring the `/ 10` divisor fails the new sweep with
`left: ["note", "warning"]` against `right: ["error", "note", "warning"]` —
the unreachable band, named. That property is why the test sweeps the range
instead of asserting a value: every single-value assertion still agreed with
itself under the broken divisor, which is how the drift survived. A stale
comment in the SARIF test file still documenting the `/ 10` formula was
corrected with it.

The next sweep re-opens at **F365**.
