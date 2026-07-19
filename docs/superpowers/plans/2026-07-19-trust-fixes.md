# Trust-Fix Cycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Fix the validated correctness/trust findings from the 2026-07-19 deep review before the flagship cycle — a broken SPA widget, an N+1 query, an OOM-on-large-repos gap, swallowed errors, missing license notices, and documentation drift.

**Architecture:** Independent fixes across the SPA renderer, one analysis SQL, the DuckDB connection layer + a new CLI flag, two HEAD-time analyses, the release workflows, and docs. No shared engine change.

**Tech Stack:** Rust workspace, DuckDB, vanilla-JS SPA, GitHub Actions.

## Global Constraints

- Gates per commit, pinned `/Users/emrec/.cargo/bin/cargo`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; the task's targeted tests. No full workspace suite.
- No `unwrap()`/`expect()` outside tests. No new `#[allow]`. No ticket IDs / plan refs / codelore version numbers / static test counts in code or non-CHANGELOG docs. CHANGELOG `[Unreleased]` gets one entry per user-visible change.
- Append-only branch `feat/trust-fixes` (base `bf453be`): `git log --oneline -1` before committing; NEVER amend/reset; stage only intended files by name (NEVER `git add -A`); NEVER Co-Authored-By.
- **Byte-identical rule (Task 2):** the `marginal_owner_risk` SQL refactor is claimed semantics-preserving, so it MUST be proven byte-identical — capture baseline output on a fixture repo before the change, run twice after, `diff` all three; attach the result to the commit message.
- Root-cause fixes only — no symptom masking. Where a root cause is a broader pattern (the in-band `__summary__` sentinel exists in multiple analyses), fix the specific broken surface in scope and ledger the pattern-level refactor as a follow-up rather than expanding this cycle.
- These are NOT in scope (validation refuted or reclassified them): `effort_exposure` red==yellow (correct per-band SQL, a data coincidence — not a bug); unclamped-negative MI (documented design choice, repo-relative bands); "red band from history factors" (band derives from `structural_risk` only).

## Validated seam facts (verified at bf453be)

- **team_composition:** row `TeamCompositionRow { author, tenure_days, bucket, veteran_breadth_ok, active, commits, files_touched, onboarding_weeks: Option<i64> }` (team_composition.rs:~55-84); `bucket` ∈ {"onboarded","experienced","veteran"} for real rows; the `__summary__` row (author=="__summary__") packs `bucket = "onboarded=X% experienced=Y% veteran=Z%"` and `onboarding_weeks = None`. SPA `renderKnowledgeSurfaces` (10_helpers_drawer.js:1580-1609) reads `tr.commit_share_pct` (line 1593) and `tr.active_authors` (1598,1602) — **neither field exists** — and does not filter the sentinel. Wired from raw `data.team_composition` (00_setup_boot.js:867); serialized `Vec<TeamCompositionRow>` (spa.rs:241) with a doc comment (spa.rs:236-240) that wrongly describes it as "one row per tenure bucket ... with active-author count, commit share." CSV emitter output/csv.rs:~1298-1326 and markdown output/markdown.rs:~1369-1403 null-guard `onboarding_weeks` but do NOT filter `__summary__` (it leaks as a data row).
- **marginal_owner_risk:** marginal_owner_risk.rs — `for (path, band) in &unhealthy` loop (~:113) issues `db.query_row` per file (~:145); the SQL (~:119-143) has a path-independent `active_authors` CTE (~:124-129) recomputed every iteration (rescans changes-lineage⋈commits). O(unhealthy × full-scan).
- **DuckDB:** facts/mod.rs:141-189 opens connections (`open_in_memory`/`open`/`open_with_flags`) with NO `memory_limit`/`temp_directory` PRAGMA anywhere in facts/; only a ReadOnly `Config`. No `--temp-dir`/`temp_dir` in args.rs or Options.
- **Swallowed Err:** clones_head.rs:55 and imports_head.rs:55-58 use `let Ok(Some(code)) = repo.read_blob_at_head(&rel) else {…}`, treating `Err` (object-db failure) as a silent/`debug`-level skip; complexity_head.rs:54-68 is the correct three-arm pattern with `tracing::warn!` on `Err`.
- **mcp.rs:96:** `wt_path.to_str().unwrap()` in `temp_worktree()` — fallible on non-UTF-8 temp path; adjacent line 91 uses `.unwrap_or(".")`.
- **License:** release.yml:~132 tar path is binary-only (4 targets); the Windows zip (~:129) bundles GPL `LICENSE` but no MPL; Containerfile:78 copies binary only. `crates/codelore-rca/LICENSE-MPL` exists on disk; workspace license `GPL-3.0-only` (Cargo.toml:9).
- **Docs count:** `AnalysisName` enum has 56 variants (analysis.rs); docs state "54 analyses" (docs/codebase_analysis.md:24,91; docs/roadmap-v1.x-and-beyond.md:46; docs/ui-roadmap.md:269) and "55" (docs/superpowers/plans/2026-07-15-defect-calibration.md:35 — a historical plan, leave it). README says only "dozens" (fine). CLAUDE.md is gitignored/absent (ignore).
- **args.rs:1-3** module doc: `//! Subcommands: analyze, diff, query, facts, explain, config, doctor, init.` — `query`/`facts`/`config`/`doctor`/`init` are phantom; real Command enum (args.rs:108-165): Analyze, Diff, Completions, Explain, Schema, Profile, Docs, Check, Mcp, IngestSarif, Calibrate, CalibrateDefects.

---

### Task 1: Fix the broken team_composition Knowledge-surfaces widget + sentinel leak

**Files:** `crates/codelore-lib/src/output/spa/js/10_helpers_drawer.js` (renderKnowledgeSurfaces), `crates/codelore-lib/src/output/spa.rs` (doc comment), `crates/codelore-lib/src/output/csv.rs`, `crates/codelore-lib/src/output/markdown.rs`; tests: `crates/codelore-lib/tests/*spa*`, csv/markdown emitter tests; `CHANGELOG.md`.

**Root cause:** the renderer reads fields that do not exist on the row and iterates the `__summary__` sentinel; the emitters leak the sentinel as a data row.

- [ ] **Step 1: Failing/whitespace check first** — capture the current SPA team-composition markup on a multi-author fixture (a fixture with ≥2 authors across tenure buckets — `delivery_repo` or `coupling_repo`) and confirm it renders `undefined` / zero-width bars (documents the bug).
- [ ] **Step 2: Fix the renderer.** In `renderKnowledgeSurfaces`, compute the tenure mix from the REAL per-author rows: filter out `author === "__summary__"`, bucket the remaining authors by their `bucket` field into onboarded/experienced/veteran, compute each bucket's share (count of authors in bucket / total authors) and author count per bucket. Render the three segments and legend from those computed values. Remove every read of `tr.commit_share_pct` and `tr.active_authors`. Keep the existing bucketColors, class names, and `aria-label`. Guard the empty case (all-summary or zero real rows) to render nothing.
- [ ] **Step 3: Filter the sentinel in the emitters.** csv.rs and markdown.rs team_composition writers skip the `author == "__summary__"` row (it is a summary carrier, not a data row) — keeping the existing `onboarding_weeks` null-guard. (If a downstream consumer needs the summary, that is the separate pattern-refactor follow-up; here we stop the leak.)
- [ ] **Step 4: Fix the spa.rs doc comment** (~:236-240) to describe the real shape: one row per author (tenure bucket, active flag, commit count, files touched, onboarding weeks) plus a `__summary__` carrier row. Current-state wording only.
- [ ] **Step 5: Tests.** SPA integration/browser test asserts the team-composition bar renders non-zero segments with bucket names and no literal `undefined` on the multi-author fixture; csv/markdown round-trip tests assert no `__summary__` line in the emitted table. Run the relevant spa + emitter suites.
- [ ] **Step 6:** CHANGELOG Fixed entry; fmt + clippy; commit `fix(spa): render team-composition from real fields and drop the summary sentinel from exports`.

### Task 2: marginal_owner_risk N+1 → single set query (byte-identical)

**Files:** `crates/codelore-lib/src/analyses/marginal_owner_risk.rs`; tests: `crates/codelore-lib/tests/*marginal*` (or wherever it is covered); `CHANGELOG.md`.

- [ ] **Step 1: Byte-identical baseline.** Build release, run `marginal-owner-risk` (csv + json) on a fixture repo with unhealthy files (e.g. `biomarker_repo`) and on this repo, save outputs.
- [ ] **Step 2: Refactor.** Compute the path-independent `active_authors` set once (materialize or CTE evaluated a single time), and replace the per-file `query_row` loop with a single set query that computes every unhealthy file's marginal-owner risk in one `GROUP BY path` pass joined against the one-shot `active_authors`. Preserve the exact risk classification, ordering, and row shape.
- [ ] **Step 3: Prove byte-identical.** Re-run Step 1's commands twice; `diff` new-vs-baseline (must be empty) and new-vs-new (deterministic). Paste the diff result into the commit message.
- [ ] **Step 4:** Existing tests green; add one asserting the set query returns the same rows as a small hand-computed fixture if none exists. fmt + clippy. Commit `perf(marginal-owner-risk): single set query, drops the per-file N+1` with the byte-identical evidence in the body.

### Task 3: DuckDB memory ceiling + spill + `--temp-dir`

**Files:** `crates/codelore-lib/src/facts/mod.rs` (connection open), `crates/codelore-lib/src/options.rs` (field + validate), `crates/codelore-cli/src/args.rs` (flag) + `analyze()`/`check` mapping in main.rs, `crates/codelore-lib/src/constants.rs` (default limit const); `CHANGELOG.md`; `docs/advanced-usage.md`.

- [ ] **Step 1:** Add `PRAGMA memory_limit` and `PRAGMA temp_directory` at every connection-open site in facts/mod.rs (in-memory and file). Default the temp directory to a subdir of the cache root (fall back to the system temp dir when no cache root); default the memory limit to a named `pub const` (a conservative ceiling, e.g. matching the documented ~4 GB envelope — verify the documented figure and cite it in the const rustdoc). The in-memory (`--no-cache`/dirty) path must spill to disk instead of OOM-killing.
- [ ] **Step 2:** Add `--temp-dir <PATH>` CLI flag (Options field `temp_dir: Option<PathBuf>`, validated as a writable dir in `Options::validate`), overriding the default temp directory. Optionally `--memory-limit` if trivial; otherwise leave the const default (YAGNI).
- [ ] **Step 3:** Test: ingest a fixture with an explicit low `memory_limit` and a `temp_dir`, assert it still succeeds (spills rather than errors) and that temp files land under the given dir. Document the flag + spill behavior in advanced-usage.md (this makes the documented envelope real). CHANGELOG Added. fmt + clippy. Commit `feat(facts): bound DuckDB memory with spill-to-disk and a --temp-dir flag`.

### Task 4: Error/panic hygiene (swallowed blob Err + non-UTF-8 unwrap)

**Files:** `crates/codelore-lib/src/analyses/clones_head.rs`, `imports_head.rs`, `crates/codelore-cli/src/mcp.rs`; `CHANGELOG.md`.

- [ ] **Step 1:** In clones_head.rs and imports_head.rs, replace the `let Ok(Some(code)) = read_blob_at_head else {…}` with the three-arm match from complexity_head.rs: `Ok(Some)` → analyze, `Ok(None)` → skip silently ("not tracked at HEAD"), `Err(e)` → `tracing::warn!` (a genuine object-db failure must not masquerade as "not tracked"). Match complexity_head.rs's message style.
- [ ] **Step 2:** mcp.rs:96 — replace `wt_path.to_str().unwrap()` with a graceful path (return a typed error, or lossy-convert with a warning) so a non-UTF-8 worktree temp path cannot panic the server. Follow the adjacent `.unwrap_or(".")` idiom or a `.ok_or_else(...)?` returning the existing error type.
- [ ] **Step 3:** fmt + clippy; a unit test for the mcp path if reachable (else note it). CHANGELOG Fixed. Commit `fix: surface blob-read failures and remove a non-UTF-8 panic path`.

### Task 5: License/MPL/NOTICE in release artifacts

**Files:** `.github/workflows/release.yml`, `Containerfile` (or `container.yml`); create `NOTICE` at repo root if warranted; `CHANGELOG.md`.

- [ ] **Step 1:** The tar packaging step bundles the top-level `LICENSE` (GPL-3.0) AND `crates/codelore-rca/LICENSE-MPL` alongside the binary for all Unix targets; the Windows zip adds the MPL file to its existing bundle; the container COPYs both license files into a `/usr/share/licenses/codelore/` path in the image.
- [ ] **Step 2:** Add a root `NOTICE` if the project lacks one, attributing the vendored MPL `codelore-rca` fork (mirrors the license metadata already in Cargo.toml/Containerfile). Verify YAML parses (`python3 -c "import yaml; ..."`).
- [ ] **Step 3:** CHANGELOG Fixed/Changed (compliance). Commit `build(release): bundle GPL + MPL license notices in every artifact`.

### Task 6: Documentation-honesty pass (analysis count + args.rs phantom subcommands)

**Files:** `docs/codebase_analysis.md`, `docs/roadmap-v1.x-and-beyond.md`, `docs/ui-roadmap.md`, `crates/codelore-cli/src/args.rs` (module doc); a doc-lint test in `crates/codelore-lib/tests/` or `crates/codelore-cli/tests/`; `CHANGELOG.md`.

- [ ] **Step 1:** Replace hard-coded "54 analyses" in the three docs with either the correct current figure derived at authoring time OR a shape descriptor that does not pin a number (prefer the descriptor to avoid re-drift — e.g. "the full registry, enumerated by `codelore --list`/`AnalysisName::all()`"). Leave the historical plan doc (`2026-07-15-defect-calibration.md`) untouched (history).
- [ ] **Step 2:** Add a doc-lint test asserting no tracked doc under `docs/` states a stale hard-coded analysis count — mirroring the existing no-static-test-count discipline. Scope it to catch the "N analyses" pattern against `AnalysisName::all().len()`.
- [ ] **Step 3:** Fix args.rs:1-3 module doc to list only real subcommands (drop `query`/`facts`/`config`/`doctor`/`init`).
- [ ] **Step 4:** fmt + clippy + the new doc-lint test. CHANGELOG Fixed. Commit `docs: single-source the analysis count and correct the CLI subcommand doc`.

---

# Verification / rollout

- Targeted suites per task + fmt/clippy CI-exact via pinned cargo.
- Real-CLI smoke on this repo: regenerate the SPA and confirm the team-composition widget renders real tenure segments (no `undefined`); run `marginal-owner-risk` and confirm identical output to baseline; run an ingest with `--temp-dir` and a low memory limit.
- Docs guard: `git grep -nE "F[0-9]{3}|v0\.[0-9]+" crates/ docs/advanced-usage.md README.md` — no new hits.
- Whole-branch final review → PR → merge on green. NO release cut this cycle unless the user asks (the next release bundles #106 + #107 + this).
- **Ledgered follow-ups (out of scope):** eliminate the in-band `__summary__` sentinel pattern across all analyses (sibling summary object); MCP reach (`list_analyses`/`run_analysis` + row caps + `outputSchema`); `main.rs` god-file split + dispatch de-dup; `codelore init`/`doctor`; vendored/generated-file exclusion (linguist-style); `--query <sql>`; the flagship agent-loop gate (spec ready on `feat/agent-loop-gate`).
