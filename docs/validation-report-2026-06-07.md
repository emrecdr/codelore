# CodeLore — Validation Report (2026-06-07)

Cold-pass audit of everything shipped through commit `689c20e` (Plan 7 docs).

## Gate results

| Gate | Result |
|---|---|
| `cargo test --workspace --all-features` | ✅ 322 passed, 0 failed, 3 ignored |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo fmt --all --check` | ✅ clean |
| `cargo deny check` | ✅ advisories ok, bans ok, licenses ok, sources ok |
| All 4 GitHub Actions workflow YAML parse | ✅ ci/bench/release/container all valid |
| `cargo metadata` exposes `workspace.metadata.dist` | ✅ all 6 targets + 4 installers visible |
| Bench harness compiles + smoke-tests | ✅ `ingest_tiny` + `ingest/medium_500_commits` both pass `--test` mode |
| Differential test suite cold | ✅ 8/8 pass — `walk_commits_produces_same_rev_set` (the previously-broken merge case) green |
| Code-maat parity tests cold | ✅ 2/2 pass when `CODE_MAAT_PATH=/tmp/code-maat` (parity validated against real code-maat) |
| End-to-end smoke per `(analysis, format)` pair | ✅ revisions/summary/hotspots×{json,sarif,markdown}/code-health md/coupling csv/clones csv all produce valid output |
| TODO / FIXME / panic audit on production code | ✅ only one `unimplemented!()` — in a type-assertion test that never runs at runtime |

The 3 ignored tests:
- 1 in vendored RCA upstream (intentional, unrelated to CodeLore)
- 2 code-maat parity tests gated on `CODE_MAAT_PATH` (run successfully when env is set)

## Findings — drift and gaps

### S1 (Stale) — `docs/perf-evidence-v1.md` numbers diverged

| Repo | Doc claim | Cold re-measurement | Drift |
|---|---|---|---|
| codescene workspace | 0.24s wall, 87 MB peak RSS | **1.06s wall**, 89 MB RSS | **4.4× slower** |
| gitoxide | 1.16s wall, 75 MB peak RSS | 1.35s wall, 79 MB RSS | 1.16× — within run-to-run variance |

The codescene-workspace number is most likely stale from before P7 added the tree-sitter grammar deps + the clones module (which is parsed at HEAD for its own complexity). The 4× drift on the smallest fixture is real and should be corrected in the doc before tagging v1.0.

### S2 (Inconsistency) — `README.md` claims both 11 and 12 analyses

```
"12 analyses × 6 output formats"            (lead, line 6)
"`codelore analyze --analysis NAME --format csv` for 11 analyses"  (later, line 55)
```

`AnalysisName` enum has 13 variants. The lead is correct (12 user-facing; `Authors` is reserved and bails); the line-55 claim is from before Plan 7 added `Clones`.

### S3 (Architectural gap) — `clones` table created but never populated

`crates/codelore-lib/src/facts/schema_v1.sql` creates the `clones` table (Plan 7 T02). Nothing in the codebase writes to it (`grep -rn "INSERT INTO clones"` returns zero hits). The `--analysis clones` runner computes results ad-hoc per CLI invocation; the `clones` table is dead storage.

This is documented as deferred ("FactsDb integration — Plan 7 v1.x" in CHANGELOG), but worth flagging that the table is currently a schema-level promise the runtime doesn't fulfil. If a user runs `--format sqlite` and inspects the resulting DB, they'll find `clones` is empty even when the CLI just reported 760 clone families.

### S4 (Missing test) — no unit test for `write_clones_csv`

The CSV emitter exists and produces output (validated end-to-end), but there's no `#[test]` that locks the CSV column shape. The other 11 CSV writers each have at least one snapshot-style test. A silent change to the column ordering or header text would slip past CI.

### S5 (Unvalidated infra) — release + container workflows never executed

`release.yml`, `container.yml`, and SLSA L3 provenance attestations are committed but have never run. The first tag push (`v1.0.0`) will execute them for the first time — any YAML/Action-version mismatch would surface there, not before. Two specific things to verify post-tag:

- `cargo binstall codelore` actually resolves and installs the published binary
- The distroless image at `ghcr.io/<owner>/codelore` is `<30 MB` as claimed

### S6 (Methodology gap) — no rename tracking in revisions/coupling/churn

`codelore analyze --analysis revisions --repo . --min-revs 5` against the codescene workspace shows `crates/bca-lib/Cargo.toml,29` as the top hotspot — but `bca-lib/` no longer exists (renamed to `codelore-lib/` in commit `93ea0d1`). Git records the rename as a delete + add pair; without `git log --follow` semantics in the gix walker, the analyses see the pre-rename and post-rename paths as **different files** and split the revision count.

Code-maat has the same gap (it's documented in their FAQ). Calling this out as a known v1 limitation in README / docs is sufficient for v1.0; full rename-tracking via `gix_diff::tree::breaks::detect_renames` is v1.x.

### S7 (Spec drift) — `Clone detection × co-change` still listed as deferred in §8

`docs/superpowers/specs/2026-06-06-codelore-design.md:727` lists "Clone detection × co-change (only flag clones that also change together)" in the deferred Feature Registry. Plan 7 ships the clone-detection half; the × coupling half (the actual "Live Clones" signal) is the v1.x item. The spec entry needs a one-line "partial — see Plan 7" note so it isn't misread.

### S8 (UX gap) — unknown-analysis error doesn't enumerate valid options

```
$ codelore analyze --analysis bogus
error: parsing --analysis "bogus": unknown analysis: bogus
```

Should be:

```
error: unknown --analysis "bogus".
Supported: revisions, hotspots, code-health, code-age, abs-churn, author-churn,
entity-churn, communication, code-ownership, change-coupling, summary, clones
```

(The `Authors` variant is reserved and should not be listed since it bails.)

### S9 (Operational) — clones analysis includes vendored RCA code

`./target/release/codelore analyze --analysis clones --repo .` surfaces ~10-member clone families inside `crates/codelore-rca/src/metrics/loc.rs` — vendored Mozilla test code that's not user-actionable. We hard-skip `.git`, `target/`, `node_modules/` but not user-vendored directories.

A `--exclude PATTERN` (or `.codeloreignore`) flag would let users opt vendored deps out of clones (and likely other analyses too).

### S10 (Workspace hygiene) — untracked artifacts

- `README_v2.md` (129 lines) — looks like a separate polished README draft; not from any committed work in this session. Worth reconciling with the main `README.md` or moving to `docs/`.
- `Cargo.lock` diff adds `alloca v0.4.0` — almost certainly pulled in by an external `cargo install ...` run, not by anything I committed. Pre-tag verification: `cargo update -p alloca` then re-check.

## Net assessment for the v1.0 tag

**The implementation is functionally complete for everything explicitly documented as "in v1 scope."** The gates are clean, the bench harness compiles, the release pipeline is wired (just never executed), and the deferred items are all named in CHANGELOG + spec.

**Pre-tag fix list (all small) — all ✅ closed in Plan 8 §1**:
- ~~S1: refresh `docs/perf-evidence-v1.md` codescene-workspace number~~ → done in `3043a42`. **Finding update:** the "4× drift" claim was wrong — it was a cold-cache first-run artifact, not real drift. Doc now distinguishes warm/cold timings.
- ~~S2: fix `README.md` line 55 "11 analyses" → "12 analyses"~~ → done in `3043a42`.
- ~~S4: add `write_clones_csv` snapshot test~~ → done in `3043a42`.
- ~~S7: one-line update to spec §8~~ → done in `3043a42`.
- ~~S8: better unknown-analysis error message~~ → done in `3043a42` (now enumerates all 12 supported names).

**Spec-gap closures — all ✅ closed in Plan 8 §2**:
- **`--analysis authors`** standalone (was bailed; spec §1.1 gap) → `0ce89ff`. Surfaced as a bonus: a real mailmap bug — `GixRepo`'s ingest used `name: b""` so .mailmap entries of the form `Canonical <c@x> Original <o@x>` (Name+Email match) didn't resolve. Fixed in `1154975`.
- **`-g` / `--group-file` flag** parsed (aggregation deferred to Plan 9) → `af572cb`.
- **`--exclude PATTERN` + `.codeloreignore`** for path-filter → `af572cb`. Closes Finding **S9** (clones scanning vendored RCA code).
- **Clones JSON + Markdown emitters** → `0ce89ff`.
- **`CODELORE-CLONE` SARIF 2.1.0 rule** → `af572cb`. Closes part of the Plan 7 DoD gap.

**Pre-tag, not blocking but high-value**:
- S5: dry-run the release workflow via `workflow_dispatch` before pushing the actual tag
- S6: add a "known limitations" section in README mentioning rename-tracking
- S10: scrub `Cargo.lock` of unintended deps; decide what to do with `README_v2.md`

**Deferred to v1.x** (architectural — addressed in Plan 8 §3–§7, in flight at the time of this update):
- S3: wire `clones` extraction into `FactsDb::ingest` → Plan 8 §4 Tasks 15-16
- The clone-coupling intersection analysis (the differentiator) → Plan 8 §6 Tasks 19-22
- T3 near-miss clones via MinHash → Plan 8 §6 Task 4 (or v1.x follow-up per the plan's optional gate)
- Persistent fact-store cache → Plan 8 §3 Tasks 11-14 (subagent in flight)
- Parallel complexity extraction → Plan 8 §5 Tasks 17-18
- `codelore diff <base>..<head>` PR-mode subcommand → Plan 8 §7 Tasks 23-29

---

*Validation produced by cold-pass audit. Each gate ran fresh against commit `689c20e`. Reproduce: `cargo test --workspace --all-features && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo fmt --all --check && cargo deny check`.*
