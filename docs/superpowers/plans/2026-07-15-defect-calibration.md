# Own-Repo Defect Calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mine a repository's own fix history (AG-SZZ), validate whether code-health predicted where defects landed, tune the eight smell weights when the evidence clears an honesty floor, and ship it all as an opt-in vintage-stamped artifact that leaves default behavior byte-identical.

**Architecture:** Pure logic (oracle, blame-porcelain parser, AG filter, linkage, metrics, tuning, artifact model) lives in `codelore-lib` as testable functions behind a pluggable line-origin seam; the only subprocess (`git blame`) is invoked from the CLI at artifact-build time (`codelore calibrate-defects`), matching the no-lib-subprocess precedent. Application threads an optional weight set through the single `smell_weights_case` seam; the `defect-validation` analysis only reads artifacts.

**Tech Stack:** Rust workspace, DuckDB fact store (hunks/commits/commit_parents), gix for blob reads, `git blame --porcelain` subprocess (CLI-side), serde artifacts.

**Spec:** `docs/superpowers/specs/2026-07-15-defect-calibration-design.md` — binding for all semantics.

## Global Constraints

1. Gates before EVERY commit: `cargo fmt --all --check` + CI-exact `cargo clippy --workspace --all-targets --all-features -- -D warnings`; full `cargo test --workspace --features test-support,spa` before each task's final commit. No `#[allow]` — extract helpers for line-count lints.
2. No `unwrap()`/`expect()` outside tests. Analysis-phase fact-store writes forbidden; the mining FactsDb is a SEPARATE store built by `calibrate-defects` (in-memory or `--cache-dir`), never the user's cache.
3. **Byte-identical without the flag**: any run without `--defect-calibration` must produce today's exact output — contract-tested (the corpus-lens precedent).
4. The kamei `fix` regex (`kamei/mod.rs::enrich_fix`) is UNTOUCHED — the new oracle is a separate module.
5. No ticket IDs/plan-§IDs/version refs in code or docs; CHANGELOG `[Unreleased]` one entry per user-visible change; Conventional Commits; NEVER `Co-Authored-By`.
6. Worktree-absolute paths only (a separate main-branch checkout exists at /Users/emrec/Projects/playground/codelore/).
7. Spec constants, verbatim: temporal split 60/40 by fix date; grid bounds ±50% relative per weight, projected sum-to-1; acceptance margin +0.02 validation-AUC over defaults; honesty floor = linked defects < 30 OR implicated files < 10 OR margin unmet → defaults kept with recorded reason; vintage default `defects-YYYY-MM-DD`.

## Validated interfaces (source-checked at f532bec; cite, don't re-derive)

- `SMELL_WEIGHTS: &[(&str, f64)]` (8 entries, sums 1.0) at `code_health.rs:96-105`; `smell_weights_case()` at `:117-128` is the ONLY generator of the SQL CASE; consumed once in `run_code_health_scoped` (`:704-713`) via `SQL.replace("{smell_weights_case}", …)`. `STRUCTURAL_SCALE_NO_DRY = " / 0.88"` (`:110`) = `1.0 − dry_weight`, applied when `!cx.include_clones` (`:699-703`) — tuned weights MUST recompute this divisor. Band cutoffs (`bands.rs:27,32`) stay fixed per spec.
- `run_code_health_scoped(db, opts, cx: &HealthScanCtx)` (`:676`); `HealthScanCtx { complexity_source, imports_source, history_cutoff, include_clones }` (`:57-89`). Weights override enters via `Options` (read inside `run_code_health_scoped`), NOT via ctx.
- Historical band scan pattern (health_trend.rs:248-269): per sampled rev — `live_paths_at(db, ts)` → `ingest_complexity_at_rev(db, repo, rev, &live, "cm_at_rev")` (at_rev.rs:23) + `materialize_imports_at_rev(db, &graph, "imports_at_rev")` (at_rev.rs:194) → ctx with sources + `history_cutoff: Some(ts)`, `include_clones: false` → scoped run. `sampled_commits(db)` (`architecture_trend.rs:68`, pub(crate), ≤12 evenly spaced). NOTE: health_trend's `file_series` caps at top-50 paths — the miner runs its own scan for FULL path coverage.
- `hunks(rev, path, old_start, old_lines, new_start, new_lines)`; `old_*` = pre-image deleted side (schema_v1.sql:63-64,84-92; index idx_hunks_rev_path).
- `Options.include_merges` default false (options.rs:71,439); merge commits filtered from the walk when false → `commit_parents` position=1 rows exist only when true. Mining ingest sets it true on its own store; nothing global flips.
- `Repo::read_blob_at(rev, path) -> Result<Option<Vec<u8>>>` (repo/mod.rs:83). NO blame anywhere; NO production lib-side git subprocess (GitCliRepo::run_git is internal to that backend; evidence.rs git calls are test-support-gated). → blame subprocess lives in codelore-cli.
- Calibrate pattern to mirror: `CalibrateArgs` (args.rs:549-576), `Command::Calibrate` (args.rs:158), dispatch (main.rs:54), `run_calibrate_cmd` (main.rs:140-199), artifact write = `serde_json::to_vec` + `create_dir_all(parent)` + `fs::write`. `--calibration: Option<PathBuf>` on AnalyzeArgs (args.rs:499-505) + CheckArgs (args.rs:207-211) → `Options.calibration` (options.rs:182) at main.rs:2023 + :612.
- Cache-key treatment for artifact-path options (options.rs:217-291): snapshot sets the path field `None`, computes `digest_of(path)` content digest, `map.remove("calibration")` + `map.insert("calibration_digest", …)`. Mirror EXACTLY for `defect_calibration`.
- Provenance: `Manifest` field + `capture` stamping at provenance/mod.rs:128-133,175,207 via `calibration::active_vintage(opts)`; mirror as `defect_vintage`.
- `stats.rs`: only `fisher_two_tail_pvalue` (`:53`). No AUC/correlation anywhere — new helpers land here.
- Registry: 55 analyses; recipe = enum variant + `as_str` arm (analysis.rs:228-284) + `registry!` entry (`:317-373`) + CLI dispatch fn (template `dispatch_code_health`, main.rs:2613-2667) + explain tuple (main.rs:1474+) + csv/markdown emitters + cli_test smoke (tiny_repo idiom, cli_test.rs:142-160).
- `Tier1Language::{from_path, as_str}` (complexity/language.rs:6-41): rust/python/java/javascript/typescript. NO reusable line-comment-prefix table exists — build one (rust/java/js/ts → `//`, python → `#`).

---

### Task 1: stats helpers — AUC and precision@k

**Files:** Modify `crates/codelore-lib/src/stats.rs`. Tests in-module.

**Interfaces (produces):**
```rust
/// Area under the ROC curve for binary labels ranked by score, computed via
/// the Mann-Whitney U statistic with midpoint tie handling. None when either
/// class is empty.
pub fn auc(scored: &[(f64, bool)]) -> Option<f64>
/// Of the k highest-scored items (ties broken by stable input order), the
/// fraction labeled positive. None when k == 0 or k > len.
pub fn precision_at_k(scored: &[(f64, bool)], k: usize) -> Option<f64>
```

- [ ] **Step 1: failing tests** (in-module, table-driven, hand-computed): perfect separation → auc 1.0; reversed → 0.0; random-ish 4-element case with one tie → hand-derive via U statistic (e.g. scores [0.9,0.7,0.7,0.1], labels [t,t,f,f] → pairs: pos{0.9,0.7} vs neg{0.7,0.1}: 0.9>both(2) + 0.7 vs 0.7 tie(0.5) + 0.7>0.1(1) = 3.5/4 = 0.875); empty-class → None; precision_at_k on 5 elements k=2 with known top-2; k=0/k>len → None.
- [ ] **Step 2:** run `cargo test -p codelore-lib --lib stats` → FAIL. **Step 3:** implement (sort-based rank with average ranks for ties; no external crates). **Step 4:** pass. **Step 5:** commit `feat(stats): AUC and precision@k helpers`.

### Task 2: defect oracle + artifact model

**Files:** Create `crates/codelore-lib/src/defect_calibration.rs` (+ `pub mod defect_calibration;` in lib.rs, alphabetical). Tests in-module + `crates/codelore-lib/tests/defect_calibration_test.rs` (serde/determinism).

**Interfaces (produces — later tasks rely on these exact names):**
```rust
pub struct OracleConfig { pub extra_patterns: Vec<String> } // Default: empty
/// Pure classifier: conventional `fix:`/`fix(scope):` prefix (case-insensitive)
/// OR word-boundary \b(bug|bugfix|fix(es|ed)?|defect|regression|hotfix)\b,
/// AND NOT a merge (caller passes is_merge) AND NOT a revert (`Revert "` prefix).
/// extra_patterns are additional regexes OR'd in (compiled once via new()).
pub struct DefectOracle { /* compiled regexes */ }
impl DefectOracle {
    pub fn new(cfg: &OracleConfig) -> Result<Self>          // invalid regex → typed error
    pub fn is_fix(&self, message: &str, is_merge: bool) -> bool
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DefectArtifact {
    pub format_version: u32,               // 1
    pub repo_identity: String,             // sha256 hex of canonical repo path (cache.rs::repo_hash_short-style, full 64-char)
    pub head_at_mining: String,
    pub vintage: String,
    pub generated_at: String,
    pub oracle: OracleConfig,
    pub mining: MiningStats,               // fixes_found, links_found, files_blamed, lines_considered, lines_dropped_cosmetic, blame_failures, pure_addition_fixes
    pub validation: ValidationMetrics,     // band_table: [(band, defect_changes, share)], auc_default: Option<f64>, precision_at_10/at_red: Option<f64>, implicated_files, linked_defects, sample_dates: Vec<String>, excluded_no_data: u32
    pub weights: Vec<(String, f64)>,       // 8 entries, tuned or default
    pub tuning: TuningDecision,            // Applied { auc_train, auc_validation_default, auc_validation_tuned } | DefaultsKept { reason: String, .. }
}
pub fn save(artifact: &DefectArtifact, path: &Path) -> Result<()>   // serde_json::to_vec_pretty? NO — compact to_vec, calibrate precedent
pub fn load(path: &Path) -> Result<DefectArtifact>                  // version check → typed error naming versions
pub fn check_repo_identity(art: &DefectArtifact, repo_path: &Path, allow_foreign: bool) -> Result<()>
```

- [ ] **Step 1: failing tests.** Oracle table: `"fix: null deref"`→true; `"Fix(parser): …"`→true; `"bugfix for #12"`→true; `"regression in DSM"`→true; `"prefix bugfixes"`→true (word boundary on bugfix); `"affix labels"`→false; `"patch bump"`→false (patch NOT in the strict set); `"Revert \"fix: x\""`→false; merge flag true→false regardless; extra pattern `"JIRA-\\d+"` matches `"JIRA-77 crash"`. Artifact: roundtrip; determinism (two saves byte-identical); version-mismatch load error; identity check pass/fail/override.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** pass. **Step 5:** commit `feat(defect-calibration): fix-commit oracle and artifact model`.

### Task 3: AG-SZZ engine (pure core + porcelain parser)

**Files:** Create `crates/codelore-lib/src/defect_calibration/szz.rs` (submodule of Task 2's module — convert `defect_calibration.rs` to `defect_calibration/mod.rs` in this task). Tests in-module + integration in `tests/defect_calibration_test.rs`.

**Interfaces (produces):**
```rust
/// Pluggable line-origin seam (roadmap's "pluggable SZZ"). Given a file at a
/// revision, returns for each requested 1-based line the commit that last
/// introduced it. The production impl (CLI) shells `git blame --porcelain`;
/// tests use an in-memory fake.
pub trait LineOriginSource {
    fn origins(&self, rev: &str, path: &str, lines: &[u32]) -> Result<Vec<(u32, String)>>;
}
/// Parse `git blame --porcelain` output into (line_number, commit) pairs —
/// pure function so the parser is unit-testable without git.
pub fn parse_blame_porcelain(output: &str) -> Result<Vec<(u32, String)>>
/// A fix commit's deleted ranges per file, straight from the hunks table.
pub fn deleted_ranges(db: &FactsDb, fix_rev: &str) -> Result<Vec<(String /*path*/, u32 /*start*/, u32 /*lines*/)>>
    // SELECT path, old_start, old_lines FROM hunks WHERE rev = ? AND old_lines > 0 ORDER BY path, old_start
pub struct SzzLink { pub defect_rev: String, pub fix_rev: String, pub path: String }
/// The engine: for each fix, blame its deleted pre-image lines at the fix's
/// FIRST PARENT, AG-filter cosmetic lines, discard candidates not older than
/// the fix, emit links + stats.
pub fn link_defects<R: Repo>(
    db: &FactsDb, repo: &R, origin: &dyn LineOriginSource,
    fixes: &[(String /*rev*/, String /*parent_rev*/, String /*date*/)],
    commit_dates: &HashMap<String, String>,
) -> Result<(Vec<SzzLink>, MiningStats)>
/// AG filter: a line is cosmetic when, reconstructed at `rev` via read_blob_at,
/// it is blank or starts (after trim) with the language's line-comment prefix.
/// Unknown language / unreadable blob → NOT cosmetic (conservative).
pub fn is_cosmetic_line(content: &str, lang: Option<Tier1Language>) -> bool
pub fn line_comment_prefix(lang: Tier1Language) -> &'static str  // rust/java/js/ts → "//", python → "#"
```
Documented limitation (rustdoc + spec-aligned): block-comment interiors that don't carry the prefix are not filtered — the honest first rung; counted lines appear in `mining.lines_considered`.

- [ ] **Step 1: failing tests.** (a) porcelain parser on a captured literal (embed a small real `git blame --porcelain` output as a raw string; assert line→sha pairs, incl. a repeated-commit group where subsequent lines omit headers). (b) `is_cosmetic_line` table: `""`→true, `"   "`→true, `"// note"`+Rust→true, `"# note"`+Python→true, `"# note"`+Rust→false, `"let x = 1; // t"`→false, unknown lang→false. (c) `link_defects` with a FakeOrigin (HashMap-backed): two fixes — one whose deleted lines map to an older commit A (→ link), one mapping to a commit NEWER than the fix (→ discarded, clock-skew guard); a cosmetic line mapping to B (→ dropped, counted); a blame failure path (FakeOrigin errors for one file → skip-with-log, blame_failures=1, other files still processed).
- [ ] **Step 2:** FAIL. **Step 3:** implement (module split; keep functions small). **Step 4:** pass. **Step 5:** commit `feat(defect-calibration): AG-SZZ linkage engine behind a pluggable line-origin seam`.

### Task 4: historical band scan + validation metrics + weight tuning

**Files:** Create `crates/codelore-lib/src/defect_calibration/validate.rs`. Tests in-module + `tests/defect_calibration_test.rs`.

**Interfaces:**
- Consumes: Task 1 `stats::{auc, precision_at_k}`, Task 3 `SzzLink`, health_trend's at_rev pattern (`sampled_commits`, `ingest_complexity_at_rev`, `materialize_imports_at_rev`, `run_code_health_scoped` — make `sampled_commits` pub(crate)-reachable; it already is within the lib).
- Produces:
```rust
/// Uncapped per-sample band maps: for each of ≤12 sampled revs, path → band.
/// Same at_rev machinery as health_trend but NO top-50 cap.
pub fn band_history<R: Repo>(db: &FactsDb, repo: &R, opts: &Options)
    -> Result<Vec<(String /*date*/, HashMap<String, String /*band*/>)>>
/// Band table + AUC + precision@k against defect labels.
pub fn validate(links: &[SzzLink], commit_dates: &HashMap<String,String>,
                bands: &[(String, HashMap<String,String>)],
                head_health: &[CodeHealthRow]) -> ValidationMetrics
    // label set: files in ≥1 link; band-at-defect = nearest sample at-or-before the
    // defect commit's date (else earliest sample, counted in excluded_no_data when absent);
    // auc/precision over head_health structural_risk vs labels
/// Constrained deterministic coordinate search per the spec's Unit D.
/// `intensities`: per-file 8-smell intensity vectors (SMELL_WEIGHTS order);
/// `train`/`validation`: (path, label) splits by fix date (60/40).
/// Returns the chosen weights + the decision with both AUCs.
pub fn tune_weights(
    intensities: &HashMap<String, [f64; 8]>,
    train: &[(String, bool)],
    validation: &[(String, bool)],
    defaults: &[(String, f64)],
) -> (Vec<(String, f64)>, TuningDecision)
```
**Tuning implementation note (design decision, state in rustdoc):** re-scoring per candidate weight set needs per-file per-smell intensities — expose them by making the miner capture the `code_health_biomarkers_v1` temp rows once (`SELECT path, smell, intensity FROM code_health_biomarkers_v1` immediately after a HEAD `run_code_health_scoped` call, same connection/session) into `HashMap<String, [f64;8]>`; then each candidate's risk = Rust-side `Σ wᵢ·intensityᵢ` clamped to 1.0 (mirrors the SQL formula, unit-tested for parity against a real run's structural_risk on the fixture). Grid: each weight ∈ {default×0.5, ×0.75, ×1.0, ×1.25, ×1.5}, all 5⁸ combos infeasible → coordinate descent, 2 passes over the 8 weights, each pass trying the 5 steps for one weight (others fixed), projecting sum-to-1 after each acceptance; deterministic order = SMELL_WEIGHTS order. Acceptance and honesty floor per Global Constraint 7.

- [ ] **Step 1: failing tests.** validate(): constructed links + bands + head rows with hand-computed band table and AUC. tune_weights(): synthetic intensities where smell #1 perfectly separates labels → tuning shifts weight toward it and improves validation AUC ≥ margin (Applied); a case with 20 defects → DefaultsKept("fewer than 30 linked defects"); a case where tuned ≤ default+0.02 on validation → DefaultsKept(margin). Parity test: Rust-side risk formula equals SQL structural_risk for the biomarker fixture (tolerance 1e-9).
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** pass; also `cargo test -p codelore-lib --features test-support --test health_trend_test` (untouched machinery stays green). **Step 5:** commit `feat(defect-calibration): historical band scan, validation metrics, constrained weight tuning`.

### Task 5: `codelore calibrate-defects` subcommand

**Files:** Modify `crates/codelore-cli/src/args.rs` (Command variant + `CalibrateDefectsArgs { repo: PathBuf (default "."), output: PathBuf (required), vintage: Option<String>, window_days: Option<u32>, allow_dirty: bool }`), `crates/codelore-cli/src/main.rs` (dispatch + `run_calibrate_defects_cmd` + `GitBlameOrigin` — the production `LineOriginSource` shelling `git -C <repo> blame -w --porcelain -L <ranges> <parent> -- <path>` with detached stdio, batching ranges per file). Test: `crates/codelore-cli/tests/cli_test.rs` e2e.

**Flow (complete):** open repo (GixRepo) → build a MINING FactsDb (in-memory; `Options { include_merges: true, ..Default }` — full history) → collect `(rev, first_parent, date, message, is_merge)` from commits+commit_parents SQL → oracle → fixes → `link_defects` with `GitBlameOrigin` → `band_history` → HEAD `run_code_health_scoped` + captured biomarker intensities → `validate` → `tune_weights` → assemble `DefectArtifact` (vintage default `defects-` + generated_at[..10]; repo_identity from canonicalized path sha256; head_at_mining = repo.head_sha()) → `save`. Progress lines to stderr per phase (calibrate's eprintln idiom). Fixes with no parent (root commits) skipped + counted.

- [ ] **Step 1: failing e2e** (cli_test.rs): build a PLANTED fixture repo inline (dated commits: A introduces `src/lib.rs` with a "buggy" line; B unrelated churn in another file; C is `fix: remove buggy line` deleting A's line; D a comment-only reformat later "fixed" by E `fix: tidy` — E's candidate must be AG-filtered so E yields no link). Run `calibrate-defects --repo <fixture> --output <tmp>/defects.calib.json`; assert exit 0, artifact parses, `mining.fixes_found == 2`, links contain exactly (defect=A, fix=C, path=src/lib.rs), tuning = DefaultsKept (floor: <30 defects), band_table present.
- [ ] **Step 2:** FAIL (unknown subcommand). **Step 3:** implement. **Step 4:** e2e passes; real-CLI run on THIS repo (`--output <scratch>/defects.calib.json`) — paste fixes_found/links_found/band table/n's + the tuning decision in the report. **Step 5:** commit `feat(cli): calibrate-defects — mine, validate, and tune from the repo's own fix history`.

### Task 6: opt-in application — weights threading + provenance + contract

**Files:** Modify `crates/codelore-lib/src/analyses/code_health.rs` (smell_weights_case(weights), structural-scale recompute, Options-read), `crates/codelore-lib/src/options.rs` (`pub defect_calibration: Option<PathBuf>` + canonical_json digest treatment mirroring `calibration` EXACTLY per options.rs:240,271,288-291), `crates/codelore-cli/src/args.rs` (`--defect-calibration` on AnalyzeArgs + CheckArgs + `--allow-foreign-calibration` on both), `crates/codelore-cli/src/main.rs` (Options mapping at :2023/:612-region), `crates/codelore-lib/src/defect_calibration/mod.rs` (`pub fn active_weights(opts) -> Result<Option<(Vec<(String,f64)>, String /*vintage*/)>>` — loads artifact, identity-checks vs opts.repo_path honoring allow_foreign), `crates/codelore-lib/src/provenance/mod.rs` (`defect_vintage: Option<String>` field + capture stamping, mirroring corpus_vintage at :128-133,175,207). Tests: `tests/code_health_test.rs` + `tests/provenance_test.rs`.

**Weight threading (exact, from validation):** `smell_weights_case()` → `smell_weights_case(weights: &[(String, f64)])` (default path passes SMELL_WEIGHTS converted); `run_code_health_scoped` resolves `let (weights, dry_w) = active_weights(opts)?.unwrap_or(defaults)`; `structural_scale` computed as `format!(" / {}", 1.0 - dry_w)` when `!include_clones` (replacing the const usage; keep the const + its invariant test for the default path documentation). Foreign-repo error surfaces before any scoring.

- [ ] **Step 1: failing tests.** (a) CONTRACT: run code-health twice on the biomarker fixture — no flag vs flag-with-artifact-carrying-DEFAULT-weights → byte-identical rows (proves plumbing is inert at defaults); (b) artifact with a shifted weight set → structural_risk/score/band change in the hand-predicted direction for a known file; (c) foreign artifact (wrong repo_identity) → typed error; with allow_foreign → applies; (d) provenance manifest carries defect_vintage iff active (extend provenance_test the way corpus_vintage is tested); (e) options canonical_json: path dropped, digest present (mirror the existing calibration digest test).
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** pass + full workspace suite. **Step 5:** commit `feat(code-health): opt-in defect-calibrated smell weights with provenance vintage`.

### Task 7: `defect-validation` analysis + docs

**Files:** Modify `crates/codelore-lib/src/analysis.rs` (variant `DefectValidation`, `"defect-validation"`, registry — #56), `crates/codelore-lib/src/analyses/mod.rs` + Create `crates/codelore-lib/src/analyses/defect_validation.rs` (`DefectValidationRow { metric: String, value: String }` rows flattened from the artifact's ValidationMetrics + TuningDecision: band table rows, auc, precision rows, n rows, weights_source row, vintage row), `crates/codelore-cli/src/main.rs` (dispatch + explain tuple), `output/csv.rs` + `output/markdown.rs` emitters (metric,value shape — mirror architecture-metrics'), docs (`docs/advanced-usage.md` new section: calibrate-defects + --defect-calibration + defect-validation, honesty framing verbatim from spec; `README.md` analysis-table row; `calibration/README.md` sibling-artifact note), CHANGELOG `[Unreleased]` entries. Test: analysis test (reads a synthetic artifact via opts.defect_calibration → exact rows; without artifact → zero rows + stderr hint) + cli smoke.

- [ ] **Step 1:** failing analysis test + smoke. **Step 2:** FAIL. **Step 3:** wire all touch points (registry recipe per validated interfaces). **Step 4:** pass; registry guard tests green; docs guard `git grep -nE "F[0-9]{3}|PAR-[0-9]|Task-[0-9]" crates/ docs/advanced-usage.md README.md` → no new hits. **Step 5:** real-CLI: `defect-validation` against the Task-5 artifact from this repo — paste output. Full gates. Commit `feat(analyses): defect-validation — the repo's own health-vs-defects evidence`.

---

## Verification (whole-plan)

- Full gates on final tree; workspace suite green.
- Real-CLI end-to-end on THIS repository: `calibrate-defects` → artifact (rich fix history exists) → `defect-validation` report → `analyze --analysis code-health --defect-calibration <artifact>` (weights applied or defaults-kept per the artifact's decision — either way provenance carries the vintage).
- Contract spot-proof: default run byte-identical with and without the feature compiled-in changes.
- Determinism: two `calibrate-defects` runs on the same history → byte-identical artifacts.

## Out of scope (spec) — do NOT implement

LLM enrichment (second Phase-3 track, own spec); Neural-SZZ/SmartCommit; gate integration of validation metrics; tuning band cutoffs or composite churn/ownership weights; kamei `lt` un-stubbing.
