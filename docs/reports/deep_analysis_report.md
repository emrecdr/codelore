# CodeLore — Deep Codebase Analysis Report

Read-only audit log. Findings are immutable F-IDs; the status field tracks state.
Shipped/fixed findings are condensed to a one-line closure row once validated against `main` (full history in `CHANGELOG.md` + git); refuted findings stay documented to prevent rediscovery.

**Last pass: 2026-07-02.** The 2026-07-01 validation + 5-dimension discovery pass added **F200–F230**; the 2026-07-02 implementation pass landed 28 of them on `main` (PRs #71, #74) and refuted F188/F202. See §3 "Implemented" tables + §6 for the current disposition.

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

### 4.1 Carried forward from prior passes (re-validated 2026-07-01)

#### F173 — Same HEAD blobs read + tree re-walked up to 3× across complexity/clones/imports
*   **Location**: `facts/ingest/mod.rs:145,158,165` (three sequential passes); each `*_head.rs:55` independently calls `read_blob_at_head`
*   **Severity**: HIGH · **Category**: performance (redundant I/O) · **Status**: Active
*   **State on main**: Still three sequential HEAD passes each re-reading live blobs. Only `head_rev`/`live_paths` were hoisted once (SQL path-queries no longer repeated); blob reads + tree walks still happen 3×. Deepened by the newly-found per-file re-resolution cost in **F206** — even a single deduped pass keeps paying F206's per-file HEAD/commit/tree decode.
*   **Deferral blocker**: divergent extractor error contracts (clones aborts ingest via `collect::<Result>>?`; complexity/imports warn-and-skip) + the memory-regression risk of hoisting all live blobs into one map. Needs a bounded shared-blob LRU or unified error contracts first.

#### F119 — Hand-rolled CSV emitter (now 1122 LOC) instead of the `csv` crate
*   **Location**: `output/csv.rs`
*   **Severity**: MED · **Category**: tool replacement · **Status**: Active (re-scoped)
*   **State on main**: Still hand-rolled (`wc -l` = 1122, up from ~826; no `csv` dep). **Re-scope note**: no longer a clean byte-identical swap — the emitter now carries a deliberate formula-injection guard (F170) and `\n` line endings; the `csv` crate would change both. Any migration must preserve the injection guard + line-ending contract, or the swap is rejected.

#### F148 — `csv.rs` + `markdown.rs` per-analysis emitters, no shared row abstraction
*   **Location**: `output/csv.rs` (~34 KB, 43 `write_*` fns), `output/markdown.rs` (~36 KB)
*   **Severity**: LOW · **Category**: copy-paste drift · **Status**: Active
*   **State on main**: Both grew past the previously-noted ~25 KB; still one `write_*` fn per analysis, no `TabularEmit`/row trait. Coupled to F119 (csv-crate) + F161 (streaming) — treat as one output-layer cluster.

#### F161 — Every emitter materializes the full `Vec<Row>` — no streaming path
*   **Location**: `output/json.rs:29`, `sarif.rs:90`, `markdown.rs` — all `rows: &[T]`
*   **Severity**: LOW · **Category**: memory architecture · **Status**: Active
*   **State on main**: All emitters still take a fully-materialized slice; no `EmitterStream`. Peak memory grows with row count; a 200k-path monorepo CSV export can spike multi-GB. SARIF stays batch (needs run-level totals); CSV/JSON/markdown are the streamable targets.

#### F177 — Three schema-version sentinels still coexist
*   **Location**: `facts/schema.rs:10` (`CURRENT_SCHEMA_VERSION="3"`), `cache.rs:25` (`CACHE_EPOCH="schema_v5"`), `schema_v1.sql` filename literal; stray `"schema_v3"` help-text at `main.rs:373`
*   **Severity**: MED · **Category**: duplicated source-of-truth · **Status**: PARTIAL
*   **State on main**: Both named sub-fixes landed — CLI `profile` now derives the schema string from `CURRENT_SCHEMA_VERSION`, and the cache sentinel was renamed to the honest `CACHE_EPOCH` (matches CLAUDE.md). But three version constants remain structurally disjoint (none derived from another) and a stray `"schema_v3"` help literal persists. Residual: unify or cross-reference the three; fix the stray literal.

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

The 5-dimension fan-out logged F200–F230; 25 landed in the 2026-07-01 pass and F200 (+ F188/F202 refutations) in the 2026-07-02 pass (see the §3 "Implemented" tables). The entries below are the deferred remainder — each is a large refactor with regression surface, a dependency-migration needing CI validation, or a low-value mechanical sweep, not a quick safe change. Since then F206 (HEAD-scan blob I/O) and F230 (gix bump) have shipped — marked Fixed inline below; F215 and F218 are the open remainder.

#### Backend performance

##### F206 — `read_blob_at` re-resolves HEAD→commit→root-tree per file and discards the gix object cache each call
*   **Location**: `repo/gix_repo.rs:293-329` (`to_thread_local()` per call → `rev_parse_single` → `find_commit` → `commit.tree()` → `lookup_entry_by_path`); default wrapper `repo/mod.rs:99-101`
*   **Severity**: HIGH · **Category**: blob I/O / redundant recomputation · **Status**: **Fixed (v0.25.0)**
*   **Description**: Every HEAD-time blob read mints a fresh thread-local `Repository` (cold object cache), re-resolves `HEAD`, re-decodes the commit + root tree, and re-walks + re-decodes every intermediate directory tree — for *each* file. A file at depth `d` re-decodes `d` tree objects; every sibling re-decodes its parent tree again. Three HEAD passes × F live files = 3F redundant resolves. This is distinct from and **deeper than** F173 (which only dedups the blob across passes — the per-file HEAD/commit/tree re-decode remains even in one deduped pass). Dominant cost of HEAD scans on large deep-nested monorepos.
*   **Suggested improvement**: Resolve HEAD → root tree once per pass and reuse it (batch `read_blobs_at_head(paths)` walking a single cached tree), or hold one `to_thread_local()` repo with `object_cache_size` enabled across the file loop. Same bytes returned — output-neutral, faster.
*   **Outcome (v0.25.0)**: shipped as this finding's suggested improvement — `Repo::blob_reader_at(rev)` returns a `BlobReader` whose `read(path)` is byte-identical to `read_blob_at`, and `GixRepo` overrides it (`repo/gix_repo/mod.rs`) to resolve the root tree once per `rayon` worker (via the existing `map_init` idiom) and reuse a warm `gix` object cache for every file that worker subsequently reads. The differential oracle (`GitCliRepo`) uses the default per-call forwarder, so the two-backend parity is unchanged (byte-identical ingested facts proven before/after). Landed with the F173/F253 Phase-1 HEAD-scan work; the remaining three-pass blob dedup (F173) stays open.

#### Rust idioms / error handling

##### F215 — Stringly-typed `format: &str` re-matched with `unreachable!()` in ~11 dispatchers
*   **Location**: `codelore-cli/src/main.rs:705` + sibling dispatch fns; also `args.output…expect("validated above")` at `:751/757`
*   **Severity**: LOW · **Category**: type-safety / simplification (optional) · **Status**: Deferred (large refactor)
*   **Description**: `--format` is validated once then re-matched in many dispatchers, each carrying an `unreachable!("format validated…")` arm — a hand-maintained invariant a parse-once `enum Format` would make compile-time-total, deleting the arms + the "validated above" coupling.
*   **Suggested improvement**: Parse `--format` into a `Format` enum at the boundary and thread it through dispatch. A non-trivial refactor across ~11 dispatchers — flagged, not forced.

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

The 2026-08-02 discovery pass logged **F249–F268** (see §7); F269 was logged this pass (below). The post-v0.26.0 deferred-backlog pass logged **F270–F272** and closed F255/F269 (see §8). The post-v0.26.0 first-run UX pass logged **F273–F283** (see §9); its 0.27.0 re-verification added **F284–F286**. The next sweep should re-open with F-IDs starting at **F288**; F287 was logged out of cycle (see §9).

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

**Pointer:** `calibrate_defects` temporal train/validation positive-leakage (fully diagnosed in
`2026-07-28-hardening-cycle-3.md` §A2-1, reconfirmed HIGH cycle-4) remains open — a rigor defect
inside the calibration-honesty machinery; don't drop it when triaging.

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

### F271 (Active) — MCP tools hand-roll JSON into a text block instead of declaring structured output

*   **Location**: `codelore-cli/src/mcp.rs` — all eleven `#[tool]` bodies return `Result<String, ErrorData>`
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

## 9. First-run UX pass (F273–F283)

Validated against the shipped 0.26.0 binary, not inferred. This section is
the F-ledger and the record of the pass; `2026-08-06-first-run-ux-review.md`
carries only what is still open — F276, F278, F279's remaining instances, and
the deferred thresholds scaffold that F276 blocks.

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

### F276 (Active) — `evaluate_all_gates` discards measured values it already computed

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

### F278 (Active) — the hygiene guard's ID vocabulary is `F`-plus-digits only

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

### F279 (Fixed — v0.27.1, partial) — a ticket ID shipped in user-facing help

*   **Location**: `args.rs` (fixed); `analyze.rs`, `explain.rs`, `options.rs`, `clone_coupling.rs` (remaining)
*   **Severity**: LOW · **Category**: convention violation
*   **Description**: `codelore analyze --help` printed `T8: An author is
    considered "departed"…`. The project forbids ticket IDs in code, and this
    one reached the published binary.
*   **Outcome**: the help-text instance is removed. Instances remaining in
    library doc comments and inline comments are not user-facing but are the
    same violation; clearing them is gated on F278's anchored rule, so they are
    fixed and guarded together rather than piecemeal.

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

### F290 (Fixed — Unreleased) — the two remaining MCP tools that returned unbounded violation lists

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

### F291 (Fixed — Unreleased) — a `limit` schema description contradicted the handler for three cycles

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

The next sweep re-opens at **F292**.
