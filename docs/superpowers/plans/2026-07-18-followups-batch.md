# Follow-Ups Batch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear the four ledgered follow-ups in one PR: sign-aware citation capture, whole-token unmatched reporting (incl. naming uncited tokens in the ⚠ stamp), the four missing MiningStats rows in defect-validation, and an MCP startup-refusal test.

**Architecture:** Tasks 1 is a self-contained upgrade of `enrichment/citation.rs` + its `engine.rs::stamp` consumer; Task 2 appends rows to the row-agnostic defect-validation flattener; Task 3 is test-only. No schema, cache-key, or emitter-header changes anywhere.

**Tech Stack:** Rust workspace; `regex` crate (NO lookbehind support — the sign design below works around that by post-match context inspection).

## Global Constraints

- Gates per commit, pinned `/Users/emrec/.cargo/bin/cargo`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; the task's targeted test commands. No full workspace suite.
- No `unwrap()`/`expect()` outside tests. No new `#[allow]`. No ticket IDs / plan refs / version numbers / test counts in code or docs. CHANGELOG `[Unreleased]` (currently EMPTY at HEAD — create the subsection) gets one entry per user-visible change.
- Append-only branch (`feat/followups-batch`, base 28e74e3): `git log --oneline -1` before committing; NEVER amend/reset. Conventional Commits. NEVER Co-Authored-By.
- Contract: the citation check remains advisory — its verdict never gates, never changes exit codes; scoring isolation untouched.
- In-code limitation docs (`citation.rs` module doc) and `docs/advanced-usage.md` §"Grounding: fact sheet in, citation check out" must describe the NEW current contract only (no "previously undetectable" history).

## Validated seam facts (verified at 28e74e3)

- Token regex `(?:\d+\.\d+|\d+)(?:%)?` at `citation.rs:110-115`; thousands-stripper `(\d),(\d)` at `:118-122`; `check_citations` loop at `:62-89` pushes `digits` (percent-trimmed) into `unmatched`; `grounded = unmatched.is_empty()`; `SMALL_INT_EXEMPTION: f64 = 12.0` (private, `:28`) guarded by `!is_percent && decimals == 0 && value <= SMALL_INT_EXEMPTION`; `matches_at`/`rounds_to` at `:93-107` (round-to-token-decimals + percent ×100 fallback).
- Disclosed-limits doc block at `citation.rs:49-60` (sign inversion / ≤12 exemption / percent collision). Doc mirror: `docs/advanced-usage.md:1041` (extraction sentence) and `:1048` (honest-limits paragraph containing "cannot detect a sign inversion"). Stamp examples at `:1043-1046`.
- `Groundedness { grounded: bool, unmatched: Vec<String> }`; sole caller `engine.rs:98`; `NarrativeResult.unmatched` + `CachedNarrative.unmatched` already persist the list end-to-end.
- `stamp()` at `engine.rs:124-132`: grounded → `"advisory — model {model}, grounded ✓"`; else `"advisory — model {model}, ⚠ contains uncited claims"`. Tests `stamp_renders_the_grounded_verdict` / `stamp_renders_the_uncited_verdict` at `engine.rs:186-205` assert those substrings; `cli_test.rs:3549` asserts the stamp renders end-to-end.
- 9 citation unit tests at `citation.rs:138-213` (names + expectations listed in Task 1); ungated — `cargo test -p codelore-lib enrichment::citation`.
- Hyphen hazard is real: fact-sheet defect-evidence emits `vintage` verbatim (`fact_sheet.rs:414`), format `defects-2026-07-15`; `defect_validation.rs:99` renders date ranges `"2 (from 2026-01-01 to 2026-04-01)"`.
- defect-validation: `DefectValidationRow { metric, value }` (`defect_validation.rs:37-41`); mining block at `:91-94` surfaces exactly `fixes_found`, `links_found`, `blame_failures`. Missing 4 of MiningStats' 7: `files_blamed`, `lines_considered`, `lines_dropped_cosmetic`, `pure_addition_fixes`. Emitters are row-agnostic (`csv.rs:753-768` header `metric,value`; `markdown.rs:727-752`). No test asserts a total row count for populated artifacts; fixture `base_artifact` (`defect_validation.rs:246-254`) has `files_blamed: 640, lines_considered: 9001, lines_dropped_cosmetic: 120, pure_addition_fixes: 33`.
- MCP: `run_mcp_server` (`mcp.rs:699-711`) runs `load` + `check_repo_identity` BEFORE the tokio runtime and before `rmcp::transport::io::stdio()` — a foreign artifact exits without reading stdin. Foreign error text (`defect_calibration/mod.rs:391-394`): `"defect-calibration artifact was mined from a different repository (artifact identity {…}, this repo is {…}); pass --allow-foreign-calibration to apply it anyway"`, prefixed `analysis error: ` (Display) and `error: ` (main). Exit code 4 (`CodeLoreError::Analysis` → `error.rs:85`; `main.rs:24-34` downcasts the anyhow chain). `mcp_test.rs` helpers: `spawn_mcp_with_args` (`:54-108`, sets `stderr(Stdio::null())` and handshakes — NOT usable for a dead-at-launch child); `write_foreign_defect_artifact` (`:543-576`).
- CHANGELOG `[Unreleased]` is empty at HEAD (line 5, then straight to `## [0.21.0]`).

---

### Task 1: Sign-aware citation capture + whole-token unmatched reporting

**Files:**
- Modify: `crates/codelore-lib/src/enrichment/citation.rs`, `crates/codelore-lib/src/enrichment/engine.rs` (`stamp` + its two tests)
- Docs: `docs/advanced-usage.md` (§ Grounding, lines ~1041-1048), `CHANGELOG.md`
- Tests: inline in both files; check `crates/codelore-cli/tests/cli_test.rs:3549`'s stamp assertion still holds (it must — keep the `"⚠ contains uncited claims"` prefix verbatim).

**Locked design (do not revisit):**

1. **Sign detection by post-match context inspection** (the `regex` crate has no lookbehind; a prefix-consuming pattern would silently change which tokens are found next to `=`/`(` etc.). Keep the existing unsigned `token_regex()` and `find_iter` exactly as-is. For each match, inspect the stripped text before `token.start()`:
   - Let `prev` = the char immediately before the match. If `prev != '-'` → token is positive (today's behavior).
   - If `prev == '-'`: let `prev2` = the char before the `-`. The minus is **unary** iff `prev2` is `None` or NOT `char::is_alphanumeric` — then the token's value is negated. Otherwise (digit or letter before the `-`: `2026-07`, `defects-2026`, `5-3` ranges) it is an infix hyphen → positive, exactly as today.
   - Byte-index safety: use `stripped[..token.start()].chars().next_back()` and `.rev().nth(1)`-style access on the char iterator of the prefix slice — no direct byte indexing arithmetic that could split a UTF-8 boundary.
2. **Exemption on magnitude**: change the guard to `value.abs() <= SMALL_INT_EXEMPTION` applied to the SIGNED value (`-3` exempt, `-15` NOT exempt). Without `.abs()` every negative integer would satisfy `value <= 12.0` and sign capture would be dead code for integers.
3. **Strict sign matching**: pass the signed value into the existing `matches_at` unchanged (`rounds_to(fact, signed_value, decimals)` — a quoted `-0.5` no longer matches a fact of `+0.5`, and a quoted `0.5` no longer matches a fact of `-0.5`; the percent fallback also receives the signed value).
4. **Whole-token unmatched**: push the full display token — sign included when unary, `%` retained: build it as `format!("-{raw}")` when negated else `raw.to_string()` (where `raw` still carries its `%`). Update `Groundedness.unmatched`'s doc comment accordingly.
5. **Stamp names the tokens**: `stamp()`'s uncited arm becomes `"advisory — model {model}, ⚠ contains uncited claims: {list}"` where `{list}` is the first 5 `unmatched` entries joined by `", "`, with `" (+{n} more)"` appended when there are more. The `"⚠ contains uncited claims"` substring stays verbatim (existing asserts depend on it). Grounded arm unchanged.
6. **Docs**: rewrite `citation.rs:49-60`'s limitation block — sign inversion is no longer in the undetectable list; remaining honest limits: the |·|≤12 exemption, percent collision, and right-number-wrong-claim. Update `advanced-usage.md:1041` (extraction sentence gains "sign-aware: a leading minus binds unless it is an infix hyphen in a date or range") and the `:1048` limits paragraph (drop the sign-inversion clause, keep the rest, keep the "labels magnitudes, does not prove claims" framing). Update the ⚠ stamp example (`:1043-1046`) to show the token list. CHANGELOG `[Unreleased]` → `### Changed`, one entry.

- [ ] **Step 1: Failing tests first** in `citation.rs` tests mod (keep all 9 existing tests green — none of their inputs contain unary minuses; `one_invented_number_is_listed_unmatched` and `large_uncited_whole_number_is_flagged` still expect `"42.5"` / `"4200"`, which the whole-token change preserves since neither has `%` or sign):
  - `signed_token_mismatching_positive_fact_is_flagged`: facts `[0.5]`, narrative `"a delta of -0.5"` → `!grounded`, `unmatched == ["-0.5"]`.
  - `signed_token_matching_negative_fact_is_grounded`: facts `[-420.7]`, narrative `"MI of -420.7"` → grounded.
  - `positive_token_no_longer_matches_negative_fact`: facts `[-0.5]`, narrative `"a value of 0.5"` → `!grounded`, `unmatched == ["0.5"]`.
  - `hyphenated_date_fragments_stay_unsigned`: facts `[]`, narrative `"vintage defects-2026-07-15"` → the fragments `2026`/`07`/`15`: `07` and `15` exempt (≤12 magnitude... `15 > 12` — NOT exempt; expectation: `unmatched == ["2026", "15"]`, `07` exempt) — the point under test is that none are read as NEGATIVE (a signed reading would make `-07`/`-15` magnitudes 7/15 with sign — assert the reported strings carry no `-`).
  - `negative_small_int_is_exempt`: facts `[]`, narrative `"a delta of -3"` → grounded.
  - `negative_large_int_is_not_exempt`: facts `[]`, narrative `"a delta of -15"` → `!grounded`, `unmatched == ["-15"]`.
  - `unmatched_percent_token_reports_the_percent_sign`: facts `[]`, narrative `"about 99.5%"` → `unmatched == ["99.5%"]`.
  - In `engine.rs`: extend `stamp_renders_the_uncited_verdict` to build a result with `unmatched: vec!["42.5%", "-0.5"]` and assert the stamp contains `"⚠ contains uncited claims: 42.5%, -0.5"`; add `stamp_truncates_the_uncited_list_after_five` (7 tokens → contains `"(+2 more)"`).
- [ ] **Step 2:** Run `cargo test -p codelore-lib enrichment` → new tests FAIL, old pass.
- [ ] **Step 3:** Implement per the locked design (sign inspection, `.abs()` exemption, signed matching, whole-token push, stamp list).
- [ ] **Step 4:** `cargo test -p codelore-lib --features test-support enrichment` → ALL pass (includes the cache-roundtrip narrate test). Then `cargo test -p codelore-cli --test cli_test explain` (stamp assertion at cli_test.rs:3549 unaffected).
- [ ] **Step 5:** Docs + CHANGELOG per design point 6. Docs-guard: no version refs.
- [ ] **Step 6:** fmt + clippy + commit `feat(enrichment): sign-aware citation capture with whole-token uncited reporting`.

### Task 2: Surface the four missing MiningStats rows in defect-validation

**Files:** `crates/codelore-lib/src/analyses/defect_validation.rs` (rows + tests), `docs/advanced-usage.md` (only if the defect-validation section enumerates the mining rows — grep first), `CHANGELOG.md`.

- [ ] **Step 1:** In `rows_from_artifact`'s mining block (after `links_found`, keeping `blame_failures` last to preserve its "skipped, never fatal" adjacency — order the block: `fixes_found`, `links_found`, `files_blamed`, `lines_considered`, `lines_dropped_cosmetic`, `pure_addition_fixes`, `blame_failures`):
```rust
        row("files_blamed", m.files_blamed.to_string()),
        row("lines_considered", m.lines_considered.to_string()),
        row("lines_dropped_cosmetic", m.lines_dropped_cosmetic.to_string()),
        row("pure_addition_fixes", m.pure_addition_fixes.to_string()),
```
  Update the block comment (it currently says "the subset worth surfacing here" — now it is the full tally set; describe current state only).
- [ ] **Step 2:** Extend `applied_artifact_flattens_to_the_expected_rows` with `value_of` asserts: `files_blamed == "640"`, `lines_considered == "9001"`, `lines_dropped_cosmetic == "120"`, `pure_addition_fixes == "33"`.
- [ ] **Step 3:** `cargo test -p codelore-lib --features test-support defect_validation` → pass; `cargo test -p codelore-lib --features test-support --test defect_calibration_test` → pass (its tests use name lookups, no count asserts).
- [ ] **Step 4:** CHANGELOG `[Unreleased]` `### Changed` entry (defect-validation now reports the full mining tally set). Docs grep + update if the row list is enumerated. fmt + clippy + commit `feat(defect-validation): surface the full mining tally set`.

### Task 3: MCP startup-refusal test (foreign artifact, no override)

**Files:** `crates/codelore-cli/tests/mcp_test.rs` only. Test-only — NO CHANGELOG entry (no user-visible change).

- [ ] **Step 1:** New test `mcp_refuses_to_start_on_foreign_artifact_without_override`: build the `Command` directly (mirror `spawn_mcp_with_args`'s construction at `mcp_test.rs:62-73` — `assert_cmd::cargo::cargo_bin("codelore")`, args `["mcp", "--repo", repo, "--defect-calibration", <artifact>]`, LLM env vars removed) but with `stderr(Stdio::piped())` and NO handshake (the child exits before reading stdin — do not use `spawn_mcp_with_args`, whose `read_ndjson` would panic on EOF). Use `write_foreign_defect_artifact` for the artifact and the existing repo fixture the other MCP tests use. Call `.output()` (or `spawn` + `wait_with_output`) and assert: exit status code == Some(4); stderr contains `"mined from a different repository"` and `"--allow-foreign-calibration"`.
- [ ] **Step 2:** `cargo test -p codelore-cli --test mcp_test` → all pass (existing + new).
- [ ] **Step 3:** fmt + clippy + commit `test(mcp): startup refusal on a foreign calibration artifact without the override`.

---

# Verification (end-to-end)

- Targeted suites per task + fmt/clippy CI-exact via pinned cargo.
- Real-CLI smoke after Task 1: `explain <path> --llm` against the local test-server path is covered by cli_test; additionally run `cargo test -p codelore-lib --features test-support --test enrichment_fact_sheet_test` (fact-sheet determinism unaffected — citation.rs does not touch the sheet).
- Docs guard: `git grep -nE "F[0-9]{3}|v0\.[0-9]+" crates/ docs/advanced-usage.md README.md` — no new hits vs 28e74e3.
- Final whole-branch review → PR → merge on green. NO release cut in this cycle unless the user asks.
