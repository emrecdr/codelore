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
| F94 | `ingest.rs` monolithic | **Fixed** | This session. Split the 1523-LOC `facts/ingest.rs` into a directory module `facts/ingest/` with topical submodules: `mod.rs` (entry-point `FactsDb::ingest` + channel-capacity controls + `IngestStats` + shared `current_head_rev`/`query_live_paths` helpers + `format_panic_payload`), `complexity_head.rs`, `clones_head.rs`, `imports_head.rs` (the three rayon-then-serial-drain HEAD passes + import resolution), `consumer.rs` (the connection-owning `ingest_loop` pump + `append_*` row writers), `lineage.rs` (path-lineage CTEs), `grouping.rs` (`apply_grouping` + bucketed/grouped materialisation). Pure code movement, zero behavior change: identical 26-function inventory, normalized content diff shows only `pub(super)` visibility widening (so the parent can call methods relocated into child submodules) + path-qualifier adjustments forced by the depth change. External path contracts preserved via `pub use` re-exports in `mod.rs` (`materialize_changes_lineage`/`materialize_changes_bucketed`/`materialize_path_lineage`/`apply_grouping`) and by keeping `IngestStats`/`set_channel_capacity_override`/`format_panic_payload` defined in `mod.rs`. Gate: fmt + clippy `-D warnings` clean, 663 tests pass / 0 fail (same count as pre-split). |
| F95 | `communication.rs` window filter | **Refuted** (filter at ingest level) |
| F96 | ECharts mount + dispose duplicated | **Fixed** | `7f36a7f` (`mountEcharts` @ 13+ sites) |
| F97 | SPA boot-time render storm blocks first paint | **Fixed (this session)** — see §3 closure log |
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
| F110 | Differential test only 4 of 8 trait methods | **Fixed** | PR #57 → main. `head_sha_matches` + 11 sibling tests now in `differential_repo_test.rs` |
| F111 | `FactsDb::conn()` leaks `&duckdb::Connection` into public API | **Fixed** | This session. Tightened `conn()` to `pub(crate)` so external consumers can't bypass FactsDb's safety surface; added narrow safe methods `prepare` / `execute_batch` / `query_row` (each wrapping the SQL error in `CodeLoreError::Analysis`, exit 4). All 9 external `.conn()` call sites across 5 test files migrated to the safe methods — 7 chained `db.conn().<m>()` calls rewritten mechanically, plus the two multi-query `let conn = db.conn();` bindings in `imports_factsdb_test.rs` expanded to direct `db.query_row(...)` calls. Zero CLI `.conn()` uses, so production callers see no API change — the finding was API hygiene, not breakage. |
| F112 | Provenance manifest missing reproducibility fields | **Fixed** | PR #57 → main. `head_sha`, `cache_key_hash`, `rust_version`, `target_triple`, `grammars: BTreeMap<String,String>` populated in `provenance/mod.rs` |
| F113 | `codelore-cli` reaches into many `codelore_lib` submodules — no façade | **Fixed** | This session. Added `codelore_lib::cli_api` — the single surface the CLI imports through. It re-exports the modules (`analyses`, `analysis`, `cache`, `constants`, `facts`, `options`, `output`, `provenance`, `quality_gates`, `repo`) and root types (`AnalysisName`, `CodeLoreError`, `Options`, `Result`) the CLI needs. Every CLI file migrated so `grep -rn 'codelore_lib::' crates/codelore-cli/src \| grep -v cli_api` returns 0. Internal modules stay `pub` (the integration-test crate needs deep white-box access — forcing 55 test files through a CLI-shaped façade would either bloat it or need a second `test_api`), so this is an ADDITIVE, non-breaking façade: the CLI↔library contract now lives in one file even though the compiler doesn't seal the internals. Shipped together with F145. |
| F118 | gix walker thread panic silently swallowed | **Fixed** | PR #62 → main. `WalkerStream` joins handle on EOF; panic mapped to `CodeLoreError::Repo` → exit 3 |
| F127 | Kamei `enrich_diffusion` NS/ND/NF correlated subqueries | **Fixed** (partial — entropy block remains) | PR #64 → main collapsed the NS/ND/NF triple. See F127 in §4 for the entropy-block remainder. |
| F128 | Kamei `enrich_size` correlated subqueries | **Fixed** | PR #64 → main. Grouped `UPDATE … FROM (… GROUP BY rev)` |
| F134 | Hotspot 'Show all' synchronously builds full HTML | **Fixed** | This session. `renderNextPage` now async + chunked in 50-row batches with `await yieldToMain()` between each; `Show all` wrapped in element-scoped `startViewTransition(.., container)`; `.hotspot-row { view-transition-name: match-element }` for per-row crossfades |
| F135 | Theme toggle re-runs full d3.pack layout | **Fixed** | This session. `Alpine.effect` rerenderer loop now yields between widgets via `window._codeloreYieldToMain()`; `.widget { view-transition-name: match-element }` so widgets animate independently |
| F138 | `startViewTransition` ignores `prefers-reduced-motion` | **Fixed** | PR #62 → main. `matchMedia('(prefers-reduced-motion: reduce)')` short-circuits the transition |
| F139 | `DiffGates` parsed but never evaluated | **Fixed** | `549c460` (evaluator + CLI wiring) |
| F140 | Six new analyses lack integration tests | **Fixed** | `7b43593` (5 new `tests/*_test.rs`) |
| F141 | `imports_factsdb_test` only asserts unresolved | **Fixed** | `7b43593` (`ingest_resolves_imports_to_target_paths`) |
| F143 | SPA headless-browser smoke test | **Fixed** | PR #56 → main. `tests/spa_browser_test.rs` + `browser-tests` feature + CI job |
| F127 (full) | Kamei `enrich_diffusion` entropy block correlated subquery | **Fixed** | This session. Entropy block rewritten as 2-pass (reset to 0.0 + grouped `UPDATE ... FROM` with window-function `p_i = loc_added / SUM(loc_added) OVER (PARTITION BY rev)`), mirroring `enrich_history`'s shape. Byte-identical semantics proven via `kamei_entropy_per_commit_distribution` regression test (3 hand-computed cases: single-file = 0.0, even 2-way = 1.0, uneven 3-way = 1.2987949...). |
| F117 | First-party GHA actions use floating tags despite credential permissions | **Fixed** | This session. 5 credential-handling actions SHA-pinned via `@<commit-sha> # vN` convention (matches existing `softprops/action-gh-release` pin): `actions/attest-build-provenance` (issues OIDC token for SLSA provenance), `docker/login-action` (consumes `GITHUB_TOKEN` for ghcr.io auth), `docker/build-push-action`, `docker/metadata-action`, `docker/setup-buildx-action`. 8 use-sites across `container.yml` + `release.yml`. Non-credential actions (`actions/checkout`, `actions/cache`, etc) intentionally left as `@vN` per finding's "credential-handling subset" framing — pinning them too would balloon Dependabot bump-PR surface without commensurate attack-surface reduction. |
| F129 | `arch_violations` materialises full imports Vec then truncates post-Rust | **Fixed** | This session. Removed the intermediate `Vec<(String, String, String)>` collect; the rows iterator is walked directly and early-breaks when `opts.rows_limit` is hit. SQL's `ORDER BY src_path ASC, target_path ASC` makes the early-break deterministic — first N violations are the same N the prior collect-then-truncate produced. On a monorepo with millions of imports + `--rows 50`, validation stops after 50 hits instead of validating every row. Smaller scope than the finding's "push validation into SQL" suggestion (SQL would need per-prefix LIKE join with planner-risk) but same observable win without the architectural change. |
| F130 | `pair_programming` O(P²) with `String::clone` per inner-loop probe | **Fixed** | This session. Refactored `HashMap<(String, String), u32>` → `HashMap<(u32, u32), u32>` with per-run author interner (`HashMap<String, u32>` + `Vec<String>` table). Inner loop now hashes pure integer-pair keys — zero `String` allocation per probe. The per-commit `participants` `Vec<String>` is replaced by a reusable `Vec<u32>` scratch buffer with `clear()` between iterations (preserves the allocation). On repos with heavy pair-programming (~100 commits per pair), the prior shape allocated ~200 redundant `String`s per pair just to discover the pair was already counted; the new shape allocates each author once at first encounter, period. New regression test `pair_programming_dedupes_pair_regardless_of_primary_orientation` guards the canonical-ordering invariant (alice↔bob encountered in two orientations must dedup to one row with `author_a` lex-less than `author_b`). |
| F153 | Generic I/O errors from `--team-map` config-file read exit code 5 instead of 3 | **Fixed** | This session. Added `CodeLoreError::RepoIo(std::io::Error)` variant mapped to exit 3 in the `exit_code()` match (no `#[from]` so generic `?` propagation still defaults to write-side `Io` → exit 5). The single load-bearing call site (`identity::team_map::load` reading a user-supplied `--team-map FILE`) now constructs `RepoIo` instead of `Io`. Recon revealed only ONE site explicitly constructed `Io` for a read-side input failure; every other `Io` use is `writeln!(...).map_err(CodeLoreError::Io)` from output emitters where exit 5 is correct. Smaller scope than the audit's "repo probing" framing because `GixRepo::open` and `GitCliRepo::open` already mapped their underlying errors to `CodeLoreError::Repo(String)` (exit 3). Updated `bca_error_exit_codes_match_spec` test to cover `RepoIo` → 3. |
| F142 | Tracing instrumentation skewed across `analyses/` (3 lines total) | **Fixed** | This session. `#[tracing::instrument(name = "<analysis-name>", skip_all, fields(min_revs = opts.min_revs))]` added to all 32 `run_*` entry points across 31 files. Operators get per-analysis spans with timing + the input gate for free via `RUST_LOG`. Verified end-to-end: hotspots span emits `hotspots{min_revs=1}` with `time.busy=6.87ms time.idle=2.25µs`. |
| F146 | `json.rs` trivial `write_*_json` shims (29 total) | **Fixed** | This session. `write_json<T: Serialize>` made `pub`; 27 trivial shims deleted, 2 non-trivial kept (`write_revisions_json` tuple→struct wrap, `write_communities_json` wrapper struct emit). 33 CLI call sites updated. Net: -137 LOC. |
| F145 | `main.rs` dispatch boilerplate is the bulk of the file | **Fixed** | This session. The 2-D `match (format, &analysis)` (≈1200 LOC, the abstraction the finding said was missing) collapsed to a 1-D `match &analysis` delegating to 32 per-analysis `dispatch_<x>` fns, each running its analysis then matching `format` to the right emitter; SARIF/HTML needs (repo_root, title, generated-at) carried in a shared `EmitCtx`; the HTML pre-branch folded into the same per-analysis fns (with a shared `html_not_wired` helper). Proven semantic-preserving by a byte-identical capture across all 228 analysis×format pairs: exit codes identical (incl. the 22 pre-existing `unreachable!` panics, now logged as F165), and the only 9 stdout diffs are environmental — clones (HEAD-time tree-sitter walk over the working tree now sees the new `dispatch_*` fns as clones), delivery-friction (`wip_age_days` wall-clock drift), and SARIF (per-run `run/<id>`). main.rs grew ~+480 LOC: the per-fn structure re-states each `run_*` per format arm rather than hoisting it (hoisting would run the analysis before the `unreachable!` for unwired ndjson/gha, risking an exit-code change), the price of exact-semantics preservation. Shipped with the F113 `cli_api` façade. |
| F147 | `AnalysisName` 3-way sync no exhaustiveness guard | **Fixed** | `549c460` (initial `_exhaustive_check`) + PR #60 (`registry!` macro). F157 closed by the macro. |
| F120 | SARIF schema URL on legacy `schemastore.azurewebsites.net` host | **Fixed (URL)** | This session. `sarif.rs:13` swapped to canonical `https://json.schemastore.org/sarif-2.1.0.json`. The hand-rolled-JSON / `serde-sarif` migration concern in the original finding was a separate refactor and is NOT closed — re-surfaces in next discovery pass if still material. |
| F124 | MSRV pin has zero buffer behind toolchain | **Fixed (policy)** | This session. `docs/RELEASING.md` now carries an "MSRV (Minimum Supported Rust Version) Policy" section explaining the deliberate "MSRV tracks channel" stance for the pre-1.0 binary-distribution model + post-1.0 reconsideration trigger. The zero-buffer is now a deliberate documented decision, not an oversight. |
| F150 | Schema version tracked in two disjoint places, no startup validation | **Fixed** | PR #61 → main. `CURRENT_SCHEMA_VERSION` const + `validate_schema_version()` on `open_read_only` |
| F151 | Leiden communities non-deterministic | **Fixed** | PR #61 → main. `LEIDEN_SEED` constant threaded into `LeidenConfig` + regression test |
| F152 | `clone_group_id` non-deterministic (std HashMap iteration) | **Fixed** | PR #61 → main. `BTreeMap<[u8;32], _>` |
| F154 | `codelore diff` base==head produces empty SARIF | **Fixed** | PR #62 → main. `bail!` at diff entry with SHA + range context |
| F155 | `DiffOutput.{base,head}_median_code_health` defaults to silent 0.0 | **Fixed** | PR #60 → main. `Option<f64>` + `skip_serializing_if` |
| F156 | `Thresholds`/`Gates`/`DiffGates` don't `deny_unknown_fields` | **Fixed** | PR #60 → main. Attribute added to all three structs + 3 regression tests |
| F157 | F147's exhaustiveness guard wraps the wrong list | **Fixed** | PR #60 → main. `registry!` macro forces single source-of-truth for both array and match |
| F158 | SARIF `informationUri`/`helpUri` hardcodes wrong project URL | **Fixed** | PR #63 → main. `CODELORE_HOMEPAGE` constant + URL-guard regression test |
| F159 | SARIF `artifactLocation.uri` not percent-encoded | **Fixed** | PR #63 → main. `percent_encode_path()` helper + non-ASCII/space/# regression test |
| F160 | Kamei NDEV/EXP same-second peer semantics inconsistent | **Fixed** | PR #64 → main. Strict `prev.date < c.date` uniformly across NDEV/NUC/EXP/REXP/SEXP |
| F163 | SARIF `automationDetails.id` is static | **Fixed** | PR #63 → main. `automation_id_for(prefix)` appends per-run 16-hex suffix |
| F162 | Parquet column types drift from CSV row-type contract | **Fixed (already-closed by side-effect)** | Parquet writers now delegate to `analyses::hotspots::build_inlined_sql` / `revisions::build_inlined_sql` shared SQL generators. Those generators already use the explicit-cast convention the original finding requested, so the CSV row-type contract is preserved verbatim through to Parquet. The 51-line `parquet.rs` shim has no remaining type-inference call site. Verified 2026-06-21 validation pass. |
| F131 | Provenance tooltip triggers 14×14 px target | **Fixed** | This session. `.tooltip-trigger` in `template.html` bumped from `width/height: 14px` → `24px` to meet WCAG 2.5.5 Target Size (Minimum). Glyph stays visually moderate (`font-size: 12px` on a 24×24 button) so the trigger doesn't dominate dense table headers, but the click/tap area is reachable for coarse pointers. `line-height: 22px` keeps the `?` glyph vertically centered inside the 24px circle minus 1px borders top+bottom; `vertical-align: -7px` re-baselines the larger button against adjacent text without disturbing label rhythm. CSS anchor positioning + the `:hover/:focus-visible` reveal path are unchanged — F131 is purely about target size, not the popup. |
| F137 | Knowledge-islands rows not keyboard-activable | **Fixed** | This session. New `wireRowKbActivation(rowEl)` helper in `widgets.js` sets `tabindex="0"` + `role="button"` on each row and forwards Enter/Space to the existing click handler (preventDefault on Space so the page doesn't scroll). Called from BOTH the KI row loop (renderKnowledgeIslands) AND the hotspot table row loop (renderNextPage) — the audit only flagged KI but the hotspot table had the same gap; one helper, two call sites. `tr.hotspot-row:focus-visible, tr.ki-row:focus-visible` paints a `2px solid var(--accent)` outline so keyboard users can see which row is about to be activated. Other table-row-as-button widgets discovered via `cursor:pointer` grep — only the two were click-on-tr; the rest are widget-level handlers (sankey, sunburst, etc.) which already route through `_codeloreShowDetail`. |
| V5 | METRIC_DEFS formula strings reference parameter names verbatim | **Fixed** | This session. New `SpaOptionsSnapshot { min_revs, min_shared_revs, min_coupling_pct, max_coupling_pct, max_changeset_size, fisher_significance }` field on `SpaDashboard`, populated from `Options::from_options` at dispatch. JS-side `interpolate(formula, data.options)` substitutes `${key}` placeholders in METRIC_DEFS strings — `coupling_pairs` and `coupling_density` formulas now read `min_shared_revs ≥ 5` / `Fisher exact p < 0.05` (or whatever this run's effective thresholds are) instead of parameter names. Unknown placeholders left literal so a stale METRIC_DEFS entry shows the `${key}` token visibly during review rather than silently filling with `undefined`. `SpaOptionsSnapshot::default()` mirrors code-maat parity baseline so tests + step-summary using `..Default::default()` stay green without per-site updates. |
| V6 | `CHANNEL_CAPACITY = 64` unmeasured | **Fixed** | This session. New `ingest_capacity_sweep` Criterion benchmark on the medium fixture sweeps `[16, 64, 256, 1024]` in one `cargo bench` invocation. Mechanism: `CHANNEL_CAPACITY_OVERRIDE: AtomicUsize` static in `facts::ingest` + `pub fn set_channel_capacity_override(n)` write hook; `channel_capacity()` reads override-else-DEFAULT_CHANNEL_CAPACITY (64) on each ingest call. Avoids `unsafe { env::set_var }` (workspace `unsafe_code = "forbid"`) and avoids expanding the public CLI surface — production dispatch never touches the override; only the bench writes it, and resets to `0` (= default) at sweep end. `bounded::<CommitEvent>(channel_capacity())` reads the runtime value, so the curve is real measurement, not folklore. |
| F114 | Single-CDN dependence for all 4 SPA assets | **Fixed** | This session. `AssetPin` extended with `url_fallbacks: &'static [&'static str]`; `fetch_and_pin` walks primary URL first, then each fallback in declaration order. Every asset's fallback is the `unpkg.com` equivalent — both jsDelivr and unpkg pull from the same npm registry, so the bytes are identical and the same SHA-256 validates whichever mirror responds. SHA-256 mismatch on ANY URL is a hard fail (not "skip to next mirror") so a tampered mirror can't be silently replaced by a clean one. A jsDelivr availability incident (DNS outage, regional block, rate-limit) no longer breaks every `--features spa` build. |
| F115 | Container base images use mutable tags | **Fixed** | This session. Both `Containerfile` base images pinned to immutable `@sha256:...` digests INLINE on the `FROM` instructions (not via ARG — Dependabot/Renovate parsers don't resolve ARG substitutions). `rust:1.96-bookworm@sha256:19817ead...` for the builder; `gcr.io/distroless/cc-debian12:nonroot@sha256:b0ae8e98...` for runtime. Renovate (not Dependabot) handles digest bumps because Dependabot's docker ecosystem only detects `Dockerfile`/`*.Dockerfile` and skips `Containerfile`; `renovate.json` extended with `dockerfile.managerFilePatterns: ["/Containerfile/"]` + a `matchManagers: ["dockerfile"]` package rule grouping all container-base bumps weekly. Reproducibility + cosign/SLSA provenance attestation now work the way they're supposed to. |
| F122 | `toml = "0.8"` one major behind | **Fixed** | This session. Workspace bumped to `toml = "1"` (latest 1.1.2+spec-1.1.0). The 1.0 release split `parse` (low-level parser) from `serde` (`from_str` / `Deserialize` glue); the dep declaration now opts into BOTH features explicitly so `Thresholds::parse` and `LayerRules::parse` keep their typed `from_str` API. No call-site changes — the high-level `toml::from_str` / `toml::Table` surface is stable across the major. Cargo.lock drops `toml_datetime` 0.6 / `toml_edit` 0.22 / `winnow` 0.7 (replaced by their 1.x successors). 652-test workspace + clippy clean. |
| F136 | Color-mode tablist mismatches WAI-ARIA Tabs pattern (no `aria-selected`) | **Fixed** | This session. JS-driven hotspot color-mode handler (`initHotspotColorToggles`) now sets `aria-selected` on every tab in the toggle loop alongside the existing `tab-active`/`active` class toggles. Initial `aria-selected="true"` on the cognitive button (default active) and `aria-selected="false"` on the other six in `template.html`. The six Alpine-driven tablists (trends, module-chord, arch-graph, multi-metric, delivery-risk, change-coupling) each gained `:aria-selected="$store.layout.<key> === <value> ? 'true' : 'false'"` next to the existing `:class` binding via a one-shot Python regex pass — 30 buttons updated total (4 hand-edits for the trends tablist + 26 from the regex). Verified by `awk` count: every `:class` `tab-active` binding is now paired with a `:aria-selected` binding on the following line. SPA integration test green. Screen readers now announce the selected tab; the WAI-ARIA Tabs pattern's "tab → tabpanel → aria-selected" loop is complete. |
| F144 | No CI dogfooding of `codelore` against `codelore` | **Fixed** | This session. New `dogfood` job in `ci.yml` builds release `codelore-cli --features spa`, runs `codelore analyze --analysis hotspots --format gha --repo .` so hotspots stream into the PR's Checks panel as inline annotations (`::warning::` / `::notice::` per the existing GHA emitter's bucketing). Same step also writes a markdown summary (top hotspots / code-health worst-10 / knowledge islands) into `$GITHUB_STEP_SUMMARY` so reviewers see CodeLore's verdict inline on every PR. PR events additionally run `codelore diff origin/${{ github.base_ref }}...HEAD --format markdown` and append the delta. `continue-on-error: true` during the bake-in period so the job surfaces signal without gating merges while thresholds are still calibrating. Uses sccache + rust-cache for sub-30s incremental runs. Verified the binary's `--format gha` + `diff <range>` syntax actually work on this repo before committing the workflow. |
| F149 | `hunks` table lacks PK + NOT NULL + `(rev,path)` index | **Fixed** | This session. Recon-revealed 3-layer gap: `Hunk` parsed at walk time, `Repo::diff_hunks` stubbed in `GixRepo`, walker constructed `FileChange.hunks: vec![]`, `append_change` never wrote rows. Wired all three layers: (1) Extended `count_loc` → `count_loc_and_hunks` in `gix_repo.rs` walking `imara_diff::Diff::hunks()` from the SAME histogram diff already running for `loc_added/loc_deleted` (no extra blob read, no second pass; converts to git's 1-indexed `@@ -old_start,old_lines +new_start,new_lines @@` convention so the differential test stays trivially green). (2) `GixRepo::diff_hunks` now resolves the commit's before/after blob OIDs via new `blob_at_path` helper + calls `count_loc_and_hunks` — root-commit-safe via `Option<ObjectId>` empty-side handling. (3) `compute_changed_files` Modification arm consumes the new tuple and populates `FileChange.hunks`. (4) `append_change` writes one hunks row per `FileChange.hunks` entry alongside the changes row. (5) Schema v3 / SCHEMA_VERSION 4 / cache `schema_v5` bumps invalidate caches naturally. (6) New `ingest_writes_hunk_rows_to_hunks_table` regression test (modify two non-adjacent regions, assert ≥2 rows + zero NULL offsets). (7) Differential test asserts gix == cli hunks across README/Cargo.toml/CHANGELOG. 653-test workspace + clippy clean. ~80 LOC net (vs the audit's "M not L" estimate — recon revealed the gix-diff API already exposed `hunks()` for free). |

**Newly REFUTED (2026-06-18 / 2026-06-21)**:

| F-ID | Original claim | Why refuted |
|---|---|---|
| F116 | Renovate AND Dependabot configured for same ecosystems | The two bots are partitioned by `package-ecosystem`, not duplicated. `.github/dependabot.yml` opens with `package-ecosystem: github-actions` (only updates `.github/workflows/*.yml` action pins). `renovate.json` carries `matchManagers: ["cargo"]` rules exclusively (Rust deps). The original audit treated the presence of both config files as evidence of duplication without reading either's scope. Keep both; they're the right split — Renovate's `matchPackageNames: ["duckdb"] rangeStrategy: pin` + `tree-sitter enabled: false` rules carry the same Cargo-bump policy CLAUDE.md documents in §"Dependabot has intentional ignore rules" but for the cargo ecosystem, which Dependabot is NOT configured to touch. |
| F123 | `crossbeam = "0.8"` + `num-format = "0.4"` in codelore-rca stale | Both pins resolve to the latest published versions. `crossbeam 0.8.4` is the current release on crates.io (no 0.9 or 1.x line exists); `num-format 0.4.4` is the current release (no 0.5 line exists). The hands-off-MPL-fork policy is intact AND the pins are current — the "stale" claim was unverified at the time. Re-checked via lib.rs / crates.io advisories index 2026-06-21. |

**Refuted findings preserved**: F88 (silent ODB skip rationale), F95 (window filter at ingest level), plus from §3/§4 of the prior report — apply_grouping JOIN shape, renderHeader listener leak, parquet/SQLite backslash escape, hotspots CTE leak, color-mode aria-label, Kamei SEXP `<` vs `<=`, tree-sitter `kind_id` ABI, AI-assist false positives, NULL-conflated AI attribution, DuckDB pinning speculation, code-health weights citation, SoC inclusive thresholds. Rationale in commits `f1aa0e7` (PR #36) + `13fefcb` (PR #38).

---

## 4. Active Findings

### NEW Active Findings — Tool replacement / dep currency

#### F119 — Hand-rolled 826-line CSV emitter → use `csv` crate

*   **Location**: `crates/codelore-lib/src/output/csv.rs`
*   **Severity**: MED

### NEW Active Findings — Backend performance

### NEW Active Findings — Test / CI / observability

### NEW Active Findings — Code complexity / maintainability

#### F164 — Task-ID (`F<NN>`) references embedded in code comments codebase-wide

*   **Location**: ~48 occurrences across `crates/codelore-lib/src/` (e.g. `cache.rs:57` "F33 fix", `analysis.rs:209` "F14 + F15 fix", `constants.rs:64` "F10:", `stats.rs:6` "(F121)", `quality_gates/mod.rs:290` "F156", `output/html.rs:298` "F20 fix", plus 6 in the `facts/ingest/` submodules).
*   **Severity**: LOW
*   **Category**: Documentation hygiene
*   **Detail**: Comments anchor their rationale to finding/task IDs ("F33 fix: …") instead of stating the current contract directly. The IDs are meaningless to a reader without the report and rot as findings close. The project convention is that history lives only in `CHANGELOG.md`; code comments describe the *current* state. Surfaced while splitting `ingest.rs` (F94) — the six in-ingest occurrences were preserved verbatim to keep that split a pure move, rather than cleaned inline.
*   **Suggested fix**: a codebase-wide sweep rewording each `F<NN> fix:`/`(F<NN>)` comment to keep only the WHY, dropping the ID. Mechanical but wide; best as its own focused commit so the diff is reviewable and doesn't ride along with a behavioural change.

#### F148 — `csv.rs` + `markdown.rs` per-analysis emitters

*   **Severity**: LOW

#### F165 — `--format ndjson`/`gha` on an unsupported analysis panics (reachable `unreachable!`)

*   **Location**: `crates/codelore-cli/src/main.rs` — the per-analysis dispatch fns' `_ => unreachable!("format/analysis combination should have been validated above")` arm (was the trailing arm of the old `match (format, &analysis)`).
*   **Severity**: MED
*   **Category**: Robustness / error handling
*   **Detail**: `ndjson` and `gha` pass top-level format validation but are only wired for a few analyses (hotspots, plus ndjson for code-health/coupling/lead-time). For every other analysis, `--format ndjson` / `--format gha` falls through to the `unreachable!` and **panics (exit 101)** instead of bailing cleanly. 22 analysis×format pairs hit this (e.g. `abs-churn --format gha`, `revisions --format ndjson`). The comment claims the combination "should have been validated above" — but nothing validates per-analysis format support for ndjson/gha the way the SARIF guard (`--format sarif` → bail for unsupported analyses, exit 1) does. Surfaced during the F145 byte-identical verification (preserved verbatim there — fixing it changes exit 101→non-panic and is out of scope for a behaviour-preserving refactor).
*   **Suggested fix**: mirror the SARIF guard — a pre-dispatch check that bails with a descriptive error (exit 1) for `ndjson`/`gha` on analyses that don't support them, OR wire those formats for all analyses. Either way the `unreachable!` becomes genuinely unreachable.

### Sixth audit pass — F161, F162 (emit memory / type contract)

#### F161 — Every emitter materializes the full `Vec<Row>` — no streaming path

*   **Location**: `crates/codelore-cli/src/main.rs:735-799` (HTML) + every CSV/JSON/markdown/SARIF arm
*   **Severity**: LOW
*   **Category**: Memory architecture
*   **Status**: Active
*   **Description**: Every `run_*` collects from the DuckDB cursor into a `Vec<Row>`; emitter signature `fn write_X(rows: &[Row], w: &mut W)` iterates over the slice. Peak memory grows with row count. On a 100k-file monorepo, HotspotRow Vec (~40 MB data + double during query→Vec staging) plus CSV staging strings can hit hundreds of MB.
*   **Failure scenario**: `codelore analyze --analysis hotspots --format csv` on a 200k-touched-path monorepo: ~5-8 GB resident peak; OOM on 4 GB CI runner.
*   **Suggested fix**: `EmitterStream<W>` trait with `emit_header` / `emit_row` / `finish`. CSV is mechanical; JSON/markdown need array streaming; SARIF stays batch (needs run-level totals).

---

## 4½. Validation Pass — 2026-06-18

Every Active / Partial entry above re-verified against current `main` HEAD via direct source inspection by a fan-out of 8 parallel validation subagents. Backwards-evidence summary so the next reader doesn't redo the same checks:

| Finding | Claim | Verified state on main | Status |
|---|---|---|---|
| F94 | ingest.rs monolithic | Split into `facts/ingest/` directory module (mod + 6 topical submodules); pure code movement verified by identical function inventory + normalized diff; 663 tests pass | **Fixed (this session)** |
| F97 | `JSON.parse` synchronous at first paint | Recon clarified the bottleneck was the boot-time WIDGETS.forEach render storm, not the JSON.parse itself. Boot converted to async with `yieldToMain` between widgets. First paint is now bounded by 1 widget render (kpi-tiles) instead of all 14. | **Fixed (this session)** |
| V4 | no `WIDGETS` registry | `const WIDGETS = [{ name, render, rerender? }]` introduced at §3 boot; single `WIDGETS.forEach` loop replaces 60 LOC of duplicated render + rerender lines; 14 widgets registered uniformly; integration + browser smoke tests green | **Fixed (this session)** |
| V5 | METRIC_DEFS not interpolated | `SpaOptionsSnapshot` field on `SpaDashboard` populated from `Options::from_options`; widgets.js `interpolate(def.formula, data.options)` substitutes `${key}` placeholders; coupling_pairs/coupling_density formulas updated to use placeholders | **Fixed (this session)** |
| V6 | `CHANNEL_CAPACITY = 64` unmeasured | `ingest_capacity_sweep` Criterion benchmark added (16/64/256/1024); `CHANNEL_CAPACITY_OVERRIDE: AtomicUsize` + `set_channel_capacity_override(n)` writer hook; `bounded::<CommitEvent>(channel_capacity())` on the hot path | **Fixed (this session)** |
| F111 | `FactsDb::conn()` leaks `&Connection` | `conn()` tightened to `pub(crate)`; `prepare` / `execute_batch` / `query_row` safe methods added; all 9 external `.conn()` test call sites migrated; zero CLI uses | **Fixed (this session)** |
| F113 | CLI reaches into many lib submodules | `codelore_lib::cli_api` façade added (re-exports the modules + root types the CLI needs); all CLI files migrated so `grep codelore_lib:: \| grep -v cli_api` = 0; internals stay `pub` for the test crate | **Fixed (this session)** |
| F114 | Single-CDN dependence | `AssetPin.url_fallbacks` added with `unpkg.com` mirror per asset; `fetch_and_pin` walks primary→fallbacks; SHA-256 enforced on whichever mirror responds; tampered-mirror substitution still fails the build loudly | **Fixed (this session)** |
| F115 | Container mutable tags | Both `FROM` lines now carry inline `@sha256:` digests; Renovate `dockerfile` manager pattern set to `/Containerfile/` (Dependabot only detects `Dockerfile`); base-image bumps grouped weekly | **Fixed (this session)** |
| F116 | Dependabot + Renovate duplicate | Both files present BUT partitioned by `package-ecosystem`: Dependabot owns `github-actions`, Renovate owns `cargo`. Not duplicated. | **REFUTED** (see refuted-findings block above) |
| F117 | First-party GHA floating tags | `release.yml`: 6 `actions/...@vN` lines (52, 88, 95, 148, 153, 165, 168, 207, 266) all floating. `container.yml`: 6 `docker/...@vN` lines (59, 61, 69, 119, 121, 129). Audit cited release.yml for the docker actions — they actually live in container.yml. | Active confirmed (location refined) |
| F119 | csv.rs 826 LOC | `wc -l = 826` ✓ (no drift); `grep 'use csv' = 0` — still hand-rolled | Active confirmed |
| F120 | SARIF schema URL on legacy host | `sarif.rs:13` swapped to `https://json.schemastore.org/sarif-2.1.0.json` | **Fixed (URL half) — hand-rolled JSON / serde-sarif migration NOT closed** |
| F121 | `fishers_exact` unmaintained | Ported in-tree as `crate::stats::fisher_two_tail_pvalue` (hypergeometric tail in log space via `ln_factorial`); supply-chain dep removed; 8 regression cases match upstream to 1e-12 relative error | **Fixed (this session)** |
| F122 | toml on 0.8.x | Workspace dep bumped to `toml = "1"` with `features = ["parse", "serde"]` (the 1.0 feature split); Cargo.lock now `toml 1.1.2+spec-1.1.0`; 652-test workspace + clippy clean | **Fixed (this session)** |
| F123 | codelore-rca stale crossbeam/num-format | `Cargo.toml:40 crossbeam = "0.8"`, `:47 num-format = "0.4"`; lock resolves crossbeam v0.8.4 + num-format v0.4.4 — identical to prior. Hands-off policy on the MPL fork. | Active confirmed |
| F124 | MSRV pinned to current stable, undocumented | `docs/RELEASING.md` now carries an "MSRV (Minimum Supported Rust Version) Policy" section: documents the deliberate "MSRV tracks channel" stance for the pre-1.0 binary-distribution model + the post-1.0 reconsideration trigger | **Fixed (policy)** |
| F125 | redundant queries fire 4× per ingest | `ingest.rs:92-98` hoist `current_head_rev` + `query_live_paths` once; threaded as `&[String]` + `&str` into 4 HEAD-time passes | **Fixed on main (PR #58)** |
| F126 | N single-row UPDATEs in resolve_imports | `ingest.rs:599-635` bulk Appender into `_resolved_imports` + single hash-joined UPDATE | **Fixed on main (PR #58)** |
| F127 | Kamei `enrich_diffusion` correlated | NS/ND/NF collapsed (`kamei/mod.rs:44-65`). Entropy block (`:72-83`) **still correlated** — known follow-up per validation observation 30251. | Partial (entropy block remains — see updated §4 description) |
| F128 | Kamei `enrich_size` correlated | `kamei/mod.rs:104-114` — single grouped `UPDATE … FROM (SELECT rev, SUM ... GROUP BY rev)` | **Fixed on main (PR #64)** |
| F129 | arch-violations materializes, truncates post-Rust | `arch_violations.rs:55-75` collects full Vec without LIMIT, validates in Rust at `:77-88`, `truncate(limit)` post-loop at `:90-93` | Active confirmed |
| F130 | pair_programming O(P²) with `String::clone` | `pair_programming.rs:102-107` literal doubly-nested loop with `participants[i].clone(), participants[j].clone()` | Active confirmed |
| F131 | Tooltip 14×14 trigger | `template.html:325-328` bumped to 24×24 with 12px glyph + 22px line-height + -7px vertical-align | **Fixed (this session)** |
| F132 | Hardcoded hex in widgets.js | All 4 sites swapped to `token('--name')` reads (label-on-dark, label-on-saturated, heatmap-1..5, chart-palette-1..15); light-theme overrides added; widget entries upgraded to `rerender: 'theme'`; zero hex literals remain in widgets.js | **Fixed (this session)** |
| F133 | No responsive < 900px | Dashboard grid swapped from `xl:grid-cols-2` to `md:grid-cols-2`; Tailwind bundle rebuilt with `md\:grid-cols-2`; 2-col kicks in at ≥768px (tablet portrait); SPA + browser tests green | **Fixed (this session)** |
| F134 | Hotspot 'Show all' synchronous | `widgets.js` now chunks `renderNextPage` into 50-row batches with `await yieldToMain()` between each; the `Show all` click is also wrapped in element-scoped `startViewTransition(..., container)` (Chrome 147+) so the table animates without freezing other widgets; `view-transition-name: match-element` on `.hotspot-row` gives per-row crossfades | **Fixed on main (this session)** |
| F135 | Theme toggle re-runs `d3.pack` | `template.html`'s `Alpine.effect` rerenderer loop now `await window._codeloreYieldToMain()` between each registered rerenderer (the d3.pack pass still runs but yields the main thread between widgets); `.widget { view-transition-name: match-element }` per-widget animation | **Fixed on main (this session)** |
| F136 | Color-mode tablist non-ARIA | JS-driven hotspot toggle sets `aria-selected` in the toggle loop; 6 Alpine tablists carry `:aria-selected` next to `:class`; template ships with `aria-selected="true"` on the default-active tab; 30 buttons paired total | **Fixed (this session)** |
| F137 | Knowledge-islands rows mouse-only | widgets.js `wireRowKbActivation(rowEl)` helper + applied to both KI and hotspot row sites; `tabindex=0` + `role=button` + Enter/Space forward → click; `:focus-visible` outline on `tr.{hotspot,ki}-row` | **Fixed (this session)** |
| F138 | `startViewTransition` ignores reduced-motion | widgets.js:694-700 now matches `prefers-reduced-motion` and runs `updateFn()` synchronously | **Fixed on main (PR #62)** |
| F142 | Sparse tracing in analyses | Exactly 3 `tracing::*` calls across 32 analysis files (lead_time.rs:86, clones.rs:95, clone_coupling.rs:278) | Active confirmed |
| F144 | No CI dogfooding | New `dogfood` job in `ci.yml` runs release `codelore analyze --format gha` for PR annotations + `codelore diff` on PR events for step-summary; `continue-on-error: true` during bake-in | **Fixed (this session)** |
| F145 | main.rs dispatch boilerplate | 2-D `match (format, &analysis)` collapsed to a 1-D `match &analysis` delegating to 32 per-analysis `dispatch_*` fns + an `EmitCtx`; byte-identical output verified (228 pairs, only env-driven diffs in clones/wip_age_days/sarif-run-id), exit codes identical | **Fixed (this session)** |
| F146 | json.rs trivial shims | `grep -cE '^pub fn write_[a-z_]+_json' = 29` — no change | Active confirmed |
| F148 | csv.rs + markdown.rs per-analysis emitters | Both still ~25KB per-analysis files (csv.rs 25825 bytes, markdown.rs 25534 bytes) | Active confirmed |
| F149 | hunks schema lacks PK / NOT NULL | Schema tightened to NOT NULL + composite PK + index; wired entire ingest pipeline (gix `count_loc_and_hunks` + `diff_hunks` proper impl + walker populates `FileChange.hunks` + `append_change` writes rows); differential test asserts gix == cli hunks | **Fixed (this session)** |
| F121 | `fishers_exact` crate unmaintained (last release 2018-11) | In-tree port in new `crate::stats::fisher_two_tail_pvalue` module (~150 LOC); supply-chain dependency eliminated; output matches the upstream's `fishers_exact(&[a,b,c,d]).two_tail_pvalue` to ≤ 1e-12 relative error across 8 regression cases | **Fixed (this session)** |
| V4 | `widgets.js` per-widget registry | New `const WIDGETS = [{ name, render, rerender? }]` array at the top of §3 Boot; single `WIDGETS.forEach(w => { w.render(); /* register theme rerender per the `rerender` flag */ })` loop replaces the prior 60-LOC sequence of duplicated `renderXxx()` + `_codeloreRerenderers.push(() => renderXxx(...))` blocks. 14 widgets registered uniformly; `rerender: false` opts out (KPI tiles, KI table, hotspot table), `rerender: 'theme'` opts into the token-cache-invalidating path (hotspot circle-pack), default falls through to `_codeloreRerenderers.push`. SPA integration test + browser smoke test green. Adding a widget is now one line. | **Fixed (this session)** |
| F132 | Hardcoded hex colors break light theme | 4 sites in `widgets.js` (sankey label `#e6e6e6`, treemap label `#fff`, calendar-heatmap 5-band ramp, 15-color author palette) externalised to CSS custom properties — new `--label-on-dark`, `--label-on-saturated`, `--heatmap-{1..5}`, `--chart-palette-{1..15}` tokens with separate light-theme overrides that retune the heatmap ramp's "low" band (from `#1a4a2c` invisible-on-white to `#c8e6c9` desaturated mint) and deepen the author palette saturation so colors don't wash out against light cards. JS sites swapped to `token('--name')` (theme-aware + cache-invalidating). Widget entries `coupling-sankey` / `hotspot-treemap` / `calendar-heatmap` upgraded to `rerender: 'theme'` so the token cache flushes on theme toggle. `grep` for `'#[0-9a-fA-F]{3,6}'` in widgets.js now returns zero hits; browser smoke test green. |
| F133 | No responsive layout below ~1280px viewport | Dashboard grid container swapped from `xl:grid-cols-2` to `md:grid-cols-2` so 2-col kicks in at tablet portrait (≥ 768 px) instead of waiting until desktop (≥ 1280 px). Wide widgets keep `xl:col-span-2` so they only span both columns at desktop; at md/lg they sit in the normal 2-col grid one per column. Mobile (< 768 px) stays at single-column. The Tailwind v4 bundle was rebuilt (`tailwindcss -i tailwind-src/input.css -o tailwind.daisyui.min.css --minify`) so `md\:grid-cols-2` is included alongside the existing `xl\:grid-cols-2`. SPA integration + browser smoke tests green; viewports 768–1279 px now get a proper 2-col layout instead of the uncompressed desktop view. |
| F97 | SPA boot-time render storm blocks first paint | Synchronous `WIDGETS.forEach` boot loop replaced by an `async function bootWidgets()` IIFE that calls `w.render()` for each entry, registers the rerenderer (theme/regular/none), and `await yieldToMain()` between widgets (not after the last — a trailing yield is a wasted task). First widget (kpi-tiles) is cheap structural HTML, so the browser paints page chrome + KPI cards immediately, then incrementally fills in the 13 heavier widgets as the event loop yields. `yieldToMain` uses the existing `scheduler.yield()` / `MessageChannel.postMessage` fallback ladder shipped for F134/F135. Smaller scope than the audit's "split JSON into per-widget blocks" — the JSON parse itself is fast (a few hundred KB worst-case); the bottleneck was synchronously rendering 14 ECharts widgets before yielding. SPA integration + browser smoke tests green. |
| F150 | Schema version disjoint, no startup validation | `facts/schema.rs:10` `CURRENT_SCHEMA_VERSION` const + `facts/mod.rs:69` `validate_schema_version()` at `open_read_only` bails on mismatch | **Fixed on main (PR #61)** |
| F151 | Leiden non-deterministic | `communities.rs:58 LEIDEN_SEED` + `:148-150 LeidenConfig { seed: Some(LEIDEN_SEED), .. }` + regression test :269-316 | **Fixed on main (PR #61)** |
| F152 | clone_group_id std HashMap | `clones/extractor.rs:151-152` switched to `BTreeMap<[u8;32], _>` | **Fixed on main (PR #61)** |
| F153 | I/O errors → exit 5 | `error.rs:22` single `Io(#[from] std::io::Error)` variant; `:68` still maps `Io(_) → 5` alongside `Output(_)`; no `RepoIo` variant | Active confirmed |
| F154 | diff base==head no guard | `diff.rs:563-569` explicit `if base_sha == head_sha { anyhow::bail!(...) }` pre-check with SHA + range context | **Fixed on main (PR #62)** |
| F155 | DiffOutput medians default 0.0 | `diff.rs:48,52` `Option<f64>` for both fields; `:673` populates `Some/None` per gate-ran branch | **Fixed on main (PR #60)** |
| F156 | Thresholds/Gates/DiffGates no `deny_unknown_fields` | `quality_gates/mod.rs:46,55,70` all three structs carry `#[serde(deny_unknown_fields)]` | **Fixed on main (PR #60)** |
| F157 | F147's guard wraps wrong list | `analysis.rs:154-164 registry!` macro expands ONCE into both the `&[AnalysisName::$variant,*]` array and the const `_guard` match — single source of truth | **Fixed on main (PR #60)** |
| F158 | SARIF informationUri hardcodes wrong URL | `sarif.rs:20 CODELORE_HOMEPAGE` + `:26-27 CODELORE_RESEARCH_FOUNDATIONS_URL` used at all 5 sites (informationUri ×3 + helpUri ×2). `grep emre/codescene = ∅` | **Fixed on main (PR #63)** |
| F159 | SARIF artifactLocation.uri not percent-encoded | `Cargo.toml:46 percent-encoding = "2"` + `sarif.rs:54 percent_encode_path(p)` applied at `:177, :354, :531` (all 3 emitters) | **Fixed on main (PR #63)** |
| F160 | Kamei NDEV/EXP same-second `<` vs `<=` inconsistent | `kamei/mod.rs:227, :246, :304` all use strict `prev.date < c.date`; `:278` documents the unified semantic; no `<=` in code paths | **Fixed on main (PR #64)** |
| F161 | Vec<Row> in every emitter | json.rs/markdown.rs/sarif.rs all still `rows: &[T]`; no `EmitterStream` trait | Active confirmed |
| F162 | Parquet types drift from CSV row-type | `parquet.rs:13` still raw `COPY … TO PARQUET`; no `CAST … AS UINTEGER`; no documentation note | Active confirmed |
| F163 | SARIF automationDetails.id static | `sarif.rs:81 automation_id_for(prefix)` appends per-run 16-hex SHA-256 suffix; applied at all 3 sites (:124, :312, :474) | **Fixed on main (PR #63)** |
| F110 | Differential test coverage incomplete | `differential_repo_test.rs:522 head_sha_matches` + 11 sibling tests at lines 62, 97, 139, 169, 209, 241, 265, 316, 390, 450, 485, 537 | **Fixed on main (PR #57)** |
| F112 | Provenance manifest missing reproducibility | `provenance/mod.rs:94,98,114,117,122` — all 5 reproducibility fields present; builder populates from real sources | **Fixed on main (PR #57)** |
| F143 | SPA browser smoke test | `tests/spa_browser_test.rs` exists; `Cargo.toml:127 browser-tests` feature wired; `ci.yml:174` runs `cargo test --features browser-tests,spa,test-support -p codelore-lib --test spa_browser_test` | **Fixed on main (PR #56)** |

`cargo deny check advisories` clean as of validation date — confirms no F-finding maps to a live CVE.

**Pruning note**: 17 findings that the 2026-06-16 validator marked "Fixed-on-branch" all reached main (PRs #56-#64 merged via v0.7.0 / v0.8.0). Their full §4 bodies have been removed from this report and condensed into the §3 closure log per the report's stated policy (line 4). F113 count corrected (8, not 13/17). F127 reclassified as Partial — see updated §4 entry. F120 + F124 closed this session (URL fix + MSRV policy doc). F116 refuted this session after reading both bot configs — Renovate handles `cargo`, Dependabot handles `github-actions`; they're partitioned, not duplicated.

---

## 5. Next Audit Cycle

**Current Active count after this validation pass + closure annotations**:

- **Closed on main (added to §3 closure-log)**: F110, F112, F117, F118, F120 (URL half), F124 (policy half), F125, F126, F127 (full — entropy rewrite closes the remainder), F128, F129, F130, F134, F135, F138, F142, F143, F146, F150, F151, F152, F153, F154, F155, F156, F157, F158, F159, F160, F163.
- **REFUTED this session**: F116 (Renovate + Dependabot partitioned by ecosystem) + F123 (crossbeam 0.8.4 + num-format 0.4.4 are current releases) — see §3 newly-refuted block.
- **Closed by side-effect**: F162 — Parquet writers now delegate to shared SQL generators that preserve CSV row-type contract via explicit casts. Verified 2026-06-21.
- **Active**: F119, F148, F161, F164, F165 = **5 Active findings** with file:line citations + severity + suggested-fix shape, ready for the next contributor to pick up.

The next sweep should re-open with F-IDs starting at **F166**.

### Deferred — discovery pass (Workflow `wf_902c8b32-45d`)

The 2026-06-18 audit attempted to fan out 8 fresh discovery dimensions (architecture, backend-perf, rust-best-practices, spa-frontend, dep-currency, test-quality, code-design, security-correctness) with 3-lens adversarial verification per candidate, looped to dry. The validation half (54 findings re-verified in parallel) completed; the discovery half — 16 subagents across 2 rounds — was lost to an Anthropic weekly-quota cap (resets 2026-06-21 21:00 Europe/Amsterdam). The workflow script + run journal persist at `/Users/emrec/.claude/projects/-Users-emrec-Projects-playground-codelore/8db19d2a-538c-4ef3-aaaa-e3093c56c8c8/workflows/scripts/codelore-deep-audit-wf_902c8b32-45d.js` — resumable with `Workflow({scriptPath, resumeFromRunId: "wf_902c8b32-45d"})` once quota resets, which will cache the entire validation phase and only re-run the discovery half.

**Validation methodology held**: every Active finding above re-verified against current `main` HEAD via direct source-line grep / wc / sed; results recorded in §4½. Closures show explicit PR numbers so the report doesn't claim closures that haven't reached main.
