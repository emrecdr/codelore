# Fast-Follow Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate the dormant defect-evidence dossier section on the `explain <path>` CLI and MCP `explain_file` surfaces, narrow the working-tree dirty guard to tracked modifications, correct the tuning honesty-floor reason wording, and refresh the README hero screenshot.

**Architecture:** Everything except the screenshot is flag/threading work over shipped, tested library code: `enrichment/fact_sheet.rs::defect_evidence_section` is gated only on `Options::defect_calibration`, so populating that field at the two dossier call sites (CLI explain, MCP tool) makes the section reachable with zero library changes. The dirty-guard fix changes `Repo::is_worktree_dirty` semantics in both git backends simultaneously, pinned by the existing differential test.

**Tech Stack:** Rust workspace, clap, gix + git-CLI dual backends, rmcp MCP server.

## Global Constraints

- `just ci` gates every commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo deny check`, `cargo test --workspace --all-features`. Use the pinned `/Users/emrec/.cargo/bin/cargo`.
- No `unwrap()`/`expect()` outside tests. No new `#[allow]` — fix root cause.
- No ticket IDs, plan references, version numbers, or test counts in code/docs. CHANGELOG `[Unreleased]` gets one entry per user-visible change.
- NEVER `Co-Authored-By: Claude`. Conventional Commits. Append-only branch: never amend/reset; run `git log --oneline -1` before committing.
- Contract (binding, from the enrichment spec): without any new flag, all outputs stay byte-identical. The new flags are additive-only.
- Do NOT rename serialized fields or fact keys (`ValidationMetrics::linked_defects`, fact key `"linked_defects"`, artifact JSON shape) — format_version 1 compatibility. Prose-only wording changes in Task 4.
- Run tests needing lib fixtures with `--features test-support`.

## Validated seam facts (answer sheet, verified at 08c999b)

- `ExplainArgs` (`crates/codelore-cli/src/args.rs:246-278`): fields `topic, repo, llm, llm_refresh, cache_dir`. No calibration fields yet.
- `CheckArgs` carries the canonical flag pair at `args.rs:225-235`: `#[arg(long)] pub defect_calibration: Option<PathBuf>` + `#[arg(long)] pub allow_foreign_calibration: bool` (AnalyzeArgs duplicates it at `args.rs:557-567`).
- `run_explain_file` (`crates/codelore-cli/src/main.rs:2300-2318`) builds `Options { repo_path, min_revs: 1, ..Default }` — the assignment site.
- `Options` fields exist: `options.rs:190` `defect_calibration: Option<PathBuf>`, `:195` `allow_foreign_calibration: bool`. `Options::validate()` has no calibration rules (load-time validation by design).
- `defect_evidence_section(opts)` (`fact_sheet.rs:403-437`) already loads + identity-checks + emits the section; called from `FileFactSheet::build` at `:150`. No library change needed.
- `defect_calibration::load(path)` hard-errors on IO/parse/format-version; `check_repo_identity(&artifact, repo_path, allow_foreign)` needs only the repo path; `save(artifact, path)` and `repo_identity(repo_path)` are `pub` (`mod.rs:295`, `:362`). `DefectArtifact` fields: `format_version, repo_identity, head_at_mining, vintage, generated_at, oracle: OracleConfig, mining: MiningStats, validation: ValidationMetrics, weights: Vec<(String, f64)>, tuning: TuningDecision` (`mod.rs:259`).
- MCP: `CodeLoreServer { repo: PathBuf }` (`mcp.rs:141-145`), `run_mcp_server(repo: PathBuf)` (`mcp.rs:683`), called from `run_mcp_cmd` (`main.rs:60-62`). `McpArgs` has only `--repo` (`args.rs:167-173`). `explain_file` tool builds `Options { repo_path, min_revs: 1, ..Default }` at `mcp.rs:603-607`. `ExplainFileParams` stays unchanged.
- Dirty guard: `ensure_mining_tree_clean` (`main.rs:641-666`) → `Repo::is_worktree_dirty`. Three production callers total: `main.rs:648` (calibrate-defects guard) and `facts/mod.rs:296`/`:320` (cache-hit warn + cache-write skip). All three protect HEAD-time metrics/mining computed over `tracked_paths_at_head()` only — untracked files cannot affect any of them, so tracked-only is correct for every caller (global semantic change, not a special case).
- `GixRepo::is_worktree_dirty` (`gix_repo.rs:254-282`) uses the unified `into_iter(Vec::new())` stream (includes untracked via dirwalk — deliberate at the time, comment at `:260-266`). `GitCliRepo` (`git_cli_repo.rs:230-240`) uses `git status --porcelain` (includes `??`). Trait doc at `repo/mod.rs:49-62`. Differential test `is_worktree_dirty_matches_on_fresh_clone` (`differential_repo_test.rs:541`).
- Floor reason string: `validate.rs:496` `"fewer than 30 linked defects"`. Exact-string asserts: `validate.rs:792` and `cli_test.rs:2924`. Related prose: module doc `validate.rs:23`, `MIN_LINKED_DEFECTS` doc `:308-311`, `linked_defect_count` doc `:425-437` (counts (defect, file) incidences, NOT deduplicated — hence the wording fix). The artifact field `ValidationMetrics::linked_defects` is a DIFFERENT counter (deduplicated defect commits) and keeps its name.
- Hero image: `README.md:31` → `docs/assets/dashboard-hotspots.png`, 1440×960 PNG, only file in `docs/assets/`. SPA generation: `cargo run --release -p codelore-cli --features spa -- analyze --format spa --output <out>.html --repo .`.
- Tests inventory: explain dossier tests in `cli_test.rs mod explain_path` (`:2997`); MCP tests `mcp_tools_list_and_repo_overview` (`mcp_test.rs:132`, asserts every tool has an inputSchema) and `mcp_explain_file_returns_fact_sheet_and_narrative_error_without_llm` (`:465`); no test covers the CLI dirty guard today; `enrichment_fact_sheet_test.rs` has no defect-evidence test today.

---

### Task 1: `explain <path> --defect-calibration` (CLI wire-up)

**Files:**
- Modify: `crates/codelore-cli/src/args.rs` (ExplainArgs, ~line 278)
- Modify: `crates/codelore-cli/src/main.rs` (`run_explain_file` Options literal, ~line 2308)
- Test: `crates/codelore-lib/tests/enrichment_fact_sheet_test.rs`, `crates/codelore-cli/tests/cli_test.rs` (mod explain_path)
- Docs: `docs/advanced-usage.md` (explain section + calibration section cross-ref), `CHANGELOG.md` `[Unreleased]`

**Interfaces:**
- Consumes: `Options::{defect_calibration, allow_foreign_calibration}`; `defect_calibration::{save, DefectArtifact, ...}` (pub).
- Produces: `ExplainArgs::{defect_calibration: Option<PathBuf>, allow_foreign_calibration: bool}` — Task 2 mirrors the same flag pair on McpArgs.

- [ ] **Step 1: Add the flag pair to ExplainArgs.** Copy the two-field block from `CheckArgs` (`args.rs:225-235`) verbatim, adapting only the effect sentence of the first doc comment to: adds a `defect-evidence` section to the file dossier; ignored when the argument names a known topic (same "Ignored when..." convention as ExplainArgs' existing `repo` doc).
- [ ] **Step 2: Thread into Options** in `run_explain_file` (`main.rs:~2308`):

```rust
    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: 1,
        defect_calibration: args.defect_calibration.clone(),
        allow_foreign_calibration: args.allow_foreign_calibration,
        ..Options::default()
    };
```

- [ ] **Step 3: Lib unit test** in `enrichment_fact_sheet_test.rs`: hand-construct a `DefectArtifact` (format_version 1 — read the const; minimal-but-plausible `ValidationMetrics` with `linked_defects`, `implicated_files`, one `band_table` row; `TuningDecision::DefaultsKept { .. }`; any `repo_identity` string), `save()` it to a temp path, build `FileFactSheet` on the existing fixture used by `fact_sheet_is_deterministic` with `Options { defect_calibration: Some(path), allow_foreign_calibration: true, .. }`, assert a `"defect-evidence"` section exists containing the `vintage` and `linked_defects` facts. Also assert the no-flag build has NO `"defect-evidence"` section (additive contract at the unit level).
- [ ] **Step 4: CLI tests** in `cli_test.rs mod explain_path`: (a) `explain <path> --defect-calibration <artifact> --allow-foreign-calibration` prints a dossier containing `defect-evidence` (produce the artifact either by reusing the calibrate-defects fixture flow from `calibrate_defects_links_planted_defect_and_ag_filters_cosmetic_fix` or by writing minimal artifact JSON — must match `DefectArtifact`'s serde shape and `DEFECT_FORMAT_VERSION`); (b) assert the existing no-flag dossier does NOT contain `defect-evidence`; (c) a bad artifact path is a hard error mentioning the path (load's config-mistake posture).
- [ ] **Step 5: Docs + CHANGELOG.** `docs/advanced-usage.md`: document the flag pair on `explain <path>` and the defect-evidence section contents (this re-advertises what commit 7eecc56 trimmed — describe current contract only, no history). CHANGELOG `[Unreleased]` Added entry.
- [ ] **Step 6: Gates + commit.** fmt, clippy, `cargo test -p codelore-lib --features test-support --test enrichment_fact_sheet_test`, `cargo test -p codelore-cli --test cli_test explain`. Commit `feat(explain): surface defect-calibration evidence in the file dossier`.

### Task 2: MCP server `--defect-calibration` startup flag

**Files:**
- Modify: `crates/codelore-cli/src/args.rs` (McpArgs), `crates/codelore-cli/src/mcp.rs` (server struct, run_mcp_server, explain_file tool + its description), `crates/codelore-cli/src/main.rs` (`run_mcp_cmd`, ~line 60)
- Test: `crates/codelore-cli/tests/mcp_test.rs`
- Docs: `docs/advanced-usage.md` MCP section, `CHANGELOG.md`

**Interfaces:**
- Consumes: the same flag-pair convention Task 1 added to ExplainArgs; `defect_calibration::{load, check_repo_identity}`.
- Produces: `run_mcp_server(repo: PathBuf, defect_calibration: Option<PathBuf>, allow_foreign_calibration: bool) -> Result<()>`; `CodeLoreServer` fields of the same names.

- [ ] **Step 1:** Add the flag pair to `McpArgs` (effect sentence: adds a `defect-evidence` section to `explain_file` fact sheets).
- [ ] **Step 2:** Add both fields to `CodeLoreServer`; extend `run_mcp_server` to take and store them; update the `run_mcp_cmd` call site. **Fail fast at startup:** before serving, when the flag is set, run `load()` + `check_repo_identity()` (discard the artifact) so a bad path/foreign artifact errors at `codelore mcp` launch, not on the first tool call.
- [ ] **Step 3:** In the `explain_file` tool body (`mcp.rs:603-607`), populate `defect_calibration`/`allow_foreign_calibration` in the Options literal from server state. Update the tool description: fact sheet gains a defect-evidence section when the server was started with `--defect-calibration`.
- [ ] **Step 4: Test** in `mcp_test.rs`: extend the spawn helper to accept extra CLI args (keep existing call sites unchanged); new test starts the server with `--defect-calibration <minimal artifact JSON> --allow-foreign-calibration`, calls `explain_file`, asserts a `defect-evidence` entry in the `fact_sheet` array. Existing tests must pass unmodified (no-flag path untouched).
- [ ] **Step 5:** Docs (advanced-usage MCP section — flag + section; keep the "no network" framing accurate: artifact is a local file) + CHANGELOG Added entry.
- [ ] **Step 6:** Gates (`cargo test -p codelore-cli --test mcp_test`, fmt, clippy) + commit `feat(mcp): defect-calibration evidence in explain_file fact sheets`.

### Task 3: `is_worktree_dirty` → tracked modifications only

**Files:**
- Modify: `crates/codelore-lib/src/repo/mod.rs` (trait doc, :49-65), `crates/codelore-lib/src/repo/gix_repo.rs` (:254-282), `crates/codelore-lib/src/repo/git_cli_repo.rs` (:230-240)
- Test: `crates/codelore-lib/tests/differential_repo_test.rs`
- Docs: `CHANGELOG.md`; check `docs/advanced-usage.md` for any claim that untracked files trip the guard/cache-skip and align it.

**Interfaces:**
- Consumes: nothing new. Produces: unchanged signature `fn is_worktree_dirty(&self) -> bool`, NEW semantics: true iff tracked content differs from HEAD (staged or unstaged); untracked files never count.

**Rationale (bake into the trait doc, not as history):** all three callers (calibrate-defects mining guard, cache-hit staleness warning, dirty cache-write skip) protect HEAD-time metrics computed over `tracked_paths_at_head()` only — untracked files cannot alter them, so counting untracked produced false positives (e.g. stray screenshots blocking `calibrate-defects`).

- [ ] **Step 1: GitCliRepo** — add `--untracked-files=no` to the `git status --porcelain` invocation. Staged and unstaged tracked changes still appear; `??` lines vanish.
- [ ] **Step 2: GixRepo** — exclude untracked from the status stream. Preferred: configure the `repo.status(...)` platform to skip untracked collection if the pinned gix version exposes it (check docs.rs for the workspace's gix version — e.g. an `UntrackedFiles::None`-style dirwalk setting); fallback: keep `into_iter(Vec::new())` and filter out untracked/dirwalk items (the `index_worktree` variant carrying directory-walk entries) before `.next().is_some()`. Must still detect BOTH head-vs-index (staged) and index-vs-worktree (unstaged) changes. Rewrite the `:260-266` comment to document tracked-only semantics + the caller rationale above (current contract only, no "was switched" history).
- [ ] **Step 3: Trait doc** (`repo/mod.rs:49-62`): define dirty = uncommitted changes to tracked files; untracked excluded, one-line why.
- [ ] **Step 4: Differential tests** — keep `is_worktree_dirty_matches_on_fresh_clone`; add: (a) create an untracked file in a fixture clone → both backends report clean AND agree; (b) modify a tracked file → both dirty AND agree; (c) `git add` that modification (staged only) → both dirty AND agree. Table-drive or three tests, following the file's existing fixture conventions.
- [ ] **Step 5:** CHANGELOG Fixed entry (user-visible: untracked files no longer block `calibrate-defects` or suppress the analysis cache). Gates: `cargo test -p codelore-lib --features test-support --test differential_repo_test`, fmt, clippy. Commit `fix(repo): dirty-tree detection counts tracked modifications only`.

### Task 4: Honesty-floor reason wording — "linked defect-changes"

**Files:**
- Modify: `crates/codelore-lib/src/defect_calibration/validate.rs` (:496 string; prose at :23, :308-311, :425-437; test assert :792)
- Modify: `crates/codelore-cli/tests/cli_test.rs` (:2924 assert)
- Docs: `docs/advanced-usage.md` (if it quotes the floor wording), `CHANGELOG.md`

- [ ] **Step 1:** `validate.rs:496`: `"fewer than 30 linked defects"` → `"fewer than 30 linked defect-changes"`. Align the surrounding prose (module doc :23, `MIN_LINKED_DEFECTS` doc, `linked_defect_count` doc, and the `TuningDecision::DefaultsKept` doc in `mod.rs:238-241` if it says "linked defects") to consistently say defect-changes = (defect, file) incidences, not deduplicated defects. Do NOT touch `ValidationMetrics::linked_defects` (field name, serde, or its doc — that one IS deduplicated defects and correctly named) nor the fact key `"linked_defects"`.
- [ ] **Step 2:** Update both exact-string test asserts (`validate.rs:792`, `cli_test.rs:2924`).
- [ ] **Step 3:** Grep `docs/advanced-usage.md` for "linked defects" — where it describes the honesty floor, update to "linked defect-changes". Leave `docs/superpowers/specs/*` untouched (historical).
- [ ] **Step 4:** CHANGELOG Changed entry. Gates: `cargo test -p codelore-lib --features test-support defect_calibration`, `cargo test -p codelore-cli --test cli_test calibrate`, fmt, clippy. Commit `fix(calibrate-defects): floor reason names defect-changes, the count it takes`.

### Task 5: README hero screenshot re-capture (controller-executed)

- [ ] Build + generate: `/Users/emrec/.cargo/bin/cargo run --release -p codelore-cli --features spa -- analyze --format spa --output <scratchpad>/codelore-dash.html --repo .`
- [ ] Playwright: open the file, light theme, viewport sized to reproduce a 1440×960 capture of the hotspot-map section (match README.md:31 alt text: bivariate hotspot map with lens toolbar visible); overwrite `docs/assets/dashboard-hotspots.png`; verify with `sips -g pixelWidth -g pixelHeight`.
- [ ] Commit `docs(readme): refresh dashboard hero screenshot to the current layout`. No CHANGELOG entry (asset refresh, no behavior change).

---

# Verification (end-to-end)

- `just ci` equivalent via pinned cargo on the final branch state.
- Real-CLI smoke on this repo: `calibrate-defects` into scratch → `explain crates/codelore-lib/src/output/spa.rs --defect-calibration <artifact>` shows the defect-evidence section; same artifact through `codelore mcp --defect-calibration` + `explain_file` handshake.
- Byte-identity spot check: `explain <path>` output with and without the branch (no flags) — identical.
- Docs guard: `git grep -nE "F[0-9]{3}|v0\.[0-9]+" crates/ docs/advanced-usage.md README.md` → no new hits vs base 08c999b.
- Release only via `./scripts/cut-release.sh` after merge, when the user-approved cycle reaches that step.
