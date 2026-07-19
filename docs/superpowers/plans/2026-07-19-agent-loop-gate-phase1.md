# Agent-Loop Temporal Gate — Phase 1 (change_context) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the pre-write half of the agent-loop temporal gate: a `[calibration]` section in `.codelore-thresholds.toml` with its precedence chain, and a `change_context` MCP tool serving a per-path temporal briefing (health, hotspot standing, co-change partners, ownership with a departed-owner flag, calibrated risk, recent churn) in compact budgeted text.

**Architecture:** Briefing assembly lives in a new lib module `change_context.rs` (the `enrichment/fact_sheet.rs` precedent: run existing analyses, pick per-path values, render deterministically); the MCP tool is a thin `spawn_blocking` wrapper. The departed-owner primitive is extracted from `knowledge_islands.rs`'s SQL into a batched helper. Calibration resolution is one shared helper applied at every surface that accepts `--defect-calibration`.

**Tech Stack:** Rust workspace, DuckDB, rmcp MCP server, toml/serde.

**Spec:** `docs/superpowers/specs/2026-07-19-agent-loop-temporal-gate-design.md` §4.1, §4.5, §4.6, §5, §6, §7, §11 (items 8-9). Phase 2 (`worktree_changes`, `change_set` engine, `gate_changes`, `codelore gate`) is a separate plan after this phase's PR merges.

## Global Constraints

- Branch `feat/agent-loop-gate` (at 740012b = spec + main incl. trust fixes), append-only: `git log --oneline -1` before committing; NEVER amend/reset; stage only intended files by name (NEVER `git add -A`); NEVER Co-Authored-By; Conventional Commits.
- Gates per commit, pinned `/Users/emrec/.cargo/bin/cargo`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; the task's targeted tests. Toolchain is 1.96-pinned: NO let-chains (`if a && let Ok(x) = …`).
- No `unwrap()`/`expect()` outside tests. No new `#[allow]`. No ticket/plan/version refs or static test counts in code or non-CHANGELOG docs. CHANGELOG `[Unreleased] ### Added` gets one entry per user-visible change.
- **Contract 1 (additive-only):** without a `[calibration]` section and without the new tool being called, every existing command/tool output is byte-identical. `Thresholds::is_empty()` MUST NOT consider `[calibration]` (it is a config selector, not a gate — mirroring the existing `fail_on_degraded` carve-out at quality_gates/mod.rs:220-222), so a thresholds file containing only `[calibration]` still vacuously passes `check`.
- **Contract 2 (determinism):** the rendered briefing is deterministic at the rendered level — fixed field order, fixed rounding, partners sorted by `degree` desc then path asc.
- **Contract 3 (scoring isolation):** `change_context` reads analyses; it never writes to the fact store and never perturbs any analysis output.
- **Token budget (spec §6, pinned by test):** ≤ 150 whitespace-split tokens per requested path in the rendered briefing.
- ENOSPC remedy: remove `/Users/emrec/.cache/cargo-target/debug/incremental`; if the volume is full, `cargo clean -p codelore-lib -p codelore-cli -p codelore-rca`.
- Shell note: bare `grep`/`find` are shadowed — use `/usr/bin/grep`, `/usr/bin/find`, or ripgrep-free Grep/Glob tools.

## Validated seam facts (answer sheet, verified at 740012b)

- `Thresholds` (quality_gates/mod.rs:53-60): `{ gates: Gates, diff: DiffGates }`, both `#[serde(default)]`, `deny_unknown_fields` everywhere; `THRESHOLDS_FILENAME` (:45); `discover()` (:164-170) returns default when absent; `from_path` (:177), `from_text` (:197); `is_empty()` (:204-223) with the `fail_on_degraded` carve-out comment at :220-222.
- `run_check_cmd` (main.rs:966): thresholds load :989-993, vacuous-pass short-circuit :995 (`thresholds.is_empty() && !args.ratchet`), Options literal :1019-1026 with `defect_calibration: args.defect_calibration.clone()`.
- MCP: `CodeLoreServer { repo, defect_calibration, allow_foreign_calibration }` (mcp.rs:150-155); `run_mcp_server` validates the artifact at :710-713 pre-runtime; dispatched from main.rs:60-66. `check_gates` tool discovers thresholds per-call (mcp.rs:461-469) and does NOT thread calibration into its Options today.
- Other Thresholds readers: diff.rs:758-766 (reads `[diff]` only), mcp.rs:461 — both parse the whole struct harmlessly.
- Data sources: `CodeHealthRow { path, cognitive, score, structural_risk, percentile, band, corpus_percentile, beyond_corpus }` via `run_code_health` (code_health.rs:680), calibrated via `active_weights(opts)` at :721; `HotspotRow { path, revisions, cognitive, code_health, hotspot_score, mi, mi_rank, ai_pct }` via `run_hotspots` (hotspots.rs:249), `ORDER BY score DESC, path ASC` so rank = row position (fact_sheet.rs:276-286 precedent) — MUST run `.with_no_row_limit()`; `CouplingRow { entity_a, entity_b, shared, revs_a, revs_b, average_revs, degree, fisher_p }` via `run_coupling` (coupling.rs:397, memoized per-FactsDb); the diff-established significance conventions are `shared >= DEFAULT_MIN_SHARED_REVS (=5, constants.rs:21)` and `fisher_p < DEFAULT_FISHER_SIGNIFICANCE (=0.05, constants.rs:41)` (diff.rs:444-480); per-path top-N precedent `fact_sheet.rs:291-319`.
- Departed-owner: knowledge_islands.rs `author_last_commit` CTE (:123-128), `days_since_main_active` via `DATE_DIFF` (:204), threshold filter `> departed_threshold_days` at :211 (bind :254/270) — the analysis ONLY returns rows past the threshold, so a new un-thresholded helper is required. `ownership_pct` = main author's LoC share ×100. `n_substantial_others` uses `DEFAULT_SUBSTANTIAL_OWNER_THRESHOLD = 0.10` (constants.rs:139). Anchor: `opts.age_time_now` or now-UTC (:222-240). `DEFAULT_DEPARTED_THRESHOLD_DAYS = 90` (constants.rs:93). Departed test fixture precedent: in-test builder `knowledge_islands_test.rs::build_fixture()` (:25-112, Alice departed 2024-03-01 / Bob active) driven with `age_time_now: Some(date!(2026-06-01))`.
- Calibrated risk: NO per-file defect probability exists (fact_sheet.rs:399-402); the per-file calibrated signal is `structural_risk`/`score` under `active_weights`; vintage via `defect_calibration::active_vintage(opts)` (mod.rs:453); `active_weights(opts) -> Result<Option<WeightsAndVintage>>` (mod.rs:421) returns None when `opts.defect_calibration` is None.
- Recent churn convention (cycle_health.rs:188-211): `WHERE co.date >= (SELECT MAX(date) FROM commits) - INTERVAL (?) DAY` binding `i64::from(opts.window_days)`; `{src}` = `lineage::source_table(opts)`.
- MCP tool pattern: 9 tools; `code_health` (mcp.rs:293-318) is the per-path template (Parameters struct :170-176, spawn_blocking, `FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())`, `internal()` mapper :43); `explain_file` (:605-622) is the calibration-threading template (`min_revs: 1`, clones server fields into the closure). mcp_test.rs asserts EXACTLY 9 tools (:163-172) + named list (:175-190) + inputSchema objects (:192-199) → bump to 10 + add `change_context`.
- All 9 tools return JSON — the compact fixed-order text output is a NEW shape (spec-mandated §6).
- Merge/rebase detection: NOTHING exists (zero grep hits for MERGE_HEAD/rebase-merge/rebase-apply/CHERRY_PICK_HEAD in src). Precedent for .git access: gix via `to_thread_local()` (`git_dir()` correct for linked worktrees); GitCliRepo shells `git` via `run_git`; paths_filter.rs:62 joins `.git/info/exclude` directly (NOT worktree-safe — do not copy that shape here).
- Options: `min_revs` default 5 (use 1 for briefing, fact-sheet precedent); `rows_limit` → `.with_no_row_limit()`; `window_days` default 90; `departed_threshold_days` default 90; `age_time_now` anchors departed calc.
- Fixtures: `coupling_repo` = guaranteed co-change pairs (alpha/svc.rs ↔ beta/svc.rs, ≥5 shared revs); `delivery_repo` = 3 authors; NO bundle has a departed author — use the in-test builder precedent.
- Docs: advanced-usage.md `### Tool reference` (:1428) `####` subsections in registration order; `explain_file` entry (:1520-1529) is the template (prose + `Parameters:` bullets + `Cost:` line). CHANGELOG `[Unreleased]` has `### Added` (2 entries) then `### Changed`.
- Token-budget test precedent: none exists — new; home = mcp_test.rs (spawn infra exists); proxy = `output.split_whitespace().count()`.

## File structure

| File | Responsibility |
|---|---|
| `crates/codelore-lib/src/quality_gates/mod.rs` | `CalibrationConfig` section + `resolve_defect_calibration` helper |
| `crates/codelore-lib/src/analyses/knowledge_islands.rs` | batched `owner_activity_for_paths` helper + `OwnerActivity` struct |
| `crates/codelore-lib/src/repo/{mod,gix_repo,git_cli_repo}.rs` | `merge_or_rebase_in_progress()` trait method |
| `crates/codelore-lib/src/change_context.rs` (new) | briefing assembly + deterministic renderer |
| `crates/codelore-cli/src/mcp.rs` | `change_context` tool (10th) |
| `crates/codelore-cli/src/main.rs` | calibration-precedence wiring at analyze/check/explain |
| docs: `docs/advanced-usage.md`, `CHANGELOG.md` | tool reference + `[calibration]` docs + entries |

Task order: 1 ∥ 2 ∥ 3 (independent) → 4 (consumes 2+3) → 5 (consumes 1+4). Execute sequentially (shared CHANGELOG) unless the controller stages CHANGELOG edits carefully.

---

### Task 1: `[calibration]` thresholds section + precedence chain

**Files:**
- Modify: `crates/codelore-lib/src/quality_gates/mod.rs` (Thresholds struct ~:53-60, is_empty ~:204-223, new struct + helper + tests)
- Modify: `crates/codelore-cli/src/main.rs` (run_check_cmd ~:989-1026, `analyze()`'s Options build, `run_explain_file`'s Options build, `run_mcp_cmd` ~:60-66)
- Modify: `crates/codelore-cli/src/mcp.rs` (run_mcp_server ~:705-733)
- Modify: `docs/advanced-usage.md` (thresholds schema section + defect-calibration section), `CHANGELOG.md`

**Interfaces:**
- Produces: `pub struct CalibrationConfig { pub defect_artifact: Option<PathBuf> }` on `Thresholds` as `pub calibration: CalibrationConfig`; `pub fn resolve_defect_calibration(cli_flag: Option<PathBuf>, repo_root: &Path) -> Result<Option<PathBuf>>` in `quality_gates` — Task 5's tool relies on the MCP server already holding the resolved path (this task wires `run_mcp_server`).

- [ ] **Step 1: Failing tests first** in quality_gates/mod.rs's existing `#[cfg(test)]` module:

```rust
#[test]
fn calibration_section_parses_and_does_not_make_thresholds_non_empty() {
    let t = Thresholds::from_text(
        "[calibration]\ndefect_artifact = \"artifacts/defects.calib.json\"\n",
    )
    .expect("parse");
    assert_eq!(
        t.calibration.defect_artifact.as_deref(),
        Some(std::path::Path::new("artifacts/defects.calib.json"))
    );
    // A calibration-only file configures no gates: `check` must keep
    // vacuously passing, exactly like a fail_on_degraded-only file.
    assert!(t.is_empty(), "calibration alone must not enable gates");
}

#[test]
fn calibration_section_rejects_unknown_keys() {
    let err = Thresholds::from_text("[calibration]\ndefect_artefact = \"x\"\n");
    assert!(err.is_err(), "deny_unknown_fields must reject the typo");
}

#[test]
fn resolve_defect_calibration_prefers_cli_flag_over_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join(THRESHOLDS_FILENAME),
        "[calibration]\ndefect_artifact = \"from-section.json\"\n",
    )
    .expect("write thresholds");
    let cli = Some(PathBuf::from("/explicit/flag.json"));
    let resolved = resolve_defect_calibration(cli.clone(), dir.path()).expect("resolve");
    assert_eq!(resolved, cli, "CLI flag wins");
    let fallback = resolve_defect_calibration(None, dir.path()).expect("resolve");
    assert_eq!(
        fallback,
        Some(dir.path().join("from-section.json")),
        "section fills None, relative path joined to repo root"
    );
}

#[test]
fn resolve_defect_calibration_without_section_is_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        resolve_defect_calibration(None, dir.path()).expect("resolve"),
        None
    );
}
```

(quality_gates tests already use `tempfile` via the test-support feature — verify the import pattern in the existing test module and mirror it.)

- [ ] **Step 2:** Run `cargo test -p codelore-lib --features test-support --lib quality_gates` → the four new tests FAIL to compile (struct/fn absent).
- [ ] **Step 3: Implement.** New struct after `DiffGates` and field on `Thresholds`:

```rust
/// The `[calibration]` section: repo-declared analysis calibration, applied
/// wherever the equivalent CLI flag is accepted. This is a config *selector*,
/// not a gate — its presence never enables gate evaluation on its own.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationConfig {
    /// Path to a `defects.calib.json` defect-calibration artifact, relative
    /// to the repo root (absolute paths are used as-is). Overridden by the
    /// `--defect-calibration` CLI flag and the MCP server's startup flag;
    /// absent everywhere means uncalibrated.
    pub defect_artifact: Option<PathBuf>,
}
```

with `#[serde(default)] pub calibration: CalibrationConfig,` on `Thresholds`. Extend the `is_empty()` doc/carve-out comment to name `calibration` alongside `fail_on_degraded` (do NOT add it to the boolean expression). Then the resolver, in quality_gates/mod.rs:

```rust
/// Resolve the effective defect-calibration artifact path for a repo:
/// an explicit flag wins; otherwise the discovered thresholds file's
/// `[calibration] defect_artifact` (relative paths joined to the repo
/// root); otherwise `None` (uncalibrated).
pub fn resolve_defect_calibration(
    cli_flag: Option<PathBuf>,
    repo_root: &Path,
) -> Result<Option<PathBuf>> {
    if cli_flag.is_some() {
        return Ok(cli_flag);
    }
    let thresholds = Thresholds::discover(repo_root)?;
    Ok(thresholds.calibration.defect_artifact.map(|p| {
        if p.is_absolute() { p } else { repo_root.join(p) }
    }))
}
```

- [ ] **Step 4: Wire the four surfaces.** In main.rs, replace `defect_calibration: args.defect_calibration.clone()` with the resolved value at: (a) `run_check_cmd` (reuse the ALREADY-LOADED `thresholds` value there instead of re-discovering — inline the same precedence from `thresholds.calibration` to avoid a double parse; add a one-line comment saying it mirrors `resolve_defect_calibration`); (b) `analyze()`'s Options build; (c) `run_explain_file`'s Options build ((b) and (c) call `quality_gates::resolve_defect_calibration(args.defect_calibration.clone(), &args.repo)?`). (d) `run_mcp_server` (mcp.rs): when the startup flag is `None`, fall back via `resolve_defect_calibration(None, &repo)?` BEFORE the existing artifact validation, so a `[calibration]`-sourced artifact is startup-validated identically to the flag path (a malformed thresholds file fails server startup — config errors are fail-fast per the existing posture).
- [ ] **Step 5: Additive checks.** `cargo test -p codelore-lib --features test-support --lib quality_gates` → pass. Run `codelore check --repo .` locally (this repo's thresholds file has no `[calibration]`) → output unchanged vs. the release binary (spot-diff). Run the explain byte-identity spot check: `explain <path>` without flags, before vs after → identical stdout.
- [ ] **Step 6: Docs + CHANGELOG.** advanced-usage.md: add `[calibration]` to the thresholds schema documentation with the precedence chain sentence; cross-reference from the defect-calibration section. CHANGELOG `[Unreleased] ### Added`: one entry.
- [ ] **Step 7:** fmt + clippy + commit `feat(thresholds): repo-declared defect calibration via a [calibration] section`.

### Task 2: Batched departed-owner helper

**Files:**
- Modify: `crates/codelore-lib/src/analyses/knowledge_islands.rs`
- Test: `crates/codelore-lib/tests/knowledge_islands_test.rs`

**Interfaces:**
- Produces: `pub struct OwnerActivity { pub main_author: String, pub ownership_pct: f64, pub days_since_main_active: i32, pub last_main_author_commit: String, pub n_substantial_others: u32 }` and `pub fn owner_activity_for_paths(db: &FactsDb, opts: &Options, paths: &[String]) -> Result<HashMap<String, OwnerActivity>>` — a map entry exists only for paths with LoC-attributable ownership; a missing entry means **Inconclusive**. No departed-threshold filtering (the caller compares `days_since_main_active` against `opts.departed_threshold_days`).

- [ ] **Step 1: Failing test** in knowledge_islands_test.rs, reusing `build_fixture()` (Alice departed on `alice_owned.txt`, Bob active on `bob_dominates.txt`):

```rust
#[test]
fn owner_activity_for_paths_returns_unthresholded_activity_and_omits_unknown() {
    let fixture = build_fixture();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        age_time_now: Some(date!(2026 - 06 - 01)),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let paths = vec![
        "alice_owned.txt".to_string(),
        "bob_dominates.txt".to_string(),
        "no_such_file.txt".to_string(),
    ];
    let map = owner_activity_for_paths(&db, &opts, &paths).expect("helper");

    let alice = map.get("alice_owned.txt").expect("alice file present");
    assert_eq!(alice.main_author, "Alice");
    assert!(
        alice.days_since_main_active > 800,
        "Alice last committed 2024-03-01; got {}",
        alice.days_since_main_active
    );

    let bob = map.get("bob_dominates.txt").expect("bob file present");
    assert!(
        bob.days_since_main_active < 30,
        "Bob is active; got {}",
        bob.days_since_main_active
    );

    assert!(
        !map.contains_key("no_such_file.txt"),
        "unknown path = no entry = Inconclusive"
    );
    assert_eq!(map.len(), 2, "no extra paths leak into the result");
}
```

(Adjust author-name expectations to the fixture's exact canonical names — read `build_fixture()` first; it uses `Alice <alice@old.com>` / `Bob <bob@active.com>`, and canonical_author may be the name or name+email form — assert on what ingest actually canonicalizes, discovering it via the existing test's assertions.)

- [ ] **Step 2:** Run → FAILS (fn absent).
- [ ] **Step 3: Implement** in knowledge_islands.rs beside `run_knowledge_islands`, reusing its CTE text. Structure: reuse the existing query's `author_last_commit`, per-path author-LoC, main-author, and substantial-others CTEs, but (a) restrict to a bound path list via the `VALUES (?), (?), …` + `Vec<&dyn duckdb::ToSql>` pattern (the marginal_owner_risk/`run_trends` precedent — batched, no per-path loop, no string-interpolated paths), and (b) DROP the `> departed_threshold_days` predicate entirely. Keep the same anchor logic (`opts.age_time_now` or now-UTC) and the same `DEFAULT_SUBSTANTIAL_OWNER_THRESHOLD` for `n_substantial_others`. Empty `paths` short-circuits to an empty map without querying. Extract shared SQL fragments into `const`s if the duplication with `run_knowledge_islands` would otherwise be verbatim-copied blocks (reuse, don't copy — the reviewer will check).
- [ ] **Step 4:** Test passes; existing knowledge_islands tests unchanged and green: `cargo test -p codelore-lib --features test-support --test knowledge_islands_test`.
- [ ] **Step 5:** fmt + clippy + commit `feat(knowledge-islands): batched per-path owner-activity lookup without the departed threshold`.

### Task 3: `merge_or_rebase_in_progress()` on the Repo trait

**Files:**
- Modify: `crates/codelore-lib/src/repo/mod.rs`, `crates/codelore-lib/src/repo/gix_repo.rs`, `crates/codelore-lib/src/repo/git_cli_repo.rs`
- Test: `crates/codelore-lib/tests/differential_repo_test.rs`

**Interfaces:**
- Produces: `fn merge_or_rebase_in_progress(&self) -> bool` on `Repo` (default `false`, matching `is_worktree_dirty`'s opt-out convention). True when any of `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `rebase-merge/`, `rebase-apply/` exists under the repository's git dir.

- [ ] **Step 1: Failing differential tests** in differential_repo_test.rs, following the file's fresh-clone-per-test convention (the dirty-guard tests are the template):

```rust
/// A fresh clone has no merge/rebase/cherry-pick in progress; both backends
/// agree.
#[test]
fn merge_or_rebase_state_clean_on_fresh_clone() {
    let fixture = differential_repo::build();
    let gix = GixRepo::open(fixture.dir.path()).expect("gix open");
    let cli = GitCliRepo::open(fixture.dir.path()).expect("cli open");
    assert!(!gix.merge_or_rebase_in_progress());
    assert!(!cli.merge_or_rebase_in_progress());
}

/// Simulate an in-progress merge by writing MERGE_HEAD into the git dir
/// (the exact artifact `git merge` leaves behind on conflict); both
/// backends must report it and agree.
#[test]
fn merge_or_rebase_state_detects_merge_head() {
    let fixture = differential_repo::build();
    let git_dir = fixture.dir.path().join(".git");
    let head = std::fs::read_to_string(git_dir.join("HEAD")).expect("read HEAD");
    // Any valid-looking sha works; reuse ORIG_HEAD-style content.
    let _ = head;
    std::fs::write(git_dir.join("MERGE_HEAD"), "0".repeat(40)).expect("write MERGE_HEAD");
    let gix = GixRepo::open(fixture.dir.path()).expect("gix open");
    let cli = GitCliRepo::open(fixture.dir.path()).expect("cli open");
    assert!(gix.merge_or_rebase_in_progress(), "gix sees MERGE_HEAD");
    assert!(cli.merge_or_rebase_in_progress(), "cli sees MERGE_HEAD");
}
```

(If the fixture clone's `.git` is a gitfile pointing elsewhere — bundles are cloned, so `.git` should be a real dir — read the layout first and adapt: resolve via `git rev-parse --git-dir` in the test if needed.)

- [ ] **Step 2:** Run → FAILS (method absent).
- [ ] **Step 3: Implement.** Trait (repo/mod.rs), doc: current-contract-only, listing the five state markers and the one-line purpose (loop tools disclose ambiguous-HEAD states honestly). Default `false`. `GixRepo`: `self.inner.to_thread_local()`, use `repo.state()` if the pinned gix exposes an in-progress-operation state API (CHECK docs.rs for gix 0.85 `Repository::state()` — it exists in that generation and returns `Option<state::InProgress>`; prefer it over hand-rolled file checks), else check the five paths under `repo.git_dir()`. `GitCliRepo`: for each of the five markers run `git rev-parse --git-path <marker>` via the existing `run_git` and test existence of the returned path (worktree-correct); or a single `git status --porcelain=v2 --branch`-free approach — prefer the `rev-parse --git-path` loop for simplicity. Both must agree on the five markers.
- [ ] **Step 4:** Tests pass: `cargo test -p codelore-lib --features test-support --test differential_repo_test merge_or_rebase`.
- [ ] **Step 5:** fmt + clippy + commit `feat(repo): expose merge/rebase-in-progress state on both backends`.

### Task 4: `change_context` briefing module (lib)

**Files:**
- Create: `crates/codelore-lib/src/change_context.rs`; register `pub mod change_context;` in `crates/codelore-lib/src/lib.rs` (alphabetical)
- Test: inline `#[cfg(test)]` + `crates/codelore-lib/tests/change_context_test.rs` (new)

**Interfaces:**
- Consumes: Task 2's `owner_activity_for_paths`; Task 3's `merge_or_rebase_in_progress`; `run_code_health`, `run_hotspots`, `run_coupling`, `active_vintage`; constants `DEFAULT_MIN_SHARED_REVS`, `DEFAULT_FISHER_SIGNIFICANCE`.
- Produces: `pub const MAX_BRIEFING_PATHS: usize = 20;` and `pub fn build_change_context<R: Repo>(db: &FactsDb, repo: &R, opts: &Options, paths: &[String]) -> Result<String>` returning the rendered briefing. Errors (`CodeLoreError::InvalidOptions`) on an empty list or more than `MAX_BRIEFING_PATHS` paths, naming the limit (spec §4.1).

**The rendered format (fixed, deterministic — this IS the output contract):** per requested path, in request order, sections separated by one blank line; when `repo.merge_or_rebase_in_progress()`, ONE leading line before everything:

```
note: merge/rebase in progress — briefing reflects committed HEAD history
```

Per-path block, exactly these five lines (line 2-6 indented two spaces), omitting nothing (absent data renders its honest-absence form):

```
crates/codelore-lib/src/cache.rs
  health 67.3 (yellow) · risk 0.42 · calibrated defects-2026-07-15
  hotspot #12 (score 0.67, 23 revs)
  co-change: options.rs (68%, p=0.003) · facts/mod.rs (54%, p=0.011)
  owner: Emre Camdere 82% (sole owner, active 12d ago)
  recent: 4 commits, 310 lines churned in last 90d
```

Field rules (pin in code as the renderer, and in the module doc):
- `health` = `CodeHealthRow.score` 1 decimal + `(band)`; `risk` = `structural_risk` 2 decimals; calibration suffix = `calibrated <vintage>` (from `active_vintage(opts)?`) when calibrated else `uncalibrated`.
- `hotspot #<rank> (score <hotspot_score 2dp>, <revisions> revs)` where rank = 1-based position in the un-row-limited `run_hotspots` ordering; a path absent from hotspot rows renders `not in the hotspot set`.
- `co-change:` top-3 partners filtered by `shared >= DEFAULT_MIN_SHARED_REVS && fisher_p < DEFAULT_FISHER_SIGNIFICANCE`, sorted `degree` desc then partner path asc, rendered `<partner> (<degree 0dp>%, p=<fisher_p 3dp>)` joined by ` · `; none → `co-change: none significant`.
- `owner:` from Task 2's map: `<main_author> <ownership_pct 0dp>% ` + concentration marker `(sole owner…` when `n_substantial_others == 0` else `(shared…`; activity: `, departed <days_since_main_active>d)` when `days_since_main_active > opts.departed_threshold_days as i32` else `, active <days>d ago)`. Missing map entry → `owner: inconclusive`.
- `recent:` from a batched window query in this module (the cycle_health window predicate + `AND ch.path IN (VALUES-bound list)`): `<revs> commits, <churn> lines churned in last <window_days>d`; zero rows → `recent: quiet in last <window_days>d`.
- A path with NO history at all (absent from code_health AND the churn query — new/untracked/typo): the block is exactly two lines: the path + `  no history at HEAD (new or untracked file)`.
- Determinism: all floats use the fixed roundings above; nothing iterates a HashMap into output order (partner sort + request-order blocks).

- [ ] **Step 1: Failing tests.** Renderer unit tests (inline, pure — construct the intermediate per-path data struct directly): grounded assertions on the exact five-line block for a fully-populated path, the `uncalibrated` suffix, the no-history block, the `inconclusive` owner line, the merge-note line, and a **token-budget unit test**: `rendered.split_whitespace().count() <= 150 * n_paths` for a maximally-populated 3-path briefing. Integration test (change_context_test.rs) on `coupling_repo`: build briefing for `["alpha/svc.rs"]` with `min_revs: 1` opts; assert the block contains `co-change:` naming `beta/svc.rs`, a `health` line, and an `owner:` line; assert byte-identical output across two runs (Contract 2). Error tests: empty paths + 21 paths both err naming the limit.
- [ ] **Step 2:** Run → FAIL (module absent).
- [ ] **Step 3: Implement.** Internal struct `PathBriefing { path, health: Option<(f64, String, f64)>, calibration: Option<String>, hotspot: Option<(usize, f64, u32)>, partners: Vec<(String, f64, f64)>, owner: Option<OwnerActivity>, recent: Option<(u32, i64)> }` + `fn assemble(...) -> Result<Vec<PathBriefing>>` + `fn render(briefings: &[PathBriefing], merge_note: bool, opts: &Options) -> String`. `assemble` runs each analysis ONCE for all paths (opts derived as `let opts = opts.with_no_row_limit();` after forcing `min_revs: 1` — mirror fact_sheet.rs:115-120), then per-path lookups. The churn query binds the path list with the `VALUES (?), …` pattern. No FactsDb writes anywhere (Contract 3).
- [ ] **Step 4:** All tests pass: `cargo test -p codelore-lib --features test-support change_context` and `--test change_context_test`.
- [ ] **Step 5:** fmt + clippy + commit `feat(change-context): temporal pre-write briefing assembly with budgeted deterministic rendering`.

### Task 5: MCP `change_context` tool + docs

**Files:**
- Modify: `crates/codelore-cli/src/mcp.rs` (10th tool), `crates/codelore-cli/tests/mcp_test.rs` (count 9→10, new tests), `docs/advanced-usage.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: Task 4's `build_change_context` + `MAX_BRIEFING_PATHS`; the server's resolved `defect_calibration`/`allow_foreign_calibration` (Task 1 made startup resolution include `[calibration]`).

- [ ] **Step 1: Failing tests** in mcp_test.rs: bump the exact-count assertion 9→10 and add `"change_context"` to the named list (run first — the count test FAILS red). Add `mcp_change_context_returns_briefing_for_known_path`: spawn against the fixture the neighboring tools use, call `change_context` with one real path (derive it the way the explain_file test derives its target), assert the text result contains the path, a `health ` line and an `owner:` line, and NO literal `undefined`/JSON braces. Add `mcp_change_context_rejects_empty_and_oversized_path_lists` (0 paths and 21 paths → tool error naming the limit). Add the MCP-level **token-budget test**: request 2 paths, assert `text.split_whitespace().count() <= 300`.
- [ ] **Step 2:** Implement the tool:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChangeContextParams {
    /// Repo-relative paths the caller intends to modify (1-20).
    pub paths: Vec<String>,
}
```

```rust
#[tool(
    name = "change_context",
    description = "Temporal pre-write briefing for files you are about to modify: \
        code-health band, hotspot standing, historically co-changed partners \
        (edit those too), owner concentration incl. a departed-owner flag, \
        calibrated structural risk, and recent churn — compact text, \
        ~150 tokens per file. 1-20 paths. Committed-history view; for \
        gate evaluation of the committed tree use `check_gates`. \
        First call on a cold cache triggers history ingest."
)]
async fn change_context(&self, params: Parameters<ChangeContextParams>) -> Result<String, ErrorData> {
    let repo_path = self.repo.clone();
    let defect_calibration = self.defect_calibration.clone();
    let allow_foreign_calibration = self.allow_foreign_calibration;
    let paths = params.0.paths.clone();
    tokio::task::spawn_blocking(move || {
        let opts = Options {
            repo_path: repo_path.clone(),
            min_revs: 1,
            defect_calibration,
            allow_foreign_calibration,
            ..Options::default()
        };
        let repo = GixRepo::open(&repo_path).map_err(internal)?;
        let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
            .map_err(internal)?;
        change_context::build_change_context(&db, &repo, &opts, &paths).map_err(internal)
    })
    .await
    .map_err(internal)?
}
```

(Match the file's exact import/idiom conventions; `build_change_context` itself enforces the 1-20 limit and its `InvalidOptions` error surfaces through `internal` with the limit named.)

- [ ] **Step 3:** All mcp tests pass: `cargo test -p codelore-cli --test mcp_test`.
- [ ] **Step 4: Docs.** advanced-usage.md: `#### change_context` subsection in the Tool reference (registration order — before or after `explain_file` to match the actual `#[tool_router]` order), using the `explain_file` template shape (prose incl. the honest-absence forms + `Parameters:` bullet for `paths` + `Cost:` line), plus the §4.6 cross-reference sentence to `check_gates`. CHANGELOG `[Unreleased] ### Added`: one entry (the tool + the briefing's fields, current-contract wording).
- [ ] **Step 5:** fmt + clippy + commit `feat(mcp): change_context — temporal pre-write briefing tool`.

---

# Verification (end-to-end, Phase 1)

- Targeted suites per task + fmt/clippy CI-exact.
- **Full CLI integration suite before the final review** (`cargo test -p codelore-cli --test cli_test` AND `--test mcp_test`) — the cross-level-regression lesson.
- Real-MCP smoke on this repo: start `codelore mcp --repo .`, call `change_context` with `crates/codelore-cli/src/main.rs` + `crates/codelore-lib/src/cache.rs`; observe a plausible briefing (main.rs should show red band, high hotspot rank, real co-change partners). Repeat with a `[calibration]` section pointing at a locally-mined artifact → risk line shows `calibrated <vintage>`.
- Byte-identity spot checks: `check`/`explain`/`analyze` outputs unchanged without `[calibration]`; with the section present but gates absent, `check` still vacuously passes.
- Docs guard: `git grep -nE "F[0-9]{3}|v0\.[0-9]+" crates/ docs/advanced-usage.md README.md` → no new hits vs 740012b.
- Whole-branch final review → PR (Phase 1 only) → merge on green. NO release cut unless the user asks. Phase 2 plan cycle starts after merge.
