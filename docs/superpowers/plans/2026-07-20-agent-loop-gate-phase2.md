# Agent-Loop Temporal Gate — Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The verification half of the agent-loop temporal gate: `Repo::worktree_changes()`, a delta-scoped change_set engine (projected code-health delta + cycle delta + coupling-absence findings on the working tree vs HEAD), the `gate_changes` MCP tool, and the `codelore gate` CLI subcommand.

**Architecture:** The engine never re-implements a formula. It re-parses only changed files (pure buffer-level extractors), substitutes their rows into a temp `complexity_metrics_projected` table, and runs the *existing* `run_code_health_scoped` twice — HEAD baseline vs projected — so history-derived terms (churn, author-frag, shotgun-surgery) and cross-file structure (god-class fan-in/out via untouched `imports`, dry) stay frozen at HEAD facts automatically, and calibrated weights, clamps, and scale divisors are inherited byte-for-byte. Cycle delta rebuilds the edge set exactly (drop changed sources + deleted targets, re-extract changed files, re-resolve against the updated live set) and compares cyclic-node *membership*. Verdicts come from the existing `[diff]` keys with their existing semantics plus one new per-file floor.

**Tech Stack:** Rust workspace, DuckDB (temp tables on read-only conns via prepared INSERT — never Appender), gix 0.85 (status platform) + git CLI (porcelain v2), tree-sitter extractors, rmcp MCP server.

## Global Constraints

1. Spec: `docs/superpowers/specs/2026-07-19-agent-loop-temporal-gate-design.md`. Phase 1 shipped (`[calibration]`, `change_context`); this plan is Phase 2 plus three ledgered follow-ups.
2. Four contracts: **additive-only** (every existing command/tool output byte-identical when the new surfaces are unused), **determinism** (rendered gate output byte-identical across runs on an unchanged tree), **dual-backend parity** (differential tests for `worktree_changes`), **scoring isolation** (the engine never writes to the FactsDb fact tables; temp tables + sidecar JSON only).
3. Gates before every commit (pinned `/Users/emrec/.cargo/bin/cargo`): targeted tests, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Toolchain resolves 1.97 but the crate floor is 1.96 — **no let-chains**.
4. No `unwrap()`/`expect()` outside tests; no new `#[allow]`; library errors via `CodeLoreError`, CLI via `anyhow::Context`.
5. No ticket IDs / version numbers / static test counts in code or non-CHANGELOG docs. CHANGELOG `[Unreleased]` gets one entry per user-visible change.
6. Stage only intended files by name; append-only branch (`git log --oneline -1` before every commit; never amend/reset). Untracked PNGs at the worktree root stay untracked.
7. Analysis-phase FactsDb writes: temp tables via `conn.execute`/prepared INSERT only (the at_rev idiom). Never `duckdb::Appender` outside ingest.
8. Token budgets are pinned by tests, whitespace-token counted (`split_whitespace().count()`): `gate_changes` ≤ 80 base + ≤ 40 per finding; delta table top-10 by |delta| with a `(+n more)` tail.
9. NO release cut. Ledger: `.superpowers/sdd13/progress.md`. Separate PR; branch `feat/agent-loop-gate-phase2` off `eb9a339`.

## Validated seam facts (all verified at eb9a339 — cite these, do not re-derive)

**Repo layer** — trait at `crates/codelore-lib/src/repo/mod.rs:11-148`; `read_blob_at(rev, path) -> Result<Option<Vec<u8>>>` (:109, both backends implement), `read_blob_at_head` wrapper (:121), `head_sha` (:47), `tracked_paths_at_head` (:135, blobs-only). `FileChange`/`ChangeType` live at `crates/codelore-lib/src/types.rs:74/83` (NOT repo/types.rs); `repo/types.rs` holds only `TagInfo` and is the home for new repo-layer value types. `is_worktree_dirty` documented contract is UNION-OF-STAGES (repo/mod.rs:50-55). GitCliRepo `run_git` injects `-c core.quotepath=false` (git_cli_repo.rs:41-60); NUL-parse precedent at :312-336. gix workspace features: `["max-performance", "blob-diff"]` + defaults → `status` + `parallel` are ON. gix status chain: `repo.status(gix::progress::Discard)?.untracked_files(gix::status::UntrackedFiles::None).index_worktree_submodules(None).into_iter(Vec::new())?` yields `Item::TreeIndex(gix_diff::index::Change)` (staged: Addition/Deletion/Modification/Rewrite, with `entry_mode`) and `Item::IndexWorktree` (unstaged; with dirwalk off the only reachable variant is `Modification { rela_path, status: EntryStatus, .. }`; `NeedsUpdate` pre-filtered; submodules never yield). Item ordering UNDEFINED; same path can appear in both streams — merge by path. CLI side: `git status --porcelain=v2 -z --untracked-files=no` (v2 carries modes mH/mI/mW for symlink 120000 / gitlink 160000 filtering, `<sub>` field, similarity; rename lines `2 R. … <newpath>\0<origpath>\0`; unmerged as `u` lines). Empirical: an added-then-worktree-deleted file shows `AD` in status but is ABSENT from `git diff HEAD` — union and net genuinely differ. Binary is NOT in status output; content-sniff via the existing 8000-byte NUL heuristic (`BINARY_SNIFF_BYTES`, gix_repo.rs:670-676); do NOT reuse the DB's `'binary'` change_type (it means typechange/unknown from CLI raw letters). Differential mutating-test precedent: private clone via `test_support::differential_repo::build()` (test_support/mod.rs:395-419), mutate with `fs::write` / `git -C … add` / direct `.git` writes (differential_repo_test.rs:584-671); `README.md`, `src/main.rs`, `Cargo.toml`, `src/lib.rs` are tracked.

**Code health** — CH = `crates/codelore-lib/src/analyses/code_health.rs`. Composite SQL `GREATEST(0.0, LEAST(100.0, 100.0*(1.0 - 0.50*structural_risk - 0.30*n_cn - 0.20*n_au)))` (CH:256-258; 0.50/0.30/0.20 hardcoded — calibration tunes only smell weights). `SMELL_WEIGHTS` (CH:100-109) sums to 1.0. ALL smell intensities are population-relative PERCENT_RANKs (per-language file-MAX distributions, CH:343-406/461-515; shotgun over the coupled set CH:411-417) — nothing is single-file recomputable, which is WHY the engine substitutes a temp complexity table and re-runs the real engine instead of projecting by hand. History-derived: shotgun only (CH:63-65). Non-single-file structure: god-class (fan_in/fan_out from whole-repo `imports`, god_classes.rs:56-99) and dry (cross-file clones). `run_clones` walks the CURRENT WORKING TREE (clones.rs:1-7,43+), has NO memo (only coupling is memoised, coupling.rs:397-412) — so dry Δ ≡ 0 at gate time and each scoped run pays a full clone walk unless memoised. Scoring gate: INNER JOIN file_revs HAVING revs >= opts.min_revs (CH:182-189,227-239; DEFAULT_MIN_REVS=5); added files have no `changes` rows → never scoreable. `run_code_health_scoped(db, opts, cx)` (CH:695-699); `HealthScanCtx { complexity_source, imports_source, history_cutoff, include_clones }` (CH:57-69), `head()` = ("complexity_metrics","imports",None,true) (CH:74-81). `CodeHealthRow` (CH:149-169). Biomarker temp tables `code_health_biomarkers_v1` / `coupling_centrality_v1` are session-scoped documented contracts (CH:320-333). n_cn = churn/MAX(churn) OVER () (CH:247); n_au raw HHI complement (CH:190-210,248). include_clones=false divides by `STRUCTURAL_SCALE_NO_DRY = 0.88` or the calibrated equivalent (CH:116,726-744) — both scoped runs MUST share include_clones. Calibration: `active_weights(opts) -> Result<Option<(Vec<(String,f64)>,String)>>` (defect_calibration/mod.rs:421,398), substitution at CH:721-744, inherited automatically by any `run_code_health_scoped` call. Parse seam: `compute_for_file(path: &Path, source: Vec<u8>, lang: Tier1Language) -> Result<Vec<ComplexityEntity>>` (complexity/mod.rs:159-163, pure); `Tier1Language::from_path` (complexity/language.rs:19-29: rs, py/pyi, java, js/jsx/mjs/cjs, ts/tsx); replicate ingest exactly: 2 MiB skip (`DEFAULT_MAX_AST_FILE_BYTES`, constants.rs:76; complexity_head.rs:79), `dedup_entities` keyed (name,start,end) with anonymous renaming (consumer.rs:364+), `f64_to_i32_clamped` for cyclomatic/cognitive INTEGER columns (consumer.rs:431-432). `complexity_metrics` schema: schema_v1.sql:101-113, PK (path,name,rev). FactsDb cache: single `.duckdb` keyed on (repo, head_sha, version, canonical opts, epoch) (cache.rs:31-82); dirty tree: HIT served with warn, MISS ingests in-memory and skips the write (facts/mod.rs:383-425); HIT opens READ-ONLY — temp-table writes work, Appender does not.

**Gates / diff / cycles / MCP** — `Thresholds { gates, diff, calibration }` all `deny_unknown_fields` (quality_gates/mod.rs:56-172); `DiffGates { delta_code_health_min, new_hotspot_max, no_new_cycles, delta_health_min, deny_degrading_verdict }` (:139-159); `is_empty()` ANDs all 10 gate + 5 diff keys, excludes fail_on_degraded + calibration (:222-245). `GateViolation { gate, path, actual, threshold }` (:275-281). `evaluate_diff_gate(&Thresholds, new_hotspot_count, delta_code_health, base_cycles, head_cycles, delta_health_ratio, delta_health_verdict) -> Vec<GateViolation>` (:409-468) — pure; equal-passes boundaries. `check` exit codes: violation → `anyhow::bail!` → **exit 1** (main.rs:24-35,1247-1251); typed errors InvalidOptions→2, Repo/RepoIo/BlobNotFound→3, Analysis→4, Output/Io→5 (error.rs:81-88). `diff` uses exit 4 for should_fail — gate mirrors CHECK (1), not diff. Vacuous pass: main.rs:995-1017 (stderr message unless --quiet, `write_github_output("result","pass")`, exit 0). run_check_cmd calibration inline at main.rs:1022-1030. Ledger `GateRunRecord` mode field doc says reserved for more modes (ledger.rs:36-52); file `repo_cache_dir(...)/gate_runs.jsonl`. `diff`'s `delta_code_health_min` = WHOLE-REPO median (base vs head) over min-revs-filtered hotspot rows (diff.rs:780-835). `compute_coupling_absences(base_coupling, pr_files, min_shared, fisher_p) -> Vec<CouplingAbsence>` (diff.rs:444-480); `CouplingAbsence { touched_file, expected_partner, historical_coupling, fisher_p, historical_shared_revs }` (:111-123); defaults `DEFAULT_MIN_SHARED_REVS=5`, `DEFAULT_FISHER_SIGNIFICANCE=0.05` (constants.rs:21,41). Import graph: `ImportGraph { id_to_path, path_to_id, adj }` (import_graph.rs:25-33); `build_import_graph(db)` memoised (:62-76); **`build_import_graph_from_edges(edges: &[(String,String)]) -> ImportGraph`** public + pure (:84-117; cycle_health already splices with it, cycle_health.rs:34-36); `graph_metrics(&g) -> GraphMetrics { n, ccd, propagation_cost, cycle_count, largest_cycle, cyclic_nodes }` (:406-461; cycle_count = SCCs len≥2 — count comparison has a merge blind spot: merging two cycles DROPS the count). `imports` table: schema_v1.sql:195-207 (`rev, src_path, target, resolved, target_path, kind`); working-tree edits are INVISIBLE to FactsDb; buffer extractor `extract_imports(source: &[u8], lang: ImportLanguage) -> Result<Vec<RawImport>>` (imports/extractor.rs:77), `ImportLanguage::from_path`, `resolve_by_extension(importer_path, target, live_paths) -> Option<String>` (resolver.rs:34-38). MCP: 10 tools; pattern = `#[tool]` method + `Parameters<XParams>` + spawn_blocking + per-call `FactsDb::open_or_ingest_with_cache_root` (mcp.rs:238+); server state `{ repo, defect_calibration, allow_foreign_calibration }` (:151-156); change_context threading precedent :704-729; **check_gates builds `Options { repo_path, ..Default }` with NO calibration fields (:479-482) — the follow-up**. Sidecar cache precedent: enrichment/cache.rs — `cache_key` = SHA-256 hex of content+versions (:78-83), path `repo_cache_dir(cache_root, repo)/enrichment/<key>.json` (:88-90), best-effort read/write, corrupt=miss. change_context `(+n more)` seam: `MAX_PARTNERS=3` (change_context.rs:91), `partners.truncate(MAX_PARTNERS)` at :212 discards the pre-truncation count; `cochange_line` at :336-346; budget test fixture has exactly 3 partners (:532-571).

## Plan-level refinements of the spec (validated rationale — binding for this plan)

1. **`delta_code_health_min` keeps its whole-repo-median semantics** in the gate (same key must never mean two things; the median is computable because both scoped runs return full row sets). The change-scoped sharp tool is the NEW optional `[diff]` key `delta_code_health_min_per_file` (floor on each changed file's projected − baseline score). `codelore diff` does not evaluate the new key; the docs comparison table says so.
2. **`no_new_cycles` in the gate uses cyclic-node MEMBERSHIP**: violation iff any path is cyclic in the projected graph that was not cyclic at HEAD (names the files; catches cycle growth; immune to the merge-count blind spot). Documented divergence from `diff`'s count comparison.
3. **No 100-file engine degrade.** The engine parses ALL changed Tier-1 files (bounded above by full-ingest cost, which the tool already pays on cache miss); the 100-file cap from the spec becomes a RENDER cap: the delta table shows top-10 by |delta| (`(+n more)` tail), JSON carries everything.
4. **Gate exit parity = `check`**: violation → exit 1 via bail; vacuous pass parity; empty change-set → pass with "no working-tree changes to gate".
5. **dry-term honesty**: `run_clones` reads the working tree on both scoped runs, so the dry delta cancels to 0 — documented, not fought. A per-FactsDb clones memo (mirroring the coupling memo) makes the second walk free.
6. **min_revs=1 + no_row_limit** for both scoped runs (change_context precedent): maximizes per-file delta coverage; the gate's median population is therefore all-scoreable-files, not diff's min-revs-5 hotspot set — the delta-of-medians semantics is preserved and the population difference is documented.
7. **Unmerged (conflict) entries** in the change-set are a hard `CodeLoreError::Analysis` ("resolve merge conflicts before gating"); a conflict-free in-progress merge/rebase proceeds with the leading note (Phase 1 form).

---

### Task 1: `Repo::worktree_changes()` on both backends + differential tests

**Files:**
- Modify: `crates/codelore-lib/src/repo/types.rs` (new type), `crates/codelore-lib/src/repo/mod.rs` (trait method), `crates/codelore-lib/src/repo/gix_repo.rs`, `crates/codelore-lib/src/repo/git_cli_repo.rs`
- Test: `crates/codelore-lib/tests/differential_repo_test.rs`

**Interfaces (produces — Tasks 2-5 rely on these exact shapes):**
```rust
// repo/types.rs
/// One tracked path whose content differs from HEAD (staged, unstaged, or both).
/// `kind` is the NET classification vs HEAD; `rename_from` is set on the
/// destination entry when the backend reported a rename (the source appears
/// as its own `Deleted` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeChange {
    pub path: String,
    pub kind: WorktreeChangeKind,
    pub rename_from: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeChangeKind { Added, Modified, Deleted }

// repo/mod.rs — trait method, default = empty (the is_worktree_dirty opt-out convention)
/// Enumerate tracked working-tree changes vs HEAD (union of staged and
/// unstaged, net-classified; untracked files excluded; symlinks and
/// submodule pointers excluded; sorted by path). Errors on unmerged
/// (conflict) entries. Hint quality: backends agree via differential tests.
fn worktree_changes(&self) -> Result<Vec<WorktreeChange>> { Ok(Vec::new()) }
```

Implementation contract (both backends):
1. Enumerate the union of staged (HEAD-vs-index) and unstaged (index-vs-worktree) entries, tracked-only, collecting candidate paths + reported rename sources.
2. Unmerged/conflict entry ⇒ `Err(CodeLoreError::Analysis("unmerged paths in working tree; resolve conflicts before gating".into()))` (gix: `EntryStatus::Conflict`; CLI: porcelain v2 `u` lines).
3. Exclude non-blob modes: gix via `entry_mode` on TreeIndex items and the index entry mode on IndexWorktree items; CLI via porcelain v2 `mH/mI/mW` octal fields (skip 120000 and 160000).
4. Net-classify each candidate uniformly: HEAD-blob presence (`read_blob_at_head(path)?.is_some()` — or a cheaper tree lookup where natural) × worktree-file presence (`fs::metadata` on repo_root/path, is_file): (no, yes) → Added; (yes, no) → Deleted; (yes, yes) → Modified; (no, no) → drop (the empirically confirmed `AD` case).
5. Renames: destination entry gets `rename_from: Some(source)`; source emitted as `Deleted` (skip its own status row if present). gix TreeIndex `Rewrite { source_location, location, .. }`; CLI v2 `2 R. … <newpath>\0<origpath>\0` (NEW first, then source).
6. Sort by path ascending; dedup by path (same path can appear in both gix streams — merge before classification).

- [ ] **Step 1: failing differential tests.** In `differential_repo_test.rs`, add four tests using the private-clone precedent (`test_support::differential_repo::build()`), each asserting `GixRepo` and `GitCliRepo` return the SAME `Vec<WorktreeChange>` AND the expected content:
  - `worktree_changes_empty_on_fresh_clone` — both `Ok(vec![])`.
  - `worktree_changes_detects_staged_and_unstaged_edits` — `fs::write` to `README.md` (unstaged), edit + `git add` `Cargo.toml` (staged), edit + `git add` + edit again `src/lib.rs` (both stages): all three appear once, kind `Modified`.
  - `worktree_changes_detects_delete_and_rename` — `git rm src/main.rs`; `git mv Cargo.toml Cargo2.toml`: expect `Cargo.toml` Deleted, `Cargo2.toml` Added with `rename_from: Some("Cargo.toml")`, `src/main.rs` Deleted.
  - `worktree_changes_drops_add_then_delete_and_untracked` — create + `git add` a new file then `fs::remove_file` it (the AD case → absent); create an untracked file (absent).
- [ ] **Step 2: run to verify failure** — `cargo test -p codelore-lib --features test-support --test differential_repo_test worktree_changes` → all four FAIL (default impl returns empty).
- [ ] **Step 3: implement GixRepo** per the verified call chain (`status(Discard)` → `untracked_files(None)` → `index_worktree_submodules(None)` → `into_iter(Vec::new())`), collecting `Item::TreeIndex` (Addition/Deletion/Modification/Rewrite via `location()`/`source_location`, mode-filtered) and `Item::IndexWorktree` Modification entries (`rela_path`; `EntryStatus::Change` variants Removed/Modification/Type; `IntentToAdd` → candidate; `Conflict` → error), then the shared net-classification. Path bytes via the crate's existing BStr→String conversion conventions.
- [ ] **Step 4: implement GitCliRepo** with `run_git(&["status", "--porcelain=v2", "-z", "--untracked-files=no"])`, NUL-tokenized parse of `1 `/`2 `/`u ` records (v2 rename reads TWO NUL fields), mode filtering, `u` → error, then the same net classification. Write the parser as a private pure function `parse_porcelain_v2(bytes: &[u8]) -> Result<ParsedStatus>` with focused unit tests in-module (record forms above, incl. a rename record and a `u` record).
- [ ] **Step 5: run tests to green**, then full differential suite: `cargo test -p codelore-lib --features test-support --test differential_repo_test` → all pass (28 = 24 pre-existing + 4 new; report exact).
- [ ] **Step 6: gates + commit** — fmt, clippy; stage the four files by name; commit `feat(repo): enumerate tracked working-tree changes on both backends`.

### Task 2: clones memo + projected-health half of the change_set engine

**Files:**
- Modify: `crates/codelore-lib/src/analyses/clones.rs` + `crates/codelore-lib/src/facts/mod.rs` (per-instance memo mirroring the coupling memo at coupling.rs:397-412 / facts/mod.rs:151-158)
- Create: `crates/codelore-lib/src/change_set.rs`; register `pub mod change_set;` in `lib.rs` (alphabetical, beside `change_context`)
- Test: `crates/codelore-lib/tests/change_set_test.rs`

**Interfaces (produces):**
```rust
// change_set.rs
pub struct FileDelta {
    pub path: String,
    pub kind: String,             // "added" | "modified" | "deleted" | "renamed"
    pub baseline_score: Option<f64>,   // HEAD score; None => reason set
    pub projected_score: Option<f64>,  // None => reason set
    pub delta: Option<f64>,            // projected − baseline when both present
    pub baseline_band: Option<String>,
    pub projected_band: Option<String>,
    pub reason: Option<String>,   // honest absence: "new file (no history baseline)",
                                  // "not a Tier-1 source file", "binary content",
                                  // "file exceeds the AST size limit", "deleted at gate time",
                                  // "no code-health row at HEAD"
}
pub struct HealthProjection {
    pub deltas: Vec<FileDelta>,          // sorted |delta| desc (None-delta rows last), tie path asc
    pub baseline_median: Option<f64>,    // whole-repo median over baseline rows
    pub projected_median: Option<f64>,   // same population rule, projected rows
}
pub(crate) fn project_health<R: crate::Repo>(
    db: &FactsDb, repo: &R, opts: &Options, changes: &[WorktreeChange],
) -> Result<HealthProjection>
```

Algorithm (validated — implement exactly):
1. Derive `opts_scan = opts.clone()` with `min_revs = 1` + `.with_no_row_limit()` (change_context precedent).
2. Baseline: `run_code_health_scoped(db, &opts_scan, &HealthScanCtx::head())`.
3. Build the projected temp table via `db.conn()` (read-only-safe):
```sql
CREATE TEMPORARY TABLE complexity_metrics_projected AS
SELECT * FROM complexity_metrics WHERE path NOT IN (SELECT path FROM changed_paths_v1);
```
where `changed_paths_v1` is a temp table of all change-set paths (changed + deleted + rename sources), inserted via prepared `INSERT` (at_rev idiom). Then for each non-deleted change: read working-tree bytes (`std::fs::read`), apply the ingest-parity pipeline — 2 MiB skip, NUL binary sniff (8000 bytes), `Tier1Language::from_path` gate, `compute_for_file`, `dedup_entities`, `f64_to_i32_clamped` — and INSERT rows into `complexity_metrics_projected` with `rev` = the current `repo.head_sha()?`. Files failing a gate get a `FileDelta.reason` instead of rows.
4. Projected: `run_code_health_scoped(db, &opts_scan, &HealthScanCtx { complexity_source: "complexity_metrics_projected".into(), imports_source: "imports".into(), history_cutoff: None, include_clones: true })`. Both runs share `include_clones: true` (scale parity); the clones memo (this task) makes the second walk free; the coupling memo already covers shotgun.
5. Join per changed path: baseline row × projected row → `FileDelta` (bands from the rows); absences → the exact `reason` strings above. Deleted files: baseline side only, `reason: "deleted at gate time"`, no delta. Medians: `median` over each full row set's scores (population = all scored files, min_revs=1 — document divergence from diff's hotspot-set population in the module doc).
6. Determinism: sort as specified with `f64::total_cmp`; no HashMap iteration into output order.

- [ ] **Step 1: clones memo.** Mirror the coupling memo exactly: `pub(crate) fn clones_memo_get/put` on FactsDb (Cell/RefCell single-slot, per-instance), consulted at the top of `run_clones`. Add an in-module test asserting two `run_clones` calls on one FactsDb return identical rows and the second is served from the memo (expose a `#[cfg(test)]` hit counter or assert via the existing pattern used by the coupling memo's tests if one exists — else rely on identical-rows + a `pub(crate)` memo-populated check).
- [ ] **Step 2: failing engine tests** in `change_set_test.rs` on a private `differential_repo::build()` clone (ingest via the standard test path):
  - `modified_file_gets_baseline_and_projected_scores` — append a large deeply-nested function to `src/main.rs` (write actual Rust source in the test), run `project_health` with that one Modified change: both scores present, `projected_score < baseline_score` (complexity strictly worse), delta negative.
  - `unchanged_repo_projects_zero_delta` — pass a Modified change whose working-tree content equals HEAD: delta == Some(0.0) exactly (byte-identical parse ⇒ identical rows ⇒ identical rank).
  - `added_file_reports_no_history_baseline` — a new tracked-in-changeset path: reason `"new file (no history baseline)"`, no delta.
  - `non_tier1_file_reports_reason` — change to `README.md`: reason `"not a Tier-1 source file"`.
- [ ] **Step 3: run to verify failure**, implement per the algorithm, run to green: `cargo test -p codelore-lib --features test-support --test change_set_test`.
- [ ] **Step 4: scoring-isolation guard test** — after `project_health`, assert `SELECT COUNT(*) FROM complexity_metrics` unchanged and no new permanent tables (query `duckdb_tables()` filtering `temporary = false`).
- [ ] **Step 5: gates + commit** `feat(change-set): projected code-health delta via substituted complexity table`.

### Task 3: cycle splice + coupling absences + report assembly + memoization

**Files:**
- Modify: `crates/codelore-lib/src/change_set.rs` (extend), `crates/codelore-lib/src/analyses/coupling.rs` (receives the MOVED absence types), `crates/codelore-cli/src/diff.rs` (imports them from the lib)
- Create: `crates/codelore-lib/src/change_set_cache.rs` OR a `cache` submodule in change_set.rs (mirror enrichment/cache.rs shape; pick the submodule if under ~150 lines)
- Test: `crates/codelore-lib/tests/change_set_test.rs` (extend)

**Layering prerequisite (move, don't copy — the trailer-parser precedent):** `CouplingAbsence` and `compute_coupling_absences` currently live in the CLI crate (`crates/codelore-cli/src/diff.rs:111-123, 444-480`); the lib engine cannot import from the CLI crate. MOVE both into `crates/codelore-lib/src/analyses/coupling.rs` verbatim (struct derives + doc comments intact; `pub`), update `diff.rs` to `use codelore_lib::analyses::coupling::{CouplingAbsence, compute_coupling_absences}`, and re-run `cargo test -p codelore-cli --test cli_test diff` to prove diff's behavior is untouched before building on it.

**Interfaces (produces — Tasks 4-5 consume `ChangeSetReport` verbatim):**
```rust
pub struct ChangeSetReport {
    pub head_sha: String,
    pub merge_in_progress: bool,
    pub changes: Vec<WorktreeChange>,        // as enumerated (sorted)
    pub health: HealthProjection,
    pub base_cyclic_paths: Vec<String>,      // sorted
    pub newly_cyclic_paths: Vec<String>,     // projected-cyclic minus base-cyclic, sorted
    pub coupling_absences: Vec<CouplingAbsence>,
    pub findings: Vec<Finding>,
}
pub struct Finding {
    pub id: String,        // 12-hex prefix of SHA-256 over "kind|path|detail" — stable across runs
    pub kind: String,      // "health-drop" | "newly-cyclic" | "coupling-absence" | "new-file" | "unparseable"
    pub path: String,
    pub detail: String,    // one sentence, deterministic
}
pub fn build_change_set_report<R: crate::Repo>(
    db: &FactsDb, repo: &R, opts: &Options, cache_root: &Path,
) -> Result<ChangeSetReport>
```

Cycle splice (validated three-part rebuild — implement exactly):
1. Baseline: `build_import_graph(db)` → `graph_metrics` → `cyclic_nodes` → paths.
2. Updated live set = `tracked_paths_at_head()` − deleted − rename sources + added/renamed destinations.
3. Projected edges: (a) SQL `SELECT src_path, target_path FROM imports WHERE target_path IS NOT NULL AND src_path NOT IN changed_paths_v1 AND target_path NOT IN deleted_paths_v1`; (b) for each changed non-deleted file with an `ImportLanguage`: `extract_imports(&worktree_bytes, lang)` → `resolve_by_extension(path, target, &live_set)` → edges; (c) re-resolution sweep: `SELECT src_path, target FROM imports WHERE NOT resolved AND src_path NOT IN changed_paths_v1`, resolve each against the updated live set, add newly resolvable edges. Then `build_import_graph_from_edges(&edges)` → `graph_metrics` → projected cyclic paths.
4. `newly_cyclic_paths` = projected − base (set difference, sorted).

Absences: `run_coupling(db, opts)` (memoised) → `compute_coupling_absences(&coupling, &changed_path_set, DEFAULT_MIN_SHARED_REVS, DEFAULT_FISHER_SIGNIFICANCE)` — changed set excludes deleted files' sources? No: include every non-deleted change-set path (a rename destination inherits nothing — document).

Findings assembly (deterministic order: kind asc, then path asc): `health-drop` for every FileDelta with delta < 0; `newly-cyclic` per newly cyclic path; `coupling-absence` per absence row; `new-file` per added file; `unparseable` per reason in {binary, size-limit}. Advisory only — verdicts come from thresholds (Task 4).

Memoization sidecar: key = SHA-256 hex of `head_sha | sorted "path\0content_sha256" lines for all change-set paths (deleted → literal "deleted") | CARGO_PKG_VERSION | "change-set-v1"` — thresholds and calibration are EXCLUDED from the key because the cache stores only MEASURED data; verdicts are always recomputed by consumers (the warm-cache-verdict lesson). Path: `repo_cache_dir(cache_root, repo_path)/change-set/<first 16 hex>.json`, best-effort read/write, corrupt = miss, serde on `ChangeSetReport` (derive Serialize/Deserialize on all report types).

- [ ] **Step 1: failing tests** — `newly_cyclic_detected_when_edit_introduces_cycle` (private clone: write two Rust files importing each other via `mod`/`use` forms the resolver handles — copy an edge form that `resolve_by_extension` resolves for `rs`, verified in its unit tests; assert the new cycle's paths appear in `newly_cyclic_paths` and a `newly-cyclic` finding exists); `absence_fires_for_historical_partner` (edit one file of the fixture's historically-coupled pair if the differential fixture has one — else build on `coupling_repo` with a scratch working-tree edit); `report_is_memoised_by_content` (two builds → identical reports; flip one changed file's content → different cache key, cache dir gains a second entry); `finding_ids_stable_across_runs`.
- [ ] **Step 2: implement** (splice, absences, findings, sidecar), run to green.
- [ ] **Step 3: determinism test** — `report_renders_byte_identical_across_two_builds` on the same mutated clone (serialize both reports to JSON, assert equal).
- [ ] **Step 4: gates + commit** `feat(change-set): cycle splice, coupling absences, findings, sidecar memoisation`.

### Task 4: `codelore gate` CLI subcommand

**Files:**
- Modify: `crates/codelore-cli/src/args.rs` (new `Gate(GateArgs)` variant + struct), `crates/codelore-cli/src/main.rs` (dispatch + `run_gate_cmd`), `crates/codelore-lib/src/quality_gates/mod.rs` (new key + evaluator)
- Test: `crates/codelore-cli/tests/cli_test.rs`

**Interfaces:**
```rust
// args.rs — GateArgs mirrors CheckArgs conventions
pub struct GateArgs { repo, thresholds_file, quiet, format: GateFormat /* Text|Json */,
                      cache_dir, temp_dir, defect_calibration, allow_foreign_calibration }
// quality_gates/mod.rs — DiffGates gains:
pub delta_code_health_min_per_file: Option<f64>,   // include in is_empty(); deny_unknown_fields already covers typos
// new pure evaluator:
pub fn evaluate_gate_thresholds(t: &Thresholds, report: &ChangeSetReport) -> Vec<GateViolation>
```
`evaluate_gate_thresholds` semantics (equal passes, mirroring evaluate_diff_gate): `delta_code_health_min` — fail if `(projected_median − baseline_median) < min` (both medians present; else skipped-with-notice); `delta_code_health_min_per_file` — one violation per FileDelta with `delta < min` (path-level, actual = delta); `no_new_cycles` — one violation naming each newly_cyclic path (or one violation listing them; pick ONE violation per path so `--format json` consumers get rows). `new_hotspot_max` / `delta_health_min` / `deny_degrading_verdict` are NOT evaluated (docs table says diff-only).

`run_gate_cmd` flow (mirror run_check_cmd): thresholds explicit-else-discover → vacuous pass parity (is_empty + same stderr wording with "gate" substituted, exit 0) → resolve calibration inline (check precedent) → open FactsDb via the cached path → `repo.worktree_changes()` → EMPTY change-set ⇒ "✅ codelore gate: PASS (no working-tree changes to gate)" exit 0 → `build_change_set_report` → `evaluate_gate_thresholds` → render text (verdict line, violations in check's exact `  - {gate}: {path} — actual {actual} vs threshold {threshold}` form, findings as advisory lines, delta table top-10 by |delta| with `(+n more files)` tail) or `--format json` (serde ChangeSetReport + violations) → ledger append with `mode: "gate"` (extend the ledger doc comment) → violations non-empty ⇒ `anyhow::bail!` (exit 1).

- [ ] **Step 1: failing CLI tests** in cli_test.rs (fixture repo + scratch thresholds file): `gate_vacuous_passes_without_thresholds`; `gate_passes_on_clean_tree_with_thresholds` ("no working-tree changes"); `gate_fails_on_per_file_floor` (mutate a fixture file to add complexity, thresholds `delta_code_health_min_per_file = 0.0` → exit 1, stderr names the file); `gate_json_shape` (`--format json` parses; has `changes`, `findings`, `violations` keys).
- [ ] **Step 2: implement** args + evaluator (+ its unit tests beside evaluate_diff_gate's) + run_gate_cmd; run to green; then FULL CLI suite `cargo test -p codelore-cli --test cli_test` (regression net).
- [ ] **Step 3: byte-identity spot-check** — `check`/`explain`/`analyze` outputs unchanged on this repo (the additive contract; diff stdout of a debug run vs the pre-branch release binary as in the Phase 1 evidence pattern).
- [ ] **Step 4: gates + commit** `feat(cli): codelore gate — working-tree quality gate with check-parity exit codes`.

### Task 5: `gate_changes` MCP tool + docs + CHANGELOG

**Files:**
- Modify: `crates/codelore-cli/src/mcp.rs` (11th tool), `crates/codelore-cli/tests/mcp_test.rs` (count 10→11 + tests), `docs/advanced-usage.md`, `CHANGELOG.md`

**Tool contract:** name `gate_changes`, NO params (`#[derive(Deserialize, JsonSchema, Default)] struct GateChangesParams {}`), spawn_blocking, threads `defect_calibration` + `allow_foreign_calibration` (change_context precedent :704-729). Returns TEXT: line 1 = verdict (`PASS` / `FAIL — N violation(s)` / `no thresholds configured — advisory only`), optional merge-note line (Phase 1 wording), violations (check's exact row form), findings (one line each: `[{kind}] {path}: {detail}`), delta table top-10 by |delta| (`{path}  {baseline} → {projected}  ({delta:+.1})`) with `(+n more files)` tail, `no working-tree changes` for the empty case. Budgets pinned: base ≤ 80 whitespace tokens; each finding line ≤ 40; test constructs a multi-finding scenario and asserts `count ≤ 80 + 40 * findings.len()`.

- [ ] **Step 1: red** — bump mcp_test tool count to 11 + add `gate_changes` to the expected name list → FAIL.
- [ ] **Step 2: implement the tool**; docs: `#### gate_changes` entry after `change_context` (registration order matches docs order — reviewer convention), Cost line, honest-absence forms, the check/gate/diff comparison table in the Quality-gate section (rows: surface, input, keys evaluated, median population, cycle semantics, exit codes); CHANGELOG `[Unreleased]` Added entries (gate CLI + gate_changes + per-file floor key).
- [ ] **Step 3: tests green** — new tests: `gate_changes_reports_clean_tree`, `gate_changes_flags_working_tree_edit` (mutate the MCP test fixture's clone), `gate_changes_token_budget_holds`; full `cargo test -p codelore-cli --test mcp_test`.
- [ ] **Step 4: live smoke** on this repo (scratch edit in the worktree, spawn the server the way mcp_test does, call gate_changes, capture the briefing for the report; revert the scratch edit).
- [ ] **Step 5: gates + commit** `feat(mcp): gate_changes — working-tree verdict for the agent loop`.

### Task 6: Phase-1 follow-ups — `(+n more)` partners, check_gates calibration, spec refresh

**Files:**
- Modify: `crates/codelore-lib/src/change_context.rs`, `crates/codelore-cli/src/mcp.rs` (:479-482 region), `docs/superpowers/specs/2026-07-19-agent-loop-temporal-gate-design.md`, `docs/advanced-usage.md` (one sentence), `crates/codelore-cli/tests/mcp_test.rs`

- [ ] **Step 1: `(+n more)`** — capture `partners.len()` before `truncate(MAX_PARTNERS)` (change_context.rs:212), thread `partners_total: usize` through `PathBriefing`, append ` (+{n} more)` in `cochange_line` when `total > MAX_PARTNERS`; extend the budget test's maximal fixture to 5 partners so the suffix renders under the ≤150 budget; keep `renders_the_exact_five_line_block` passing (its fixture has 2 partners — unaffected); add `cochange_line_discloses_truncated_partners` unit test; docs sentence "shows up to three; more are disclosed as (+n more)".
- [ ] **Step 2: check_gates calibration** — replace the bare `Options { repo_path, ..Default }` (mcp.rs:479-482) with the change_context threading form (clone both server fields); add mcp_test `check_gates_honors_calibration_section` (thresholds file with `[calibration]` pointing at a nonexistent artifact in the fixture → tool call errors mentioning the artifact — proves threading; startup validation makes valid-path testing redundant).
- [ ] **Step 3: spec refresh** — §4.1: enumerate `health: no code-health row` + the every-feed no-history condition (current-contract wording); §4.4/§6: record the three plan-level refinements (whole-repo median + per-file floor key; membership cycles; render-cap not engine-degrade) as the implemented design.
- [ ] **Step 4: run** change_context inline + integration + full mcp_test; gates; commit `feat(mcp)+docs: partner-count disclosure, check_gates calibration, spec refresh`.

---

## Verification (whole-branch, before the PR)

- Full suites (the cross-level-regression lesson): `cargo test -p codelore-cli --test cli_test`, `--test mcp_test`, `cargo test -p codelore-lib --features test-support` (lib + integration), `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- Real-CLI smoke on this repo: scratch-edit a tracked file → `codelore gate --repo .` (advisory/no-thresholds path AND with the repo's committed thresholds), verify exit codes, revert.
- Contracts: additive byte-identity (check/explain/analyze/diff unchanged), determinism (gate twice on an unchanged dirty tree → byte-identical), parity (differential suite), scoring isolation (guard test).
- Docs guard: `git grep -nE "F[0-9]{3}|v0\.[0-9]+" crates/ docs/advanced-usage.md README.md` — no new hits vs eb9a339.
- Whole-branch final review (most capable model) → Phase 2 PR → merge on green. NO release cut unless the user names it.
