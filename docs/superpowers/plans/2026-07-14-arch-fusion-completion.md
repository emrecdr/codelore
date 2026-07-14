# Architecture Fusion Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the structure×history fusion over the architecture graph: a `cycle-health` analysis (which tangles are alive, where to cut), a DSM Fusion cell-mode (structure-vs-history agreement in the matrix), and corpus-relative architecture percentiles (with the calibration pipeline extended to pool repo-level metrics).

**Architecture:** Three additive units on existing seams. Unit A consumes the `import_graph` kernel + windowed churn SQL. Unit B is SPA-JS-only over payload data that already ships. Unit C threads a new optional `repo_metrics` section through the calibration artifact, extends head-only ingest with the HEAD-time imports passes, and adds optional rows to `architecture-metrics`.

**Tech Stack:** Rust workspace, DuckDB (`FactsDb`, read-only post-ingest), vendored ECharts SPA (vanilla JS + Alpine store), serde.

**Spec:** `docs/superpowers/specs/2026-07-14-arch-fusion-completion-design.md` — binding for all semantics.

## Global Constraints

1. `cargo fmt --all --check` + CI-exact `cargo clippy --workspace --all-targets --all-features -- -D warnings` before EVERY commit; full `cargo test --workspace --features test-support,spa` before the final commit of each task. No `#[allow]`; fix root causes.
2. No `unwrap()`/`expect()` outside tests. Analysis-phase fact-store writes forbidden (read-only after ingest); temp writes via prepared `INSERT` only — never `Appender` outside ingest.
3. No ticket IDs, plan §IDs, or version references in code/docs; CHANGELOG.md `[Unreleased]` gets one entry per user-visible change; comments describe current contracts in the codebase's voice.
4. Everything is ADDITIVE: no shipped row-shape changes; `CalibrationArtifact.format_version` stays `1`; absent `repo_metrics` = no lens; `architecture-metrics` output without an active artifact is byte-identical to today (contract-tested).
5. SPA: no new vendored libraries; new JS follows `40_architecture.js` idioms (`modulePath`, `aggregateImportsAt`, `mountEcharts`, Alpine `layout` store persistence, theme-reactive rerender registration). Never color-only encoding.
6. Windowed semantics reuse `Options::window_days` (default 90, anchored to the repo's last commit date) and the lineage-aware changes source (`analyses/lineage.rs::source_table(opts)`).
7. Commit messages: Conventional Commits. NEVER add `Co-Authored-By`.

## Validated interfaces (source-checked; cite, don't re-derive)

- `import_graph.rs`: `ImportGraph { id_to_path: Vec<String>, path_to_id: HashMap<String, usize>, adj: Vec<Vec<usize>> }`; `build_import_graph(db) -> Result<Rc<ImportGraph>>`; `build_import_graph_from_edges(&[(String,String)]) -> ImportGraph`; `tarjan_scc(&[Vec<usize>]) -> Vec<Vec<usize>>`; `graph_metrics(&ImportGraph) -> GraphMetrics { n, ccd, propagation_cost, cycle_count, largest_cycle, cyclic_nodes }`.
- `dependency_cycles.rs`: `DependencyCycleRow { cycle_id: u32, size: u32, path: String }` — one row PER MEMBER; cycle ordering = size desc. Its test (`tests/dependency_cycles_test.rs`) builds an inline Rust crate with `src/a.rs ↔ src/b.rs` cycle + `src/c.rs → a`, single-date commits.
- `coupling.rs`: `CouplingRow { entity_a, entity_b, shared: u32, revs_a, revs_b, average_revs: u32, degree: f64, fisher_p: f64 }`; `partner_index(&[CouplingRow]) -> HashMap<String, HashSet<String>>`.
- `calibration.rs`: `CalibrationArtifact { format_version: u32, corpus_vintage, generated_at, repos_included, repos_attempted, languages: Vec<LanguageTable> }`; `LangObservations::observe(&mut self, language, metric, value)`; `build_from_observations(vintage, generated_at, obs) -> CalibrationArtifact`; `merge(base, additional)`; `load_active_artifact(&Options) -> Result<Option<Cow<'static, CalibrationArtifact>>>`; `percentile(art, language, metric, value) -> Option<CorpusPercentile { p, beyond_corpus }>` (1001-breakpoint interpolation — Unit C needs a NEW raw-values helper).
- `facts/ingest/mod.rs`: `ingest_head_only` (early-return at ~line 78); full path calls `populate_imports_at_head(repo, opts, &live_paths, &head_rev) -> Result<usize>` then `resolve_imports_at_head(&live_paths, &head_rev) -> Result<usize>` (both `pub(super)`, in `imports_head.rs`).
- `cache.rs`: `const CACHE_EPOCH: &str = "schema_v9";` (~line 25).
- `architecture_metrics.rs`: `run_architecture_metrics(db, opts) -> Result<Vec<ArchitectureMetricRow { metric: String, value: String }>>`; emitters `write_architecture_metrics_csv` / markdown exist; dispatch at `main.rs::dispatch_architecture_metrics` (~line 3255).
- SPA payload: `SpaDashboard.coupling: Vec<CouplingRow>` and `.imports: Vec<ImportEdgeRow>` already serialized; `40_architecture.js` helpers `modulePath(p, depth)`, `aggregateImportsAt(imports, depth)`; Alpine store `window.Alpine.store('layout')` with persisted fields (`archGraphDepth`, `archGraphLayout` precedent).
- Registry: 53 analyses; add-an-analysis = enum variant + `as_str` arm + `registry!` entry + explain tuple + dispatch fn + csv/markdown emitters + tests (mirror `ArchitectureMetrics`'s touches: enum ~line 97, as_str ~253, registry ~341).

Known gap the plan covers: NO existing fixture combines resolvable import cycles with multi-date commits — Task 1 builds one.

---

### Task 1: `cycle-health` computation + fixture + unit tests

**Files:**
- Create: `crates/codelore-lib/src/analyses/cycle_health.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs` (add `pub mod cycle_health;` alphabetically)
- Test: `crates/codelore-lib/tests/cycle_health_test.rs` (new)

**Interfaces:**
- Consumes: `import_graph::{build_import_graph, tarjan_scc, graph_metrics, build_import_graph_from_edges, ImportGraph}`; `lineage::source_table(opts)`; `Options::window_days`.
- Produces: `pub struct CycleHealthRow { pub cycle_id: u32, pub size: u32, pub members_preview: String, pub heat_pct: f64, pub verdict: String, pub extract_candidate: String, pub predicted_pc_drop: Option<f64> }` (derives `Debug, Clone, serde::Serialize, serde::Deserialize`); `pub fn run_cycle_health(db: &FactsDb, opts: &Options) -> Result<Vec<CycleHealthRow>>`. Also `pub(crate) fn extraction_candidate(adj: &[Vec<usize>], scc: &[usize]) -> usize` (unit-testable core).

**Semantics (from the spec — implement exactly):**
- Non-trivial SCCs (size ≥ 2) from `build_import_graph(db)`; `cycle_id` = rank by size desc, ties by lexicographically smallest member path.
- `members_preview`: first 3 members lexicographically joined with `", "`, plus `" +N more"` when size > 3.
- `heat_pct`: 100 × (window LOC churn of members) / (window LOC churn of all files); one SQL over `{src}` = `lineage::source_table(opts)` joined to `commits` with `co.date >= (SELECT MAX(date) FROM commits) - INTERVAL (?) DAY` (bind `opts.window_days`); members matched via a `path IN (…)` prepared query per cycle is O(cycles) round-trips — instead fetch per-path window churn ONCE into a `HashMap<String, i64>` and sum in Rust. Denominator 0 → `heat_pct = 0.0`.
- `verdict`: `"live"` iff any member has window churn > 0 (a commit touching it in-window with zero LOC counts as touched: use per-path window REVISION count > 0, not LOC — fetch `COUNT(rev)` in the same query), else `"fossil"`.
- `extraction_candidate(adj, scc)`: for each member m, run `tarjan_scc` on the SCC-induced subgraph minus m; score = (largest surviving SCC size, total surviving cyclic nodes, member path) — minimize lexicographically over that tuple. Returns the winning node id.
- `predicted_pc_drop`: only when `size <= 64`: `graph_metrics(&full_graph).propagation_cost - graph_metrics(&graph_without_candidate).propagation_cost` where `graph_without_candidate` is rebuilt via `build_import_graph_from_edges` from the original edge list minus every edge touching the candidate node (node removal). When `size > 64`: `None`, and `extract_candidate` = the member with highest in-SCC degree (in+out edges within the SCC), ties lexicographic.
- Row order: `heat_pct` desc, then `size` desc, then `cycle_id` asc. Respect `opts.rows_limit` the way `dependency_cycles.rs` does (read it and mirror).

- [ ] **Step 1: Write the failing tests.** In `tests/cycle_health_test.rs`, build an inline fixture like `dependency_cycles_test.rs`'s (copy its `run_git`/write helpers) but with DATED commits (`GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE` env, the `commit_at` pattern from `tests/ingest_test.rs`): commit 1 @ 2026-01-01 creates `src/a.rs` ↔ `src/b.rs` (mutual `use crate::…`) + `src/c.rs → a` + `src/lib.rs` declaring mods; commit 2 @ 2026-06-01 modifies `src/a.rs` (adds a line). Tests:

```rust
#[test]
fn cycle_health_reports_live_cycle_with_heat() {
    // fixture as above; ingest with Options { window_days: 90, ..permissive }
    let rows = run_cycle_health(&db, &opts).expect("run");
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.size, 2);
    assert_eq!(r.verdict, "live"); // a.rs modified at repo-max-date
    assert!(r.heat_pct > 0.0 && r.heat_pct <= 100.0);
    assert!(r.members_preview.contains("src/a.rs"));
    assert!(r.predicted_pc_drop.is_some()); // size 2 <= 64
}

#[test]
fn cycle_health_fossil_when_window_excludes_member_churn() {
    // same fixture but commit 2 touches ONLY src/d.rs (outside the cycle);
    // window (90d from max date) contains no member churn
    let rows = run_cycle_health(&db, &opts).expect("run");
    assert_eq!(rows[0].verdict, "fossil");
    assert_eq!(rows[0].heat_pct, 0.0);
}
```

Plus an in-module unit test in `cycle_health.rs` for `extraction_candidate` on a hand-built adjacency: a 3-cycle `0→1→2→0` PLUS edge `1→0` (so removing node 1 leaves `0,2` acyclic but removing 0 or 2 leaves the 2-cycle `0↔1` or none — hand-derive the true winner and assert it; also assert the deterministic tie-break with a symmetric 2-cycle → lexicographically smaller path wins).
- [ ] **Step 2:** `cargo test -p codelore-lib --features test-support --test cycle_health_test` → FAIL (module/function missing).
- [ ] **Step 3:** Implement `cycle_health.rs` per the semantics block. Rustdoc on the module: what heat/verdict/candidate mean, the size-64 bound and honest absence, window anchoring. Cite Baldwin/MacCormack core-periphery + DV8 hotspot lineage the way sibling analyses do (short, no URLs beyond the existing citation style).
- [ ] **Step 4:** Tests pass: the two integration tests + unit tests. Run also `cargo test -p codelore-lib --features test-support --test dependency_cycles_test` (kernel untouched, must stay green).
- [ ] **Step 5:** Commit `feat(analyses): cycle-health — behavioral heat and cut points for import cycles`.

### Task 2: `cycle-health` registry + CLI + emitters

**Files:**
- Modify: `crates/codelore-lib/src/analysis.rs` (enum variant `CycleHealth`, `as_str` arm `"cycle-health"`, `registry!` entry — mirror `ArchitectureMetrics`'s three touch points)
- Modify: `crates/codelore-cli/src/main.rs` (explain tuple near the architecture-metrics one; dispatch arm `AnalysisName::CycleHealth => dispatch_cycle_health(...)`; `fn dispatch_cycle_health` next to `dispatch_architecture_metrics` handling `"csv" | "json" | "markdown"` + `unsupported_format`)
- Modify: `crates/codelore-lib/src/output/csv.rs` (`write_cycle_health_csv`: header `cycle-id,size,members,heat-pct,verdict,extract-candidate,predicted-pc-drop`; floats `{:.2}`; `predicted_pc_drop` empty cell when `None`)
- Modify: `crates/codelore-lib/src/output/markdown.rs` (`write_cycle_health_markdown` mirroring the csv columns; `—` for absent drop)
- Test: `crates/codelore-cli/tests/cli_test.rs` (smoke: csv header + exit 0 on a fixture repo — copy the shape of the newest analysis smoke test in that file)

**Interfaces:** Consumes `run_cycle_health` + `CycleHealthRow` from Task 1. Produces the registered analysis name `"cycle-health"`.

- [ ] **Step 1:** Write the failing CLI smoke test (assert stdout starts with the csv header above).
- [ ] **Step 2:** Run it → FAIL (unknown analysis name).
- [ ] **Step 3:** Wire all five touch points. Explain text: one line "behavioral heat + extraction candidate per import cycle", paragraph naming heat/verdict/candidate/drop semantics and the size bound.
- [ ] **Step 4:** Smoke test passes; `cargo test -p codelore-lib --features test-support analysis` (registry guard tests) green.
- [ ] **Step 5:** CHANGELOG `[Unreleased]` → `### Added`: the `cycle-health` analysis (heat, live/fossil, extraction candidate, predicted propagation-cost drop; size-64 bound stated). Commit `feat(cli): register cycle-health analysis`.

### Task 3: `repo_metrics` artifact section + raw-values percentile helper

**Files:**
- Modify: `crates/codelore-lib/src/calibration.rs`
- Test: in-module `#[cfg(test)]` + extend `crates/codelore-lib/tests/calibration_test.rs`

**Interfaces (produces — Tasks 5 and 6 rely on these exact names):**
```rust
/// Repo-level metric pools: one observation per corpus repo. Absent on
/// artifacts built before this section existed — absent = no lens.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RepoMetrics {
    /// Sorted ascending. Key: metric name ("propagation_cost", "cycle_file_share").
    pub values: std::collections::BTreeMap<String, Vec<f64>>,
}
// On CalibrationArtifact:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub repo_metrics: Option<RepoMetrics>,

/// Midpoint-rank percentile of `value` among `sorted` (ascending). None when
/// `sorted` is empty. p in 0..=1: (count_less + 0.5*count_equal) / n.
#[must_use]
pub fn raw_percentile(sorted: &[f64], value: f64) -> Option<f64>
```
Also: `build_from_observations` gains nothing; instead a new `pub fn attach_repo_metrics(artifact: &mut CalibrationArtifact, pools: RepoMetrics)` that sorts each vec ascending and sets the field (empty pools → sets `None`).

- [ ] **Step 1:** Failing tests: (a) serde roundtrip with and without `repo_metrics` (an old-artifact JSON WITHOUT the field must deserialize with `repo_metrics: None` — paste a minimal artifact JSON literal in the test); (b) `raw_percentile` table-driven: empty → None; `[1,2,3]` value 2 → 0.5; value 0 → ~0.1667 (midpoint of zero less, wait — 0 less + 0 equal? value 0 < all → (0+0)/3 = 0.0); value 4 → 1.0; ties `[1,2,2,3]` value 2 → (1 + 0.5*2)/4 = 0.5. Hand-verify each expected number in the test comments.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Pass, plus the existing calibration determinism tests stay green (the embedded world artifact lacks the field → None path exercised).
- [ ] **Step 5:** Commit `feat(calibration): optional repo-level metric pools with midpoint-rank percentiles`.

### Task 4: head-only ingest gains HEAD-time imports + CACHE_EPOCH bump

**Files:**
- Modify: `crates/codelore-lib/src/facts/ingest/mod.rs` (`ingest_head_only`: after the complexity pass, call `self.populate_imports_at_head(repo, opts, &live_paths, &head_rev)?` then `self.resolve_imports_at_head(&live_paths, &head_rev)?` — the same two calls, same order, as the full path; update the function's rustdoc to say head-only populates entities + complexity_metrics + imports)
- Modify: `crates/codelore-lib/src/cache.rs` (`CACHE_EPOCH` `"schema_v9"` → `"schema_v10"`, comment per the file's convention: head-only caches now carry imports; older head-only caches lack them)
- Test: extend the equivalence test in `crates/codelore-lib/tests/ingest_test.rs` (`head_only_ingest_matches_full_ingest_complexity_and_leaves_history_empty`): additionally assert the resolved-imports row multiset (`SELECT src_path, target_path FROM imports WHERE target_path IS NOT NULL ORDER BY 1,2`) is identical between the two modes, and `commits` still 0 rows head-only.

- [ ] **Step 1:** Extend the test with the imports-equality assertion → run → FAIL (head-only has 0 imports rows).
- [ ] **Step 2:** Implement the two calls + epoch bump + rustdoc.
- [ ] **Step 3:** Test passes; also run `--test calibration_test` and the cache-key tests (must stay green — key derivation untouched).
- [ ] **Step 4:** Commit `feat(facts): head-only ingest extracts HEAD-time imports`.

### Task 5: calibrate pools repo-level architecture metrics

**Files:**
- Modify: `crates/codelore-cli/src/main.rs` (`run_calibrate_cmd` + `calibrate_one_repo` + a new `fn pool_repo_metrics(db: &FactsDb, pools: &mut codelore_lib::calibration::RepoMetrics) -> Result<()>` beside `pool_complexity`)
- Test: extend the calibrate e2e in `crates/codelore-cli/tests/cli_test.rs`

**Interfaces:** Consumes Task 3's `RepoMetrics`/`attach_repo_metrics` and Task 4's head-only imports. `pool_repo_metrics` builds the graph via `codelore_lib::cli_api::analyses::import_graph::build_import_graph(db)` (verify the `cli_api` re-export path exists — if `import_graph` isn't re-exported, add it to `cli_api` the way sibling analyses are), computes `graph_metrics`, and pushes `propagation_cost` and `cycle_file_share = cyclic_nodes as f64 / n.max(1) as f64` into `pools.values` under those exact metric names. Repos with an EMPTY import graph (n == 0) contribute NOTHING (skip both metrics — a no-Tier-1 repo must not drag the pool to zero); log one debug line.

- [ ] **Step 1:** Extend the local-manifest calibrate e2e: after the run, parse the artifact JSON and assert `repo_metrics.values["propagation_cost"]` has one entry per included repo and every value is in `[0,1]`, and `cycle_file_share` likewise. Run → FAIL (field absent).
- [ ] **Step 2:** Implement pooling: `calibrate_one_repo` gains a `&mut RepoMetrics` param threaded like `obs`; `run_calibrate_cmd` calls `attach_repo_metrics(&mut artifact, pools)` after `build_from_observations`. `merge()` behavior: when either side lacks `repo_metrics`, keep whichever exists; when both exist, concatenate + re-sort each metric vec (document: exact pooling, unlike the quantile blend). Add a merge unit test in calibration.rs for that rule.
- [ ] **Step 3:** e2e passes. **Step 4:** Commit `feat(calibrate): pool repo-level architecture metrics into the artifact`.

### Task 6: `architecture-metrics` corpus rows + additivity contract

**Files:**
- Modify: `crates/codelore-lib/src/analyses/architecture_metrics.rs`
- Test: extend `crates/codelore-lib/tests/architecture_metrics_test.rs` (or create if the analysis's tests live elsewhere — locate them first)

**Interfaces:** Consumes `calibration::{load_active_artifact, raw_percentile}`. After building today's seven rows, when `load_active_artifact(opts)?` yields an artifact whose `repo_metrics` is `Some`, append rows in this exact order:
```text
corpus_percentile:propagation_cost   <raw_percentile vs values["propagation_cost"], "{:.2}">
corpus_percentile:cycle_file_share   <same vs values["cycle_file_share"], "{:.2}">
corpus_n                             <values["propagation_cost"].len() as shown, integer>
```
Each row only when its metric's pool is non-empty; `corpus_n` only when at least one percentile row was emitted (use the propagation_cost pool length; if only cycle_file_share exists, use that one — state the rule in a comment).

- [ ] **Step 1:** Failing tests: (a) additivity — run with `opts.calibration = None` and embedded-world artifact active-but-lacking-`repo_metrics` (the CURRENT embedded artifact lacks it, so the default path already proves absence) → rows identical to a hardcoded expectation of exactly 7 metric names; (b) with a synthetic artifact file (write a temp JSON containing `repo_metrics` with known pools) passed via `opts.calibration` → the three extra rows appear with hand-computed percentile values.
- [ ] **Step 2:** Run → (a) may already pass, (b) FAILS. **Step 3:** Implement. **Step 4:** Both pass; CLI smoke `--analysis architecture-metrics --format csv` on a fixture stays green.
- [ ] **Step 5:** CHANGELOG `### Added` entry (corpus-relative architecture percentiles; honest coarse base — "percentile among N corpus repositories"). Commit `feat(analyses): corpus-relative architecture percentiles`.

### Task 7: DSM Fusion cell-mode (SPA)

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/js/40_architecture.js` (`renderArchMatrix` + a small pure helper `classifyCells(structEdges, couplingPairs)`)
- Modify: `crates/codelore-lib/src/output/spa/template.html` (legend row inside `widget-arch-matrix`; the toggle buttons live in JS via the existing `wt-btn` injection pattern — read how the force graph injects its layout toggle and mirror it; persist mode in `Alpine.store('layout').archMatrixMode` with `'structure'` default)
- Test: `crates/codelore-lib/tests/spa_integration_test.rs` (payload assertion) + `crates/codelore-lib/tests/spa_browser_test.rs` (toggle click)

**Semantics:** In Fusion mode, aggregate `data.coupling` pairs to the current module depth with the same `modulePath` used for imports; a module pair's coupling weight = max `degree` among its file pairs. Cell classes:
- both structural edge AND coupling ≥ any aggregated pair → `agree`: blend of the structural hue, opacity graded by coupling degree (`0.45 + 0.5 * degree/maxDegree`);
- structural only → `struct-only`: current color at 0.35 opacity;
- coupling only (no structural edge either direction at this depth) → `temporal-only`: the violation amber used by the force graph's dashed edges;
- back-edges below the diagonal keep red in BOTH modes (class applies to the above-diagonal rendering only; below-diagonal red wins).
Tooltip: `A → B — <class label>, imports: N, co-change degree: D%`. Legend row lists the four cell renderings with text labels. Structure mode = today's rendering, byte-identical options object (assert by keeping the existing code path untouched when mode==='structure'). When `data.coupling` is empty, Fusion mode renders the structural view plus a one-line in-widget hint (`No co-change data — showing structure only`), matching the spec's honest-absence rule; cover it with an integration-test assertion on a coupling-free fixture.

- [ ] **Step 1:** Browser test first: extend `spa_browser_test.rs` on the coupling fixture — locate the new mode toggle by its `wt-btn` label `Fusion`, click it, assert zero console errors and that the matrix chart re-rendered (existing tests show the assertion idiom — reuse it). Integration test: assert the embedded JSON has non-empty `coupling` AND `imports` for the fixture (precondition for fusion).
- [ ] **Step 2:** Run browser test → FAIL (no toggle).
- [ ] **Step 3:** Implement `classifyCells` as a pure function + wire the toggle + legend + tooltip. Register mode in the Alpine layout store next to `archGraphLayout`.
- [ ] **Step 4:** Browser suite green (`cargo test -p codelore-lib --features "browser-tests spa test-support" --test spa_browser_test`; note gracefully if headless Chrome unavailable and rely on the integration test + report it).
- [ ] **Step 5:** CHANGELOG `### Added` (DSM Fusion cell-mode: structure×history agreement classes in the matrix). Commit `feat(spa): DSM fusion cell-mode — structure vs co-change agreement`.

### Task 8: factor-tile annotation, world artifact rebuild, docs

**Files:**
- Modify: `crates/codelore-cli/src/main.rs` (`build_spa_dashboard` region: where the Architecture `FactorTile.detail` is composed — append `, P<pp> of <n> corpus repos` for propagation cost when the corpus rows are present; degrade-to-absent otherwise)
- Modify: `crates/codelore-lib/src/calibration/world.calib.json` (rebuilt artifact CONTENT — same path, now carrying `repo_metrics`)
- Modify: `docs/advanced-usage.md` (cycle-health section beside the architecture suite; corpus-architecture-percentile paragraph in the calibration section, stating the ~N-repo coarse base; DSM fusion mode paragraph), `README.md` (one line in the analysis table for `cycle-health`), `calibration/README.md` (repo_metrics mentioned in the artifact description)
- Test: existing embedded-artifact tests must pass against the rebuilt artifact

- [ ] **Step 1:** Rebuild the world artifact: `cargo run --release -p codelore-cli -- calibrate --repos calibration/corpus.toml --vintage world-<today's date YYYY-MM-DD> --output <scratch>/world.calib.json --cache-dir <scratch>/cache` (shallow head-only — expect minutes). Validate: `repos_included` matches the previous artifact's, `repo_metrics.values` pools have one entry per included repo, complexity quantiles UNCHANGED vs the shipped artifact for a spot-checked language/metric (the same trees produce the same pools — diff the `languages` section programmatically; report any drift as BLOCKED, do not install).
- [ ] **Step 2:** Install the artifact content; update the vintage string wherever the previous vintage is named (CHANGELOG [Unreleased] may reference it — grep). Run the embedded-artifact test suite (`cargo test -p codelore-lib --features test-support calibration`).
- [ ] **Step 3:** Factor-tile detail wiring + SPA integration assertion (detail string contains `corpus` only when artifact has repo_metrics).
- [ ] **Step 4:** Docs + README + CHANGELOG wording per Global Constraint 3. Real-CLI verification: `cycle-health` and `architecture-metrics` (csv) against this repo with the rebuilt embedded artifact — paste outputs in the report.
- [ ] **Step 5:** Full workspace suite + fmt + CI-exact clippy. Commit `feat(calibration): world artifact with repo-level architecture pools + docs`.

---

## Verification (whole-plan)

- Full gates on the final tree: fmt, CI-exact clippy, `cargo test --workspace --features test-support,spa`.
- Real CLI on this repo: `--analysis cycle-health --format markdown` (expect the known rca tangle as a live cycle), `--analysis architecture-metrics --format csv` (expect `corpus_percentile:*` rows), SPA build + browser suite.
- Additivity spot-proof: run `architecture-metrics` with `--calibration` pointing at an artifact WITHOUT `repo_metrics` → byte-identical to no-artifact output.
- Docs guard: `git grep -nE "F[0-9]{3}|PAR-[0-9]|Task-[0-9]" crates/ docs/advanced-usage.md README.md` → no new hits.

## Out of scope (spec follow-ups — do NOT implement)

- `arch_health`/factor formula changes from cycle heat; any new quality gate; DSM reordering by SCC; per-member cycle-health rows.
