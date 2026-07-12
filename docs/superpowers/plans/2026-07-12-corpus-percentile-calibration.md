# Corpus-Relative Percentile Scoring + Biomarkers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the corpus-relative percentile lens ("your complexity is P74 vs a reference corpus") plus three new biomarkers (deep-nesting, many-args, complex-conditional), per the approved design at `docs/superpowers/specs/2026-07-12-corpus-percentile-calibration-design.md`.

**Architecture:** A new leaf module `calibration.rs` owns the quantile-breakpoint artifact (versioned compact JSON, embedded world artifact + `--calibration` override) and interpolation lookup. A public `codelore calibrate` subcommand builds artifacts by running the standard ingest pipeline over a pinned-repo manifest and pooling **raw per-function metric** distributions per language. `CodeHealthRow` gains an additive `corpus_percentile: Option<f64>` (worst raw-dimension corpus percentile). The three new biomarkers ride a schema bump (nesting extraction implemented in the complexity layer, nargs wired from the existing rca module, boolean-conditional counting added) and fold into `structural_risk` with reweighted smell weights.

**Tech Stack:** Rust workspace, DuckDB fact store, vendored rust-code-analysis fork (`codelore-rca`), serde_json artifacts, clap CLI, existing SPA/MCP surfaces.

## Validated deviations from the spec (discovered at plan time — surface these in the PR)

1. **LCOM4 is deferred by the spec's own validation gate.** No cohesion computation exists; `codelore-rca` does not track field↔method membership for any Tier-1 language. Building it is its own initiative. v1 ships 3 of 4 new biomarkers.
2. **Corpus percentiles calibrate RAW metrics, not `structural_risk`.** Pooling `structural_risk` cross-repo would compare self-normalized quantities (each repo's risk is built from in-repo percentile intensities) — reintroducing the critique this phase fixes. The artifact carries per-language quantiles of raw function metrics (`cyclomatic`, `cognitive`, `sloc`, `nargs`, `max_nesting`); the file-level `corpus_percentile` = the MAX of the file's per-metric corpus percentiles (its worst dimension vs the world), documented as such.
3. **Artifact container is compact JSON** (repo has no CBOR/bincode precedent; serde_json is the universal convention).

## Global Constraints

- `just ci` must pass after every task (fmt, clippy `--workspace --all-targets --all-features -- -D warnings`, cargo-deny, full test suite). No `#[allow]` to mask lints.
- No `unwrap()`/`expect()` outside tests. Library errors via `CodeLoreError`, CLI via `anyhow::Context`.
- `FactsDb` is read-only after ingest; analysis-phase temp writes use prepared `INSERT`, never `Appender`.
- Shipped self-relative `percentile`, existing `band` thresholds (0.55/0.28), and the score formula weights (0.50/0.30/0.20) are UNCHANGED. `corpus_percentile` is additive: a run without calibration data must produce byte-identical output to today for all existing fields.
- New biomarkers change `structural_risk`/`score`/`band` VALUES (deliberate). Schema bump `CURRENT_SCHEMA_VERSION` "4" → "5" (facts/schema.rs:10 and types.rs:27 `SCHEMA_VERSION: u8 = 5`).
- No ticket/plan IDs or version anchors in code, comments, or docs. Docs describe the current contract only. CHANGELOG `[Unreleased]` gets one entry per user-visible change.
- Conventional Commits; never add Co-Authored-By. Commits are append-only on shared branches.
- Fixture changes go through the bundle-regeneration recipe documented in `test_support/mod.rs` (revive builder from the cited commit, rebuild, `git bundle create ... --all`, commit the bundle, update the documented HEAD SHA).
- Execution on a feature branch `feat/corpus-calibration` off main; PR at the end; the user merges.

## Validated facts the tasks rely on (do not re-derive)

- `complexity_metrics` columns (schema_v1.sql:103–113) already include `max_nesting INTEGER, mean_nesting DOUBLE, sd_nesting DOUBLE, total_nesting INTEGER` — all currently written as constant 0 from `complexity/mod.rs:36` ("Nesting is not exposed as a standalone stat"). NO `nargs` column exists.
- `codelore-rca` has `metrics/nargs.rs` (computed, never extracted). `ComplexityEntity` (complexity/mod.rs:19–41) is the extraction seam.
- Tier-1 languages (complexity/language.rs:8–14): Rust, Python, Java, JavaScript, TypeScript.
- Five current smells + weights (code_health.rs `file_structural` CTE ~lines 141–153): complex-method 0.30, god-class 0.25, large-method 0.15, dry 0.15, shotgun-surgery 0.15; `STRUCTURAL_SCALE_NO_DRY` renormalizes by /0.85 when clones excluded. Biomarker inserts: `BIOMARKERS_INSERT` (~270–315, SQL percentile intensities), `SHOTGUN_INSERT` (~320–326), Rust-side god-class/dry ranking (~390–423).
- `CodeHealthRow` (code_health.rs:86–93) derives Serialize; MCP serializes `Vec<CodeHealthRow>` directly (mcp.rs ~269–294); SPA drawer reads `score`/`cognitive`/`band` only (spa/js/10_helpers_drawer.js ~729–737); `SpaDashboard.code_health` at output/spa.rs:98.
- Consumers of code-health output (blast radius of score shifts): refactoring_targets (reads `code_health_biomarkers_v1` temp table incl. `dominant_type`), effort_exposure(`_with_health`), health_trend (scoped per-rev), quality-gate evaluators, evaluate_all_gates row reuse, SPA, MCP.
- `Options::canonical_json()` (options.rs:197–268) auto-propagates new fields to cache key + provenance; cosmetic exclusions null out `rows_limit`/`explain`/`target`; file-path fields are replaced by SHA-256 content digests (`team_map_file` pattern). Provenance manifest: provenance/mod.rs `Manifest::capture()`.
- Subcommand precedent: `Command::IngestSarif(IngestSarifArgs)` (args.rs:149, struct at 506–522), dispatch `main.rs:38–52`, impl `run_ingest_sarif_cmd` at main.rs:64.
- Gate precedent: `Gates.max_findings_in_hot_files: Option<u32>` (quality_gates/mod.rs:102), pure evaluator `evaluate_finding_overlap_rows` (~586–601), skip-with-ledger-record wiring in `evaluate_all_gates` (main.rs ~710–766), notice rendering in `emit_gate_notices` (main.rs ~398–412).
- Leaf-module precedent: `hashing.rs`; `pub mod calibration;` slots alphabetically in lib.rs between `bands` and `cache`. `stats.rs` has NO quantile helpers (only Fisher).
- Fixtures: `biomarker_repo` bundle HEAD `fc3edfb3435c690a87750c8fe0050a2497d75b60`, builder revivable from commit `b412e6d`; `medium_repo` (500 commits, HEAD `40c13e73…`) suits calibrate round-trips.
- Schema bump forces re-ingest via provenance `schema_version` comparison on cache open.

## File structure (created/modified)

- Create: `crates/codelore-lib/src/calibration.rs` — artifact model, load/save, interpolation, pooling builder.
- Create: `crates/codelore-lib/tests/calibration_test.rs` — artifact/interpolation/round-trip tests.
- Create: `calibration/corpus.toml` — world-corpus manifest (repo URL + pinned SHA + languages).
- Create: `crates/codelore-lib/src/calibration/world.calib.json` — committed world artifact (generated; may be a placeholder mini-artifact until the maintainer runs the full corpus build — see Task 12).
- Modify: `crates/codelore-rca` (nesting + boolean-op exposure if needed), `crates/codelore-lib/src/complexity/mod.rs` (+`ComplexityEntity` fields), `facts/schema_v1.sql` + `facts/schema.rs` + `types.rs` (schema 5), the ingest writer for complexity rows.
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (3 new smells, reweight, corpus_percentile join), `options.rs` (+`calibration`), `provenance` (vintage stamp), `quality_gates/mod.rs` (+gate), `output/csv.rs`/`markdown.rs` (new column), `output/spa.rs` + `spa/js/10_helpers_drawer.js`, `codelore-cli/src/args.rs` + `main.rs` (Calibrate subcommand, --calibration flag, gate wiring), `mcp.rs` (no code change expected — serde propagation; verify), docs + CHANGELOG.
- Regenerate: `biomarker_repo` bundle with nesting/nargs/conditional content.

---

## Task 1 — Nesting + nargs + boolean-conditional extraction (complexity layer)

**Files:** Modify `crates/codelore-lib/src/complexity/mod.rs`; possibly `crates/codelore-rca` (only if a metric is not reachable through the existing `FuncSpace` API — check first); Test: extend `crates/codelore-lib/src/complexity/` unit tests (find the existing module tests and mirror their fixture style).

**Interfaces — Produces:** `ComplexityEntity` gains `pub nargs: u32` and real values in the existing `max_nesting/mean_nesting/sd_nesting/total_nesting` fields, plus `pub bool_ops: u32` (boolean operator count in conditions — the complex-conditional driver).

- [ ] Step 1: Read `crates/codelore-lib/src/complexity/mod.rs` fully and the `codelore-rca` metric API it consumes (`FuncSpace`/`m.` accessors). Establish for each: (a) `nargs` — rca computes it (`metrics/nargs.rs`); find the accessor (likely `m.nargs.…`) and its per-language coverage; (b) nesting — determine whether rca exposes a nesting stat on spaces or whether max nesting must be computed by walking the space tree / re-deriving from the AST; (c) boolean-operator count — check whether rca's cognitive machinery exposes a boolean-sequence count; if not, count boolean operators via a small tree-sitter query per Tier-1 language grammar in the complexity layer. Record findings in the task report BEFORE coding.
- [ ] Step 2: Write failing unit tests against small language snippets (one per Tier-1 language where the metric applies), e.g. for Rust: a function with 4 args → `nargs == 4`; nested `if` 3 deep → `max_nesting == 3`; `if a && b || c` → `bool_ops == 2`. Use the existing complexity test fixture style.
- [ ] Step 3: Implement extraction; populate the previously-zero nesting fields and the new `nargs`/`bool_ops` fields. If a metric is genuinely unavailable for a language, it yields 0 and the task report documents which (honest-absence).
- [ ] Step 4: Tests green; `cargo test -p codelore-lib complexity`.
- [ ] Step 5: Commit `feat(complexity): extract nesting, argument-count, and boolean-conditional metrics`.

## Task 2 — Schema v5: persist the new metrics

**Files:** Modify `crates/codelore-lib/src/facts/schema_v1.sql` (add `nargs INTEGER, bool_ops INTEGER` to `complexity_metrics`), `facts/schema.rs:10` (`"4"` → `"5"`), `types.rs:27` (`SCHEMA_VERSION: u8 = 5`), the complexity-row ingest writer (find where `ComplexityEntity` maps to the `complexity_metrics` INSERT/Appender at ingest — Appender is correct there, it IS ingest). Test: extend the ingest test that asserts complexity rows.

**Interfaces — Produces:** `complexity_metrics` rows carry real `max_nesting`, new `nargs`, new `bool_ops` for downstream SQL.

- [ ] Step 1: Failing test — ingest `biomarker_repo` (current bundle), assert a known function row has `nargs > 0` and `max_nesting > 0` (the fixture's `complex` function has nested loops/ifs). Expect FAIL (columns missing / zero).
- [ ] Step 2: Add columns + bump both version constants + wire the writer.
- [ ] Step 3: Test green. Verify the schema-mismatch path: open a v4 cache → typed re-ingest error (existing mechanism; add an assertion only if a test for the mismatch already exists to extend).
- [ ] Step 4: Commit `feat(facts): persist nesting, nargs, and boolean-conditional metrics (schema v5)`.

## Task 3 — Three new biomarkers in the composite

**Files:** Modify `crates/codelore-lib/src/analyses/code_health.rs`. Test: `crates/codelore-lib/tests/code_health_test.rs`.

**Interfaces — Produces:** smells `deep-nesting`, `many-args`, `complex-conditional` in `code_health_biomarkers_v1`; new weights; updated `STRUCTURAL_SCALE_NO_DRY`.

New weight table (sums to 1.00; preserves current relative ordering; document in the module docstring and `docs/advanced-usage.md`):

| smell | weight |
|---|---|
| complex-method | 0.22 |
| god-class | 0.18 |
| large-method | 0.12 |
| dry | 0.12 |
| shotgun-surgery | 0.12 |
| deep-nesting | 0.10 |
| many-args | 0.07 |
| complex-conditional | 0.07 |

`STRUCTURAL_SCALE_NO_DRY` becomes `" / 0.88"`.

- [ ] Step 1: Failing tests on the REGENERATED fixture (Task 4 provides it — coordinate: write tests against the new bundle's known content): deep-nesting fires on the deeply-nested function's file; many-args on a 7-arg function; complex-conditional on a multi-clause boolean chain; weights sum asserted `1.0` (unit test over the weight CASE — extract weights to a `const SMELL_WEIGHTS: &[(&str, f64)]` so the test can sum it, and generate the SQL CASE from it to keep one source of truth).
- [ ] Step 2: Extend `BIOMARKERS_INSERT` with three more `PERCENT_RANK() OVER (PARTITION BY lang ORDER BY …)` selects: `MAX(max_nesting)`, `MAX(nargs)`, `MAX(bool_ops)` per file (mirroring `file_cx`). Same intensity/co-occurrence mechanics as the existing SQL smells.
- [ ] Step 3: Tests green. Then run the full suite: score-shift fallout in other tests (hotspot/effort/health-trend/cli assertions that pin scores or bands) is EXPECTED — update each pinned value with a comment-free recalculation, verifying each new value is explainable (task report lists every updated pin and why).
- [ ] Step 4: Commit `feat(code-health): deep-nesting, many-args, and complex-conditional biomarkers`.

## Task 4 — Regenerate `biomarker_repo` with content for the new smells

**Files:** Modify `crates/codelore-lib/src/test_support/mod.rs` (docs: new HEAD SHA), replace `data/biomarker-repo.bundle`. Follow the documented recipe: revive the builder from commit `b412e6d`, add three files (`src/nested.rs` — 5-deep nesting; `src/many_args.rs` — fn with 7 params; `src/conditional.rs` — `if` with 4 boolean operators), keep all existing content and dates, rebuild deterministically (build twice, identical HEAD), capture with `--all`, update the module doc SHA.

- [ ] Step 1: Revive + extend builder in a scratch dir (never committed); build twice; HEADs identical.
- [ ] Step 2: Replace bundle; update doc SHA; run every biomarker_repo consumer suite (code_health, effort_exposure, finding_hotspot_overlap, health_trend, refactoring_targets, cli, quality_gates unit) — update pinned expectations where fixture content legitimately changed them (report each).
- [ ] Step 3: Commit `test(fixtures): biomarker_repo exercises nesting, args, and conditional smells`.

**Ordering note:** Tasks 3 and 4 are co-dependent (tests in 3 target the new fixture). Execute as 4-then-3 or as one combined implementer session with two commits in that order.

## Task 5 — `calibration` module: artifact model + interpolation

**Files:** Create `crates/codelore-lib/src/calibration.rs`; add `pub mod calibration;` to lib.rs (between `bands` and `cache`). Test: `crates/codelore-lib/tests/calibration_test.rs`.

**Interfaces — Produces:**

```rust
pub const CALIBRATION_FORMAT_VERSION: u32 = 1;
pub const MIN_LANG_SAMPLE: u64 = 500; // functions; below → language treated as absent
pub const QUANTILE_POINTS: usize = 1001; // q0.000..q1.000 inclusive

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CalibrationArtifact {
    pub format_version: u32,
    pub corpus_vintage: String,      // e.g. "world-2026-07"
    pub generated_at: String,        // RFC3339
    pub repos_included: u32,
    pub repos_attempted: u32,
    pub languages: Vec<LanguageTable>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LanguageTable {
    pub language: String,            // Tier1Language as_str
    pub sample_functions: u64,
    pub strata: Vec<Stratum>,        // v1 world artifact: exactly one stratum
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Stratum {
    pub sloc_min: u64, pub sloc_max: u64,   // stratum bounds; v1: 0..u64::MAX
    pub metrics: Vec<MetricQuantiles>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetricQuantiles {
    pub metric: String,              // "cyclomatic"|"cognitive"|"sloc"|"nargs"|"max_nesting"
    pub quantiles: Vec<f64>,         // len == QUANTILE_POINTS, non-decreasing
}

pub fn load(path: &Path) -> Result<CalibrationArtifact>;      // unknown format_version → Err (caller warns once, proceeds w/o corpus lens)
pub fn embedded_world() -> Option<&'static CalibrationArtifact>; // lazily parsed include_bytes; None if the embedded file is the placeholder
pub fn percentile(art: &CalibrationArtifact, language: &str, metric: &str, value: f64) -> Option<CorpusPercentile>;
pub struct CorpusPercentile { pub p: f64 /*0..=1*/, pub beyond_corpus: bool }
pub fn build_from_observations(vintage: &str, obs: &LangObservations…) -> CalibrationArtifact; // pooling → quantile vectors
pub fn merge(base: CalibrationArtifact, additional: …) -> CalibrationArtifact; // pooled re-quantiling from retained raw pools — see Step design
```

`percentile()`: binary-search the quantile vector, linear interpolation between neighbors; value < q[0] → p=0.0; value > q[last] → `CorpusPercentile { p: 1.0, beyond_corpus: true }`; language sample below `MIN_LANG_SAMPLE` or language/metric absent → `None`.

`--merge` design: exact pooled re-quantiling needs raw observations, which the artifact (quantiles only) doesn't retain. v1 `merge` therefore does **weighted quantile blending** (sample-count-weighted interpolation of the two quantile vectors) and documents it as an approximation in the rustdoc; exactness requires re-running `calibrate` over the union manifest (also documented).

- [ ] Step 1: Failing unit tests: round-trip serde; `percentile` exact-breakpoint / midpoint-interpolation / below-min / beyond-max / unknown-language / unknown-metric / under-sample-floor / unknown-format-version-on-load; quantile-vector monotonicity validated on load (non-monotonic → Err).
- [ ] Step 2: Implement; tests green.
- [ ] Step 3: Commit `feat(calibration): quantile-breakpoint artifact with interpolated corpus percentiles`.

## Task 6 — `codelore calibrate` subcommand

**Files:** Modify `crates/codelore-cli/src/args.rs` (`Command::Calibrate(CalibrateArgs)`: `--repos <manifest.toml>` required, `--output <path>` required, `--merge <existing>` optional, `--vintage <string>` optional w/ date-derived default, `--cache-dir` optional), `crates/codelore-cli/src/main.rs` (`run_calibrate_cmd`), `crates/codelore-lib/src/calibration.rs` (manifest model: `CorpusManifest { repos: Vec<CorpusRepo { url_or_path, sha, languages }> }` + TOML parse — toml crate is already a dependency via thresholds parsing; verify and reuse). Test: CLI round-trip in `crates/codelore-cli/tests/cli_test.rs` over two bundled fixtures.

**Behavior:** for each manifest repo: local path → open directly; URL → `git clone` into a tempdir then `git checkout <sha>` (child processes with detached stdio per the mcp.rs invariant precedent); run standard ingest (`open_or_ingest_with_cache_root`), query per-function raw metrics (`SELECT lang, cyclomatic, cognitive, sloc, nargs, max_nesting FROM complexity_metrics …` joined to language via path extension mapping — reuse the Tier1Language extension logic), pool observations. Per-repo progress line; a failing repo → warn + skip; artifact header records attempted/included. Emit via `calibration::build_from_observations` (+ `merge` when `--merge`).

- [ ] Step 1: Failing CLI test: manifest listing two LOCAL fixture paths (tiny_repo + biomarker_repo clones materialized in the test) → artifact exists, parses, has `languages[rust].sample_functions > 0`, monotone quantiles; `--merge` run over the same artifact keeps sample counts coherent (doubles pooled weight).
- [ ] Step 2: Implement; green; also `codelore calibrate` on a manifest with one unreachable repo → exit 0, `repos_included == attempted - 1`, warning printed.
- [ ] Step 3: Commit `feat(cli): calibrate subcommand builds corpus calibration artifacts`.

## Task 7 — `corpus_percentile` lens in code-health (additive)

**Files:** Modify `crates/codelore-lib/src/analyses/code_health.rs` (`CodeHealthRow` + join), `crates/codelore-lib/src/options.rs` (`pub calibration: Option<PathBuf>`; excluded from cache key via `snapshot.calibration = None` alongside `rows_limit` — calibration never affects ingest; ADD a content digest `calibration_digest` following the `team_map_file` digest pattern so provenance captures it), `crates/codelore-cli/src/args.rs` (`--calibration <path>` on `AnalyzeArgs` + `CheckArgs`) + `main.rs` mapping. Test: extend `code_health_test.rs` + the additivity contract test.

**Interfaces — Produces:** `CodeHealthRow` gains:
```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub corpus_percentile: Option<f64>,   // 0..=1; MAX over per-metric corpus percentiles
#[serde(skip_serializing_if = "std::ops::Not::not", default)]
pub beyond_corpus: bool,
```
Computation (Rust-side, after the existing SQL): for each file, per-language corpus `percentile()` of its `MAX(cyclomatic)`, `MAX(cognitive)`, `MAX(sloc)`, `MAX(nargs)`, `MAX(max_nesting)` (the raw drivers already selected or cheaply added to the existing per-file aggregation); `corpus_percentile = max` of the available ones; `beyond_corpus = any`. Artifact source: `opts.calibration` file if set, else `calibration::embedded_world()`, else `None` for every row (plus ONE deduped stderr notice at the CLI layer only — not in the library).

- [ ] Step 1: Failing tests: with a hand-built two-language test artifact, a file whose cyclomatic sits at the corpus q750 breakpoint → `corpus_percentile == Some(0.75)`; unknown-language file → `None`; beyond-max → `Some(1.0)` + `beyond_corpus`.
- [ ] Step 2: ADDITIVITY CONTRACT test: run code-health twice on `biomarker_repo` — once without calibration, once with — and assert every pre-existing field (`path,cognitive,score,structural_risk,percentile,band`) byte-identical between runs (serialize with the new fields stripped).
- [ ] Step 3: Implement + wire flags; green.
- [ ] Step 4: Commit `feat(code-health): additive corpus-percentile lens`.

## Task 8 — Provenance vintage stamp

**Files:** Modify `crates/codelore-lib/src/provenance/mod.rs` (`Manifest` gains `pub corpus_vintage: Option<String>`, captured from whichever artifact was active). Test: extend the provenance test.

- [ ] Failing test → implement → green → commit `feat(provenance): stamp the active calibration corpus vintage`.

## Task 9 — Surfaces: CLI columns, SPA, MCP

**Files:** Modify `crates/codelore-lib/src/output/csv.rs` + `markdown.rs` (code-health emitters gain a `corpus-pct` column, empty when None — follow the exact existing column style), `spa/js/10_helpers_drawer.js` (~730: add `Corpus percentile` `<dt>/<dd>` rendering `(ch.corpus_percentile*100).toFixed(0)+'%'` when present, `—` otherwise), `output/spa.rs` (no struct change — `CodeHealthRow` already flows), MCP (verify-only: serde propagation covers it; extend `mcp_test` code_health assertion to accept/probe the optional field). Test: emitter unit tests + spa_integration assertion + mcp_test touch.

- [ ] Failing emitter tests → implement → green → commit `feat(output): corpus percentile across CLI, dashboard, and MCP surfaces`.

## Task 10 — `corpus_percentile_max` gate

**Files:** Modify `crates/codelore-lib/src/quality_gates/mod.rs` (`Gates.corpus_percentile_max: Option<f64>` + `pub fn evaluate_corpus_percentile_rows(max: f64, rows: &[CodeHealthRow]) -> Vec<GateViolation>` — violation per file with `corpus_percentile > max`, gate name `"corpus_percentile_max"`), `crates/codelore-cli/src/main.rs` (`evaluate_all_gates`: reuse the already-computed `code_health` rows; if the gate is configured but NO calibration data is active → `verdict: "skipped"` ledger record + `emit_gate_notices` arm `("corpus_percentile_max","skipped") => "skipped — no calibration artifact (embed or pass --calibration)"` — mirror the sidecar gate skip exactly). Test: gate unit tests (fires/passes/boundary/skip) + a check-level CLI test for the skip record.

- [ ] Failing tests → implement → green → commit `feat(check): corpus-percentile ceiling gate with honest skip`.

## Task 11 — Golden test-calibration artifact + determinism

**Files:** Create a tiny committed test artifact `crates/codelore-lib/tests/fixtures/calibration/test.calib.json` GENERATED by running `calibrate` over the bundled fixture repos (tiny+biomarker+coupling); Test: `calibration_test.rs` gains a determinism test — building the artifact twice from the fixtures yields identical bytes (fixture-bundle precedent; requires `generated_at` to be injectable — give `build_from_observations` the timestamp as a parameter, CLI passes now(), tests pass a constant).

- [ ] Generate + commit artifact; determinism test green; commit `test(calibration): golden fixture artifact with byte-determinism guard`.

## Task 12 — World corpus manifest + embedded artifact

**Files:** Create `calibration/corpus.toml` (top-level dir): ~25 permissive-license, active, size-stratified repos per Tier-1 language, each `url + sha + languages` (curate: well-known OSS — e.g. for Rust: ripgrep, serde, tokio, clap…; equivalents per language; the implementer curates 25/lang with license verified from each repo's manifest and pins current default-branch SHAs). Create the embedded artifact: **if a full corpus build is feasible in the execution environment** (network + ~2–4 h), run `codelore calibrate --repos calibration/corpus.toml --vintage world-2026-07 --output crates/codelore-lib/src/calibration/world.calib.json`; **otherwise** embed the Task-11 fixture-derived artifact with vintage `"placeholder-fixtures"` and `embedded_world()` returning `None` for it (vintage-prefix check), so the corpus lens stays absent-but-wired until a maintainer runs the real build — documented in `calibration/README.md` with the exact command. Wire `include_bytes!` + lazy parse in `calibration::embedded_world()`.

- [ ] Manifest curated + committed; embedded path wired + tested both ways (placeholder → None; real/fixture-vintage artifact → Some); commit `feat(calibration): world corpus manifest and embedded artifact wiring`.

## Task 13 — Docs + CHANGELOG

**Files:** Modify `docs/advanced-usage.md` (new section: corpus percentiles — what the number means, artifact format/vintage, `calibrate` usage incl. org-corpus + `--merge` approximation note, the gate; biomarker section updated to nine smells + new weight table + LCOM4 absence stated plainly as current contract), `README.md` (one line in the analyses/health blurb: "corpus-relative percentiles — how your code compares to a reference corpus, or to your own organization's"), `CHANGELOG.md` `[Unreleased]` (entries: corpus percentile lens + calibrate subcommand + three biomarkers/schema v5 + gate + provenance stamp).

- [ ] Docs truthful against final behavior (write AFTER all code tasks; verify claims against the shipped code) → commit `docs: corpus-relative percentiles and the expanded biomarker set`.

---

## Verification (end-to-end)

- `just ci` green at every task boundary and on the final branch.
- Real-CLI dogfood: `codelore analyze --analysis code-health --repo . --format markdown` shows the corpus column (empty or populated per embedded-artifact state); `codelore calibrate` over a two-repo local manifest completes; `codelore check` with `corpus_percentile_max` configured produces pass/fail/skip correctly.
- Additivity contract test present and green (Task 7 Step 2) — the non-negotiable.
- Windows subset consideration: none of the new test binaries need adding to the windows filter (they're not platform-sensitive); `calibrate`'s git-clone path uses detached stdio.
- Docs guard: `git grep -nE "F[0-9]{3}|v0\.[0-9]+" crates/ docs/advanced-usage.md README.md` → no new hits.

## Out of scope

LCOM4 (deferred by validation — no cohesion extraction exists), coverage ingestion, code-cartography, scheduled world-artifact CI rebuilds, exact (non-approximate) `--merge` re-quantiling.
