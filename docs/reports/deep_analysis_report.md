# CodeLore — Deep Codebase Analysis Report

Read-only audit log. Findings are immutable F-IDs; the status field tracks state.
Shipped/fixed findings are condensed to a one-line closure row once validated against `main` (full history in `CHANGELOG.md` + git); refuted findings stay documented to prevent rediscovery.

**Last pass: 2026-07-02.** The 2026-07-01 validation + 5-dimension discovery pass added **F200–F230**; the 2026-07-02 implementation pass landed 28 of them on `main` (PRs #71, #74) and refuted F188/F202. See §3 "Implemented" tables + §5 for the current disposition.

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
    D -->|SQL views / parameterized queries| G[54 behavioral analyses]
    G -->|emitters| H[CSV · JSON · SARIF 2.1.0 · Markdown · Parquet · SQLite · HTML · SPA · GHA]
```

1.  **Repository Traversal**: `GixRepo` (pure-Rust `gitoxide`, hot path) + `GitCliRepo` (differential-testing oracle).
2.  **Event Ingestion**: `duckdb::Connection` is `!Send + !Sync`. Producer-consumer: background thread walks commits → bounded `crossbeam-channel` → connection-owning thread runs DuckDB Appender (`facts/ingest/consumer.rs::ingest_loop`).
3.  **HEAD-time work**: complexity, clones, imports extraction read blobs from the gix ODB, parse via tree-sitter on a rayon pool, drain serially into the DuckDB Appender.
4.  **SQL-Driven Analyses**: 54 behavioral analyses run as parameterised DuckDB queries. Path-aggregating analyses opt into rename-aware aggregation via the `changes_lineage` CTE rewriter.

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

The 5-dimension fan-out logged F200–F230; 25 landed in the 2026-07-01 pass and F200 (+ F188/F202 refutations) in the 2026-07-02 pass (see the §3 "Implemented" tables). The entries below are the deferred remainder — each is a large refactor with regression surface, a dependency-migration needing CI validation, or a low-value mechanical sweep, not a quick safe change.

#### Backend performance

##### F206 — `read_blob_at` re-resolves HEAD→commit→root-tree per file and discards the gix object cache each call
*   **Location**: `repo/gix_repo.rs:293-329` (`to_thread_local()` per call → `rev_parse_single` → `find_commit` → `commit.tree()` → `lookup_entry_by_path`); default wrapper `repo/mod.rs:99-101`
*   **Severity**: HIGH · **Category**: blob I/O / redundant recomputation · **Status**: Deferred (own perf pass)
*   **Description**: Every HEAD-time blob read mints a fresh thread-local `Repository` (cold object cache), re-resolves `HEAD`, re-decodes the commit + root tree, and re-walks + re-decodes every intermediate directory tree — for *each* file. A file at depth `d` re-decodes `d` tree objects; every sibling re-decodes its parent tree again. Three HEAD passes × F live files = 3F redundant resolves. This is distinct from and **deeper than** F173 (which only dedups the blob across passes — the per-file HEAD/commit/tree re-decode remains even in one deduped pass). Dominant cost of HEAD scans on large deep-nested monorepos.
*   **Suggested improvement**: Resolve HEAD → root tree once per pass and reuse it (batch `read_blobs_at_head(paths)` walking a single cached tree), or hold one `to_thread_local()` repo with `object_cache_size` enabled across the file loop. Same bytes returned — output-neutral, faster.
*   **Deferral reason**: Restructures the hot HEAD-scan loop and overlaps the F173 blocker (divergent extractor error contracts); wants its own focused perf pass rather than riding this batch.

#### Rust idioms / error handling

##### F215 — Stringly-typed `format: &str` re-matched with `unreachable!()` in ~11 dispatchers
*   **Location**: `codelore-cli/src/main.rs:705` + sibling dispatch fns; also `args.output…expect("validated above")` at `:751/757`
*   **Severity**: LOW · **Category**: type-safety / simplification (optional) · **Status**: Deferred (large refactor)
*   **Description**: `--format` is validated once then re-matched in many dispatchers, each carrying an `unreachable!("format validated…")` arm — a hand-maintained invariant a parse-once `enum Format` would make compile-time-total, deleting the arms + the "validated above" coupling.
*   **Suggested improvement**: Parse `--format` into a `Format` enum at the boundary and thread it through dispatch. A non-trivial refactor across ~11 dispatchers — flagged, not forced.

#### SPA / UI / UX

##### F218 — Any single layout-selector change re-renders every widget (full-dashboard cascade)
*   **Location**: `output/spa/template.html` (one `Alpine.effect` subscribing to all layout knobs → all `_codeloreRerenderers`)
*   **Severity**: MED-HIGH · **Category**: render performance · **Status**: Partially Fixed (Unreleased)
*   **Description**: Bumping the Kamei window 30→60 (one sparkline) re-runs `d3.pack` over the whole hotspot tree, rebuilds every ECharts instance, and re-lays-out the arch graph + DSM. The code yields between rerenderers to stay responsive — treating the symptom. The scenario toggle also auto-clicks the knowledge-loss tab, double-rendering the circle-pack on the first pick.
*   **Partial fix**: Split the monolithic `Alpine.effect` into (a) a pure theme effect that reads only `store.theme.isDark` and fires `_codeloreRerenderers` with cooperative yield (F135), and (b) a separate layout/offboarding effect that reads `store.layout.*` and `store.scenario.departed` — so theme toggles no longer chain through layout/scenario subscriptions or trigger the CSS-token invalidation pass from unrelated clicks. The cross-widget selection and brush effects were already separate.
*   **Residual (open)**: layout/scenario changes still fire ALL registered rerenderers — the headline Kamei-window scenario above is unchanged. Remaining follow-on: key the rerenderer registry by the store fields each widget depends on, so a layout change re-renders only its subscribers.

#### Code hygiene

##### F231 — Comprehensive `Plan N` version-phase marker sweep (comment rule violation)
*   **Location**: **62 comment sites across 25 files** (validated 2026-07-02 via `grep -rn "Plan [0-9]" crates/codelore-lib/src crates/codelore-cli/src` → 62; files include `types.rs`, `analysis.rs`, `constants.rs`, `options.rs`, `output/{mod,sarif,parquet}.rs`, `provenance/mod.rs`, `clones/*`, `complexity/*`, `repo/{mod,git_cli_repo,gix_repo}.rs`, `facts/{schema_v1.sql,ingest/*}`, `arrow_facade.rs`, `codelore-cli/src/{args,diff,diff_output,main}.rs`)
*   **Severity**: LOW · **Category**: comment rule violation (no version/task markers) · **Status**: Active (deferred — dedicated scripted sweep)
*   **Description**: F164 swept `F<NN>` finding-IDs out of comments but left `Plan N` phase markers, which are the same banned class under the project's no-version/task-markers-in-comments hard rule. F205 fixed the one *factually-wrong* instance (`gix_repo.rs:355-356`); 62 more remain, several also stale (e.g. `repo/mod.rs:1-2` "the default impl is `gix` in Plan 1; a `GitCliRepo` … lands in Plan 6"). Deferred as a dedicated scripted sweep (like F164) rather than 62 hand-edits riding an unrelated branch — mixing prefixes, parentheticals, and stale future-tense claims, so a blind `sed` would mangle grammar.
*   **Suggested improvement**: A mechanical comment-only sweep like F164's — drop each `Plan N` marker, keep or correct the surrounding rationale (some are false future-tense claims, e.g. "Plan 4 will add X" for X already shipped). Leave vendored `codelore-rca` (MPL fork) untouched.

#### Dependency currency (verify latest before acting — assessed offline from declared/resolved versions)

Overall hygiene is strong: `thiserror 2`, `toml 1`, `ureq 3`, `anyhow`/`serde`/`clap`/`rayon`/`time`/`percent-encoding` all current. `tree-sitter*` + `petgraph` are deliberately pinned (CLAUDE.md) — out of scope. Two items worth active tracking:

##### F230 — `gix` 0.84 → 0.85 bump
*   **Location**: `crates/codelore-lib/Cargo.toml`
*   **Severity**: LOW · **Category**: dependency currency (routine) · **Status**: **Fixed (merged via #74)**
*   **Outcome (2026-07-02)**: bumped `gix 0.84 → 0.85` (latest, published 2026-06-22) with `provenance::GIX_VERSION` + banner refs. API-compatible — the two-backend differential harness (`differential_repo_test.rs`) passed unchanged, so `GixRepo` still matches the `git`-CLI oracle. Consolidated Dependabot #68 (whose only failure was the drift guard) into #74; full CI matrix green. Related closed dep PRs: #69 (arrow 58→59 — deferred, would desync from duckdb's pinned arrow 58), #70 (duckdb group — superseded by F229).

### 4.3 Code-health composite score — design observation (2026-07-02)

#### F232 — Coupling centrality counted twice in the composite code-health score

*   **Location**: `analyses/code_health.rs` — `SHOTGUN_INSERT` (reads `coupling_centrality_v1`, writes `shotgun-surgery` biomarker) + `normalized` CTE `n_cp` term (also reads `coupling_centrality_v1` directly)
*   **Severity**: LOW · **Category**: scoring design / weight calibration · **Status**: Fixed (Unreleased)
*   **State on main**: `coupling_centrality_v1` previously fed the composite score via two independent paths: (1) the `shotgun-surgery` biomarker (`intensity = PERCENT_RANK(ORDER BY centrality)`) flowing into `structural_risk`; and (2) directly as `n_cp = normalize(centrality)`. A high-centrality file was penalised through both at once — a double-count of the same signal.
*   **Fix**: the `n_cp` term and its centrality join were removed from the score SQL; coupling now enters the composite once, as the shotgun-surgery biomarker inside `structural_risk`. Score reweighted to `w_sr = 0.50, w_cn = 0.30, w_au = 0.20` (sum 1.0). `CACHE_EPOCH` bumped.

### 4.4 Refactoring-targets analysis — cross-analysis contract and display fidelity (2026-07-03)

#### F233 — Implicit cross-analysis contract: `refactoring-targets` consumes `code_health_biomarkers_v1` temp table as a side-effect

*   **Location**: `analyses/refactoring_targets.rs` (dominant-biomarker lookup query reads `code_health_biomarkers_v1`); `analyses/code_health.rs` (`materialize_biomarkers` creates the table as a side effect of `run_code_health`)
*   **Severity**: MED · **Category**: implicit contract / latent-robustness · **Status**: Fixed (Unreleased)
*   **Description**: `refactoring-targets` is the first external consumer of the `code_health_biomarkers_v1` temporary table, which `run_code_health` materialises as a side effect via `materialize_biomarkers`. This elevates a private implementation detail of `code-health` into an implicit cross-analysis contract: if `code_health` dropped the table before returning, or the call order in `run_refactoring_targets` changed so the biomarker query ran before `run_code_health`, the dominant-biomarker lookup would fail at runtime with a DuckDB "table not found" error.
*   **Fix**: the `BIOMARKERS_DDL` in `analyses/code_health.rs` now carries a doc-comment documenting the external consumer and the session-scoped contract (must stay session-scoped and readable after `run_code_health` returns; columns not renamed without updating the consumer). The two `refactoring_targets` integration tests exercise the full path and guard a runtime regression.

#### F234 — `loc` display floor: files with no LOC entry show `loc = 1` (fabricated value)

*   **Location**: `analyses/refactoring_targets.rs` — `loc_by_path.get(...).unwrap_or(0).max(1)` used for both the displayed `loc` column and the priority denominator
*   **Severity**: LOW · **Category**: display fidelity / cosmetic · **Status**: Fixed (Unreleased)
*   **Description**: A file with no non-NULL LOC entry (e.g. a file the complexity walker skipped) had its `loc` stored as `0.max(1) = 1` — a fabricated value propagated directly into the output row. The `EA_Z_FLOOR` (25) already floors the priority denominator independently, so the `.max(1)` affected only the displayed `loc`.
*   **Fix**: `refactoring-targets` now reports the true `loc` (`0` = no LOC data); the EA-Z effort floor is confined to the priority denominator (`max(loc, EA_Z_FLOOR)`). The integration-test assertion was updated accordingly.

#### F235 — `structural_risk` saturated at the ceiling on real repositories

*   **Location**: `analyses/code_health.rs` — `BIOMARKERS_INSERT` (`ranked`/MAX rollup) + `file_structural` (probabilistic-OR aggregate)
*   **Severity**: HIGH · **Category**: metric quality / scoring · **Status**: Fixed (Unreleased)
*   **Description**: Empirical run on this repo showed **67 of 69 files at `structural_risk = 1.0000` and 100% `red` band** — the metric did not discriminate. Two compounding causes: (1) `BIOMARKERS_INSERT` ranked FUNCTIONS by `PERCENT_RANK` then took the per-file `MAX`, so any file with enough functions had one in the top percentile → intensity ≈ 1.0; (2) `file_structural` combined intensities with a probabilistic-OR (`1 − Π(1−intensity)`) × co-occurrence multiplier, which drove any file with ≥2 high smells to the `LEAST(1.0, …)` clamp. The per-row invariant tests (range/monotonicity/determinism) all passed while the distribution was degenerate.
*   **Fix**: (1) rank FILES not functions — aggregate to the file first (`MAX` per file), then `PERCENT_RANK` across files, so each smell's intensity is uniformly spread; (2) replace the probabilistic-OR with a bounded weighted sum (per-smell weights summing to 1.0, absent smells contribute 0, co-occurrence implicit). Empirical result on this repo: `structural_risk` spreads `0.01–0.96`, band split **8 red / 32 yellow / 31 green** (thresholds `0.55 / 0.28`). Added a distribution regression test (`code_health_structural_risk_discriminates`) on a purpose-built `biomarker_repo` fixture. Residual (Phase-2): small per-language cohorts (e.g. a handful of JS files) make `PERCENT_RANK` coarse — the cross-repo corpus percentile addresses this.

#### F236 — biomarker normalization inconsistency: god-class/dry used min-max while others used percentile

*   **Location**: `analyses/code_health.rs` — `materialize_biomarkers` (god-class + dry insertion)
*   **Severity**: MED · **Category**: metric quality / scoring · **Status**: Fixed (Unreleased)
*   **Description**: After F235, three biomarkers (complex-method, large-method, shotgun-surgery) used per-file `PERCENT_RANK` but god-class and dry still used min-max `/max` — an outlier-dominated scheme that under-discriminated (one extreme god class compresses the rest toward 0). Naively switching them to `PERCENT_RANK` *among files that have the smell* was worse (a lone or tied occurrence — e.g. two files each with one clone — collapses to a 0 rank, losing the DRY signal entirely; verified on the fixture).
*   **Fix**: god-class and dry now rank their raw metric over the FULL per-language file universe (absent files contributing 0), matching complex/large's scheme exactly (same `cyclomatic IS NOT NULL AND loc IS NOT NULL` filter). A file with the smell ranks above the zero-majority (e.g. a tied duplicated pair → ~0.8, not 0 or a min-max 1.0). (shotgun-surgery keeps its own pre-existing universe — the coupled-file set from `coupling_centrality_v1`, not language-partitioned; unifying that too is a separate follow-up.) Band thresholds retuned `0.50/0.25 → 0.55/0.28` to keep the red set selective after god-class intensities rose. Added `biomarker_repo` fixture + tests: `code_health_structural_risk_discriminates`, `code_health_biomarkers_fire_distinct_smells` (closes the T2-M2 dropped-`UNION`-arm gap), and `refactoring_targets_dominant_type_varies_on_biomarker_repo` (closes the P2-T2-M1 gap). `CACHE_EPOCH → schema_v8`.

#### F237 — Phase 1/2 deep-validation audit fixes

*   **Location**: `main.rs` (explain, check gate, refactoring-targets dispatch), `analyses/code_health.rs` (module doc), `analyses/refactoring_targets.rs` (dominant-type query), `quality_gates/mod.rs`, tests
*   **Severity**: HIGH (aggregate) · **Category**: correctness / consistency / test coverage · **Status**: Fixed (Unreleased)
*   **Description**: A five-stream adversarial audit of the Phase-1/2 changes confirmed the core computations correct (bounded, NaN-free, deterministic) but surfaced: (a) HIGH — `codelore explain code-health` printed the pre-F232 formula (`0.40/0.25/0.15` + a separate coupling term, `0.66/0.33` bands); (b) HIGH — the `check` gate `code_health_min` evaluated the hotspots inline cognitive-only proxy (floored 60), not the composite, so a red file (composite ~20) silently passed a `code_health_min = 70` gate; (c) MED — `refactoring-targets` claimed `html` support but its dispatch returned `html_not_wired`; (d) LOW — `dominant_type` reported a biomarker at intensity 0 (should be "none"); (e) LOW — the module doc overstated per-language-percentile uniformity (shotgun excepted); plus several weak/tautological tests and untested contracts (weights, band thresholds, code-health CSV columns).
*   **Fix**: (a) explain tuple updated to the current formula/weights/thresholds; (b) `code_health_min` rewired to the composite via a new `evaluate_code_health_gate(&[CodeHealthRow])` (breaking change, noted in CHANGELOG); (c) `html` wired via the generic `write_html`; (d) `WHERE intensity > 0` added to the dominant-type query; (e) module doc scoped. Tests: reframed the architecturally-invalid `structural_risk_rewards_multiple_cooccurring_smells`, hardened the silent-skip in `code_health_penalizes_churn`, removed the tautological `god_class_and_dry_are_biomarkers`, fixed the `min_by_key(loc)` tie in the ManualUp test, and added `code_health_csv_column_contract` + `code_health_band_matches_thresholds`. Residual (deferred): exact-weight assertion (needs exposing `n_cn`/`n_au`), god-class biomarker firing (needs a fan-in fixture), and churn/ownership global-vs-per-language normalization (a Phase-2 tuning consideration).

### 4.5 SPA linked-brushing — publish-side + subscriber deep validation (2026-07-03)

A post-slice deep validation of Plan 3b (four selection subscribers) ran three parallel source-level validators (subscribe correctness / publish completeness / test integrity) plus a validation-spec pass. The subscribe side was confirmed fully correct; the findings below are on the publish side and two subscriber edge cases. F239–F241 were fixed together in `0a41b1d`; F238 is the one that needs a design decision (tracked for Plan 3c).

#### F238 — parallel-coords cross-widget highlight is visually inert (`emphasis.disabled`)

*   **Location**: `output/spa/widgets.js` — parallel series config (`emphasis: { disabled: true }`) + the `parallel-coords` selection subscriber
*   **Severity**: LOW · **Category**: UX / linked-brushing fidelity · **Status**: Fixed (Unreleased)
*   **Description**: The pre-existing `parallel-coords` subscriber calls ECharts `highlight`/`downplay` on selection, but the parallel series sets `emphasis: { disabled: true }`, so those actions have NO visible effect — a file selected in another widget does not visibly stand out in the parallel-coordinates plot. The subscriber is wired but inert.
*   **Fix (`dd9cfad`)**: investigation confirmed `emphasis: { disabled: true }` is a LOAD-BEARING ECharts-6 regression workaround (the hovered polyline disappears under emphasis) — so re-enabling emphasis was NOT an option. Instead the subscriber now restyles the selected line's per-item `lineStyle` directly (bold `--color-info`, width 3, opacity 1; the rest fade to opacity 0.12; a null selection restores the default `--color-warning`), extracting the series data into `parallelData` once so it can mutate + re-apply without a rebuild. No emphasis transform, so the regression stays avoided. Setting every item each fire means no stale A→B highlight.

#### F239 — `trends` subscriber leaves a stale A→B highlight

*   **Location**: `output/spa/widgets.js` — `trends` selection subscriber
*   **Severity**: MED · **Category**: correctness / linked-brushing · **Status**: Fixed (Unreleased)
*   **Description**: ECharts `dispatchAction({type:'highlight'})` is additive — it does not implicitly clear a prior highlight. The `trends` subscriber highlighted the selected series without downplaying first, and because the file-detail drawer is NON-modal (`.show()`, not `.showModal()`), a user can switch selection directly from file A to file B with no intervening null-clear. Result: both A's and B's trend lines stayed bold+un-blurred — a stale highlight of the previous file. (The `coupling`/`dsm` subscribers added in the slice already downplayed-first; `trends` and `parallel-coords` did not.)
*   **Fix (`0a41b1d`)**: `trends` now `downplay`s unconditionally first, then early-returns on null, then highlights only on a match — matching the coupling/dsm shape. (`parallel-coords` was validated benign here — its `emphasis.disabled` means no highlight state persists; see F238.)

#### F240 — linked-brushing publish side was asymmetric (receive-only widgets)

*   **Location**: `output/spa/widgets.js` — click handlers for the circle-pack map, coupling sankey, treemap, X-Ray sunburst
*   **Severity**: MED · **Category**: feature completeness · **Status**: Fixed (Unreleased)
*   **Description**: "Select a file in ANY widget → highlight everywhere" needs both a publish and a subscribe half. Plan 3b completed the subscribe half but the publish half was asymmetric: only four surfaces (hotspot table, parallel-coords, KI-table, keyboard treeview) routed clicks through `_codeloreShowDetail` (which publishes `selection.set`); the map canvas, sankey, treemap, and X-Ray sunburst called `showFileDetailDrawer` DIRECTLY — opening the drawer without broadcasting. So the coupling sankey, DSM, trends, and the map canvas were effectively receive-only.
*   **Fix (`0a41b1d`)**: the four direct-drawer file-clicks now route through the guarded `_codeloreShowDetail` idiom (sankey gated to files mode — a module-depth node name is a prefix, not a file, and must not go on the file-level bus). The map click drops its redundant direct `selectedCouplingFile`/`updateCouplingArcs()` in the broadcast branch (the `hotspot-map` subscriber does it on fan-out). Validated NOT-VALID and intentionally excluded: the DSM as a publish source — its axes/cells are module-level, so a click cannot identify a single file for the file-level bus. Follow-on (`27d666b`): the map's canvas background-click now clears the shared selection (bus) instead of only its local arcs, so deselection is symmetric with the new publish-on-select.

#### F241 — coupling-sankey subscriber highlight dead in module-depth view

*   **Location**: `output/spa/widgets.js` — `coupling` selection subscriber
*   **Severity**: LOW · **Category**: linked-brushing fidelity · **Status**: Fixed (Unreleased)
*   **Description**: The `coupling` subscriber highlighted the sankey node by the raw full path (`name: selectedPath`). In files mode (default) node names ARE full paths so it matched, but in module-depth view nodes are named by truncated `modulePathSeg` prefixes, so the highlight silently no-op'd — the sankey did not participate in cross-widget selection at non-file depths.
*   **Fix (`0a41b1d`)**: the subscriber now maps the bus path into the current node-name space (`modulePathSeg(selectedPath, userSankeyDepth)` when depth is numeric, else the full path), mirroring the DSM subscriber's module-mapping. Also recorded as a validated non-issue: a proposed DSM "empty-indices" guard is dead code — the per-index diagonal guide cells guarantee the scan always yields ≥1 index.

#### F242 — module-depth coupling-subscriber browser test is inert against the differential fixture

*   **Location**: `crates/codelore-lib/tests/spa_browser_test.rs` — Step 13 in `rendered_spa_boots_without_console_errors`
*   **Severity**: LOW · **Category**: test coverage · **Status**: Fixed (Unreleased)
*   **Description**: The original Step 13 (inside `rendered_spa_boots_without_console_errors`) asserted the coupling subscriber highlights a selected file's `modulePathSeg(path, 2)` module prefix in module-depth sankey view. The production mapping is correct (verified by source review), but that test's only fixture — `differential_repo::build()` — has near-zero co-changes, so at depth 2 the change-coupling sankey had no cross-module links and no qualifying node; the step always SKIPPED. Net: the guard executed no assertion in CI and provided zero live regression protection, and it spun a ~3s re-render poll to no effect on every run.
*   **Fix (`373747e`, `9030159`)**: added a dedicated `coupling_repo` fixture (`test_support/mod.rs`) — three 2-segment modules (`src/alpha`, `src/beta`, `src/gamma`), with `alpha/svc.rs`↔`beta/svc.rs` co-changed across 6 commits so a `src/alpha`↔`src/beta` depth-2 edge is guaranteed under any coupling threshold, plus per-file solo churn for hotspot rows — and moved the assertion into its own test `sankey_module_depth_highlights_mapped_node` rendered from that fixture with `permissive_coupling_opts`. The inert Step 13 was removed from the smoke test. The new test FAILS (not skips) if the depth-2 sankey has no qualifying node, and asserts the captured highlight name equals the module prefix (not the raw path). Independently verified: `spa_browser_test` 9/9 (was 8), the new test exercises its assert (full-boot run, ~9.3s), `spa_integration_test` 4/4.

### 4.6 Whole-codebase architecture review + hygiene pass (2026-07-04)

A five-dimension architecture review (four parallel read-only analysts: architecture/boundaries, coding-structure/patterns, performance, SPA; plus two validation-and-spec passes) surveyed the engine + SPA for improvement leverage. Headline: the codebase is genuinely well-built — error handling exemplary (3 non-test `unwrap`s, all justified), the `Repo` two-backend abstraction + `!Send` ingest pipeline + cross-crate boundaries clean, CSV-injection closed, test quality high. The real structural debt concentrates in one place: the analysis→output **dispatch fan-out** (43 stringly-typed dispatchers × per-format arms + 43+43 tabular emitter fns). The low-risk validated wins were fixed this pass; the large refactors are logged own-slice below.

#### F243 — `html` output support un-advertised in 4 dispatchers (stringly-typed drift)

*   **Location**: `codelore-cli/src/main.rs` — `dispatch_authors`/`dispatch_top_committers`/`dispatch_knowledge_islands`/`dispatch_clone_coupling`
*   **Severity**: LOW · **Category**: correctness / user-facing message · **Status**: Fixed (Unreleased)
*   **Description**: Each of these 4 dispatchers has a working `"html" => write_html(...)` arm, but its `unsupported_format(...)` error message advertised a format list OMITTING `html` (e.g. `"csv|json|markdown"`), so a user passing an invalid `--format` was told html isn't supported when it is. Same class as F237(c). This is a live symptom of the stringly-typed dispatch (the advertised list is a hand-maintained string parallel to the actual `match` arms — F215).
*   **Fix (`acd9568`)**: added `html` (and `sarif` where applicable) to the 4 advertised strings. Byte-identical for every success path — only the error text for an unsupported format changed. This is a symptom patch; the root cause (parallel hand-maintained format lists) is F215, logged own-slice below.

#### F231 — `Plan N` phase markers in code comments — now Fixed via a self-enforcing guard

*   **Status**: **Fixed (Unreleased)** (`52c427c`) — previously Active/deferred.
*   **Fix**: rather than the deferred one-off scripted sweep, the existing `comment_hygiene_test.rs` was extended with a `no_plan_phase_markers_in_code_comments` test (a `Plan`+digit whole-comment scan, sibling to the `F<NN>` guard), then all **69** existing markers were scrubbed — the majority stripped (parenthetical/provenance tags), ~18 stale future-tense/current-claim comments rewritten to present state (verified against source: single-producer ingest, Type 1/2 clones, all output formats shipped, `gix` default + `GitCliRepo` oracle). Test + fixes landed atomically; the guard makes the convention self-enforcing forever (strictly better than a sweep that can silently regress).

#### Other validated DO-NOW improvements landed this pass

*   **Clippy `#[allow]` justification** (`356efc9`) — 19 previously-unjustified `#[allow(clippy::…)]` sites gained a true per-site technical reason (Golden Rule #14). The tempting "consolidate casts to a workspace allow" was validated and REJECTED — it would disable the lint repo-wide.
*   **SPA listener-bus unification** (`1d645af`) — the two byte-identical registries (`selection` + `brush`) collapsed to a `makeListenerBus(arrayName)` factory (behavior-preserving; browser tests green).
*   **SPA browser-test coverage** (`e4ec986`) — the main browser test's dashboard fixture was broadened so previously-dark widget render branches (arch-trend, fusion overlays, MI tile, ownership/clones colour maps) now execute under the no-console-error / exception gate. No latent render bug surfaced.

#### Logged own-slice (validated REAL, each warrants its own byte-identical/benchmark-gated slice)

*   **F244 — Central analysis registry / `enum Format` + `TabularRow` (root cause behind the dispatch fan-out).** Refines/absorbs **F215** (stringly-typed dispatch), **F148** (no shared tabular-row trait; 88 csv/markdown `write_*` fns), **F119** (hand-rolled CSV). Validated true scope: 43 dispatchers + ~137 match arms; a clap `ValueEnum` `Format` is the minimal first increment (deletes the hand-maintained master-list `match`, adds parse-time validation + completions) — but carries an exit-2-vs-exit-4 contract delta for invalid `--format` and does NOT alone fix the per-dispatcher advertised-string drift (needs a `supported_formats()` source of truth). `main.rs` (3283 LOC) splits into `commands/`+`dispatch/` after the enum lands. L, byte-identical-gated. Sequence AHEAD of the output-emitter cluster.
*   **F245 — SPA `widgets.js` build-time module split.** 4588-line single IIFE; split into topical source `.js` fragments concatenated into the `{{WIDGETS_JS}}` placeholder in `spa.rs` (fits the offline-single-file constraint — no build step, one output file). **Fixed (Unreleased)**: implemented as a compile-time `concat!` of seven ordered `spa/js/*.js` modules (byte-identical assembly proven via `cmp`; full browser suite green), files named to match contents.
*   **F246 — SPA canvas-chart keyboard operability.** The arch-graph/DSM/module-chord/X-Ray/treemap have `role="img"`+`aria-label` but no keyboard-navigable data equivalent (only the circle-pack has its `role="tree"`). Largest remaining a11y gap; scope to the 1-2 highest-value widgets. M.
*   **Still tracked:** F206/F173 (HEAD-scan I/O — the one big perf lever, benchmark-gated), F218 residual (per-widget layout-change routing — the theme-path split landed; see the finding).

#### Validated and REJECTED (do NOT act)

*   **Domain newtypes (Golden Rule #15)** — low Rust ROI: values are string-shaped from DuckDB straight into emitters with no cross-type mixup risk; the one primitive-confusion bug (path vs module-prefix, F240/F241) was JS-side. Accepted deviation.
*   **`Options` builder** — already rejected in the roadmap; its cross-field-validation value is already delivered by `Options::validate()`.

---

## 5. Next Audit Cycle

**State after the 2026-07-01 + 2026-07-02 implementation sessions (all merged to `main`):**

- **Implemented + merged (28)**: 2026-07-01 (25) — F191, F201, F203, F204, F205, F207–F214, F216, F217, F219–F228 (PR #71); 2026-07-02 (3) — **F200** (deleted the divergent+vacuous `commit_metadata` + `CommitMetadata`, kept the real `changed_files`/`diff_hunks` cross-checks; #71), **F229** (dropped the vendored `libduckdb-sys` fork; `duckdb → =1.10504.0`; #71), **F230** (`gix 0.84 → 0.85`; #74). Full CI matrix green on every merge, including `test (windows-latest)`.
- **Refuted on validation (2026-07-02)**: F188 (ruleset omission is intentional + drift-guarded), F202 (fan-out divergence is mostly by design — god-classes externals vs internal coupling).
- **Deferred — large refactor / focused pass**: F206 (HEAD-scan I/O restructure — wants a benchmark), F215 (`enum Format`), F218 residual (per-widget layout routing; the theme/layout effect split is Partially Fixed above), F231 (62-site `Plan N` scripted sweep).
- **Carried-forward Active (output/blob cluster)**: F119 (csv-crate), F148 (`TabularEmit` dedup), F161 (`EmitterStream`), F173 (HEAD blob dedup) — byte-identical-critical (F206 is the deeper lever for F173).
- **Carried-forward Partial / design**: F177 (schema sentinels), F186 (bench PR gate — design), F197 (dogfood advisory/separate-cache).
- **Fixed (Unreleased) 2026-07-03**: F232 (coupling double-count — `n_cp` removed, score reweighted), F233 (`code_health_biomarkers_v1` cross-analysis contract — documented at the DDL), F234 (`loc` display floor — reports true value), **F235 (`structural_risk` saturation — rank files not functions + weighted sum; 67/69-at-1.0 → 8/32/31 band split)**, **F236 (biomarker normalization unified on full-universe per-file percentile; `biomarker_repo` fixture + distribution/vocabulary/dominant-type tests)**, **F237 (deep-validation audit: stale explain, check-gate composite rewiring, refactoring-targets html, dominant-type intensity>0, test hardening)**, **F239 (trends A→B stale highlight — downplay-first)**, **F240 (SPA linked-brushing publish symmetry — map/sankey/treemap/X-Ray now broadcast)**, **F241 (coupling-sankey highlight fires in module-depth view)** (all `0a41b1d`), **F238 (parallel-coords highlight made visible via a direct per-item `lineStyle` restyle — `dd9cfad`)**.

**Highest-leverage work remaining:**
1. **HEAD-scan I/O** (F206 + F173) — resolve HEAD→tree once per pass with a per-worker cached repo; benchmark via `ingest_capacity_sweep`. Biggest large-repo wall-clock lever.
2. **Output-emitter cluster** (F119 / F148 / F161) — csv-crate migration (preserve the F170 injection guard + `\n` endings), `TabularEmit` dedup, `EmitterStream` streaming, in one coordinated byte-identical pass.
3. **`Plan N` marker sweep** (F231) — 62-site scripted comment cleanup (a hard-rule violation).

**F242 (module-depth coupling-subscriber browser test made live — new `coupling_repo` fixture + dedicated test; `373747e`, `9030159`)** closed the last SPA linked-brushing follow-up.

**2026-07-04 architecture-review pass**: **F243** (html un-advertised in 4 dispatchers — Fixed `acd9568`) and **F231** (Plan-N markers — Fixed via self-enforcing hygiene guard `52c427c`) closed; clippy-allow justification + SPA listener-bus + browser-fixture coverage landed. New own-slice: **F244** (analysis registry / `enum Format` + `TabularRow`, absorbs F215/F148/F119), **F245** (widgets.js module split), **F246** (canvas keyboard a11y).

The 2026-08-02 discovery pass logged **F249–F268** (see §6). The next sweep should re-open with F-IDs starting at **F269**.

**F248 (Active) — no integration coverage that `health-trend`'s `arch_health` falls as the import graph decays.** The `health-trend` analysis (`analyses/health_trend.rs`) computes `arch_health` from per-rev `GraphMetrics`, and the unit tests cover the pure function (empty/acyclic/fully-tangled/clamp). But the integration test (`tests/health_trend_test.rs`) only asserts shape/ranges/oldest-first, not the spec's "degrading architecture ⇒ `arch_health` decreasing" case — because the only ≥2-commit fixture, `biomarker_repo`, is six independent Rust files with no inter-file imports, so its import graph is empty and `arch_health` is pinned at 100 across every sample. Fix: add an import-structured fixture whose later commits introduce a dependency cycle (mirror `architecture_trend`'s `trend_captures_cycle_introduction_over_time`), then assert the newest sample's `arch_health` is below an earlier sample's. Surfaced by the whole-branch review of the Repo Health Timeline (piece 2).

**F247 (Active) — `run_coupling_scoped` cutoff ignores lineage/time-bucket source in `good_commits`.** The rev-parameterizable `code_health` history cutoff (`HealthScanCtx::history_cutoff`) routes coupling through `run_coupling_scoped(db, opts, "changes_at_ts")`, which overrides only the pair-source + Fisher-denominator tables. The internal `good_commits_cte(bucket, use_lineage)` still reads the opt-derived `changes_lineage`/`changes`. For the primary path (no lineage, no time-bucket) this is equivalent — the cutoff-window revset equals full-history ∩ window. But `history_cutoff` combined with `--use-canonical-lineage` yields coupling pairs keyed on pre-rename path names, and combined with `--time-bucket` aggregates buckets over full history. The **same class** applies to code-health's own churn / revs / author-fragmentation CTEs: under a cutoff `{src}` becomes the raw, non-lineage `changes_at_ts` view, so those terms also lose rename-awareness when a cutoff is combined with `--use-canonical-lineage`. Neither combination is exercised (the timeline consumer uses the primary path — cutoff without lineage/bucket) nor required by the spec; documented in the `run_coupling_scoped` and `CHANGES_AT_TS_DDL` doc comments. Fix if a future consumer needs cutoff + lineage/bucket: build `changes_at_ts` from the lineage-rewritten source and thread `changes_source` into `good_commits_cte`. Surfaced by the Task-4 review + the whole-branch review of the rev-parameterizable code-health branch.

---

## 6. Discovery pass — 2026-08-02 (F249–F268)

A six-dimension read-only research fan-out (robustness / rigor / performance / feature-deepening /
error-handling / testing), each grounded in source and adjudicated against the tracked baseline;
the controller then verified the load-bearing items directly. Full narrative — location, failure
scenario, proposed direction, value/effort, verification status, invariant touches — in
[`2026-08-02-discovery-pass-f249-f267.md`](./2026-08-02-discovery-pass-f249-f267.md). Index:

| F-ID | Subject | Sev | Status |
|---|---|---|---|
| F249 | `ensure_ingest_witnessed` guards only 2 of ~13 ingest entry points — `analyze`, `gate`, `gate_changes`, `explain`, 8 MCP tools render confident empty reports over a blind (fetch-depth:1 / all-excluded) ingest. ✅ grep-verified; convergent (5 signals). Gotcha: `analyze`'s `--after/--before` also empties `commits` → message must branch. | HIGH | Active |
| F250 | `codelore explain delivery-friction` 404s on a shipped, fully-documented metric. ✅ verified | LOW | Active |
| F251 | `coordination-needs` / `knowledge-islands` classify `high` tier / 100% ownership off n=2–5 with no denominator field (unlike `bus_factor`/`ownership`). ✅ verified | MED | Active |
| F252 | `write_github_output` silently swallows the open+write `Err` (`let _ =`). ✅ verified | LOW | Active |
| F253 | HEAD-scan blob I/O Phase-1 (refines F173/F206): blocker smaller than tracked (blob-read handling already identical; divergence is downstream AST-parse). One warm-ODB reader per rayon worker via the existing `map_init` idiom; also fixes `architecture-trend`/`cycle-origins` (never cached, re-paid per `analyze`). | HIGH | Active |
| F254 | Cache-hit path runs a full O(tracked-files) `is_worktree_dirty()` walk on every invocation, just to maybe warn — defeats the cache on the agent-loop/CI hot path. ✅ verified | MED | Active |
| F255 | `panic = "abort"` × long-lived `codelore mcp`: one panicking `spawn_blocking` tool call SIGABRTs the server for every client. ✅ verified (profile scope). Add an MCP-only `catch_unwind` boundary. | HIGH | Active |
| F256 | Small per-language cohorts collapse biomarker intensities to near-binary → false `structural_risk` red-bands; disclose cohort `n` (refines F236 residual — verify the "corpus lens addresses this" claim first). | MED | Active |
| F257 | Repo-wide function-level hotspots via `entities × hunks × commits` (no tree-sitter reparse — columns ✅ verified present). New capability. | HIGH | Active |
| F258 | `first_party_import_share` wildcard misclassification (`use crate::foo::*` tagged Wildcard→excluded) + a `wildcard_import_share` row. ✅ verified (`classify` branch order). | MED-HIGH | Active |
| F259 | Dead `commits.committer_email` (all refs are test `INSERT`s ✅) → a `landed_by_other_pct` gatekeeper metric; must ship the no-`committer_name`-mailmap caveat. | MED | Active |
| F260 | `hotspot-velocity` combined-window floor lets a single-window burst out-rank steadier activity; `RECENT/BASELINE_DAYS` + `EA_Z_FLOOR` uncited & unoverridable (bypass the `constants.rs` convention). | MED | Active |
| F261 | Dead `changes.similarity` (rename %) → an `avg_rename_similarity` / low-similarity-rename signal. | LOW | Active |
| F262 | Survival analysis on hotspots (Kaplan-Meier over hot-episodes) — re-scope of the roadmap Tier-1 item; no new ingest, but needs a design pass (stateful episode extraction + KM). | — | Active (design) |
| F263 | `[new_code]` gate `run_new_code_scope` has zero test coverage (only the pure evaluator is tested). Pairs with F249. | HIGH | Active |
| F264 | `is_shallow()` has zero tests — the primitive behind cycles 2/3/4's top finding. Pairs with F249. | HIGH | Active |
| F265 | `calibrate` total-failure (0-of-N) exit path untested (only partial-failure is) — would red-flag cycle-2's G2 bug. | MED | Active |
| F266 | Differential harness missing binary / non-ASCII / submodule probes (fixture documents its own boundary). Touches the two-backend-parity invariant. | HIGH | Active |
| F267 | MCP `hotspots` never invoked via `tools/call`; `entity-effort`/`entity-ownership` have zero behavioral coverage. | MED | Active |
| F268 | CI `Build test binaries` link exhausts runner disk (SIGBUS in `ld`, different binary each run) linking 100+ fat test binaries. | MED | **Fixed (#196)** — Linux-only disk-reclaim step before checkout |

**Correction logged (not a finding):** Type-3 near-miss clones is *not* a latent-data quick win —
`clones/fingerprint.rs` stores a single SHA-256 digest (zero similarity signal); MinHash+LSH needs a
shingled representation = new ingest. Re-scope the roadmap's "~100 LOC" estimate before scheduling.

**Pointer:** `calibrate_defects` temporal train/validation positive-leakage (fully diagnosed in
`2026-07-28-hardening-cycle-3.md` §A2-1, reconfirmed HIGH cycle-4) remains open — a rigor defect
inside the calibration-honesty machinery; don't drop it when triaging.

The next sweep re-opens at **F269**.
