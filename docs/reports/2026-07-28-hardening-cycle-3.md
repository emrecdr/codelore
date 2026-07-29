# CodeLore — Hardening Cycle 3: validated findings, retractions, and extension plan

**Date:** 2026-07-28
**Audit target:** `efcdb2a` — v0.23.0 plus PRs #171 and #172, byte-verified snapshot, 452 non-target files.
**Head at delivery:** `origin/main` advanced to `5cd8982` (#173, two-band new-code-period gate) while this report was being written. **§14 is a verified delta against it:** no finding is retracted, one High finding's blast radius grew, and one new Medium is added. The local working copy sits at `76cca86`, two commits behind.
**Method:** four independent audit agents (hardening, extension, SPA/UX, statistics-and-scale), then **four independent adversarial validators** run against their output with the standing instruction *"your default verdict is REFUTED"* and *"severity equals consumer blast radius, not the elegance of the code smell."*

This cycle's headline is not a finding. It is that **adversarial validation changed the answer.** Of seven findings the hardening pass ranked High, three survived at High, three fell to Low, and one fell to Medium after the memory was actually measured rather than estimated. Two findings the original pass ranked *below* High were elevated to High. Three claims were refuted outright and are recorded here as retractions rather than quietly dropped. Six factual errors in the extension audit and three in the brief that commissioned it were caught the same way.

The severity rule is the one this project's own workflow established: a defect that can move a `codelore check` / `diff` / `gate` exit code or a CI verdict is High; a defect that only makes a CLI table look odd is Low; a defect with no in-tree consumer at all is informational. Elegance of the underlying code smell counts for nothing.

---

## 1. Executive summary

| Sev | ID | One line | Status |
|---|---|---|---|
| **Critical** | S1 | SPA boot dies on an unhandled promise rejection in every browser without `scheduler.yield` — 22 of 23 widgets blank | CONFIRMED (rendered) |
| **High** | G2 | Shallow merge-tip checkout → zero commits ingested → every gate green, `result=pass`, exit 0 | CONFIRMED (real git) |
| **High** | A6-2 | 0-byte `.codelore-ratchet.toml` parses cleanly to an all-`None` snapshot → silent rebaseline, verdict "improved" | CONFIRMED (reproduced) |
| **High** | A1-2 | `MAX(commits.date)` used as "now" at ~14 sites → one future-dated commit collapses two gates | CONFIRMED (measured) |
| **High** | A1-4 | `effort_exposure` churn numerator is band-restricted, denominator is not → `max_red_effort_pct` under-fires | CONFIRMED (executed) |
| **High** | A1-7 | `author_aliases.raw_email` PRIMARY KEY cannot represent name+email mailmap resolution → an author's commits vanish | CONFIRMED (structural) |
| Medium | A1/#172 | "Stable under improvement" is false **as stated**; the stability pin structurally cannot detect the counterexample | CONFIRMED (2 counterexamples) |
| Medium | S3 | Colour-lens switch freezes the page 3.5–5.3 s at scale; **the proposed `scope` fix is a provable no-op** | CONFIRMED (A/B measured) |
| Medium | S2 | Corrupt `#codelore-data` renders a complete-looking dashboard with zero content and no visible error | CONFIRMED (rendered) |
| Medium | S4 | The declared a11y alternative to the health lens tells a different, over-optimistic story; `badge-error` is dead code | CONFIRMED (rendered + traced) |
| Medium | A2-1 | Positive-path leakage across the train/validation split inflates the AUC delta by up to +0.167 | CONFIRMED (computed) |
| Medium | A8-1 | Reachability closure on the gate path — downgraded from OOM after measurement | DOWNGRADED |
| Medium | A8-2 | `cycle_health` rebuilds the full graph per candidate; `--rows` truncates *after* the loop | CONFIRMED |
| Medium | A1-19 | `changes_lineage` emits duplicate `(rev, path)` — live by default; original mechanism was wrong | CONFIRMED, corrected |
| Medium | A4 | Dependabot auto-merge step can never succeed and fails its job | CONFIRMED |
| Medium | A7-1 | `--base-cache` digest omits crate version + cache epoch → `exit(4)` on an unchanged branch | CONFIRMED |
| Medium | A2 | Python `ElseClause` misses the `boolean_seq.reset()` its five sibling languages all perform | CONFIRMED (executed) |
| Medium | **N1** | `[new_code]` gate skips under any shallow checkout — silently on the MCP path; second instance of G2's class | CONFIRMED — **new in #173, see §14** |
| Low ×8 | A1-1, A1-3, A1-5, A1-6, A1-12, A2-2, A2-3, G1, A6-1/3/4 | Real mechanics, no gate consumer | DOWNGRADED |
| **Refuted** | A5-1, A6-6/M1, SC-1, ext-#10, `analyze.rs:91` | See §4 | RETRACTED |

Two structural root causes cut across the list and are worth more than any individual entry — see §5.

---

## 2. Critical

### S1 · The SPA is 4% functional in every browser without `scheduler.yield`

`crates/codelore-lib/src/output/spa/js/10_helpers.js:377-379` × `crates/codelore-lib/src/output/spa/js/00_setup_boot.js:975,990`

```js
// 10_helpers.js:377-379  (concat lines 1487-1489)
let _yieldFallbackChannel = null;
let _yieldFallbackInitialized = false;
function yieldToMain() {
  if (typeof scheduler === 'object' && scheduler && typeof scheduler.yield === 'function') {
    return scheduler.yield();        // Chrome 129+ returns here; the TDZ is never touched
  }
  if (!_yieldFallbackInitialized) {  // <-- ReferenceError everywhere else
```

`bootWidgets` is an async IIFE at `00_setup_boot.js:975`. An async function body runs *synchronously up to its first `await`*, so the `yieldToMain()` call at `:990` is evaluated during the synchronous pass — before `10_helpers.js` (concat lines 1111–1534) has been evaluated at all. The two `let` bindings at concat 1487–1488 are still in their temporal dead zone. On Chrome ≥129 the `scheduler.yield()` early return at `:381` jumps clean over the TDZ read, which is precisely why the bug is invisible in the maintainers' own browser.

The validator reconstructed `dashboard.html` byte-faithfully from this tree (replicating `spa.rs::write_spa`, with the four vendored libraries installed from npm and SHA-256-matched against the `build.rs` pins byte-for-byte), served it over HTTP, and rendered it in headless Chromium:

```
=============== MODE: with-scheduler (Chrome/Edge ≥129) ===============
scheduler.yield is: function
canvases: 19   rendered widget bodies: 23 / 23
console: clean

=============== MODE: NO-scheduler (Firefox / Safari / Chrome <129) ===============
scheduler.yield is: undefined
canvases: 2    rendered widget bodies: 1 / 23
[PAGEERROR] Cannot access '_yieldFallbackInitialized' before initialization
     at yieldToMain (codelore.html:4368:5)
     at bootWidgets (codelore.html:3865:15)
```

The rejection propagates out of the un-awaited IIFE and terminates the loop at `i = 0`, so only `factor-header` ever renders. The visible result is the header, six nav chips and the quality-dimension tiles sitting above roughly 14,000 px of blank white.

**Corrected wording — use this, not the original.** This is an *unhandled promise rejection*, not a console error, and it affects **all Safari and iOS Safari versions ever shipped, Chrome/Edge below 129, and Firefox below 142**. The earlier phrasing "browsers without Chrome ≥129" understated it by omitting Safari entirely.

**Provenance, re-verified rather than carried forward:** the identical arrangement exists in the `codelore-125`, `-v22`, `-main` and `-150` trees at `spa/js/10_helpers_drawer.js:376-377` with `00_setup_boot.js:908/977/990`. PRs #169/#170 did not introduce this — but the decomposition did not surface it either.

**Fix.** Move both `let` bindings (or all of `yieldToMain`) into `00_setup_boot.js` above the boot IIFE; or declare them with `var`; or move the boot IIFE to the end of `90_toggles_utils.js`. **Ship the regression test with the fix**: a headless load with `scheduler` deleted before evaluation, asserting all 23 widget bodies render. That test would have caught this, and it is the only durable protection against the same class recurring — S11 below is the same hazard, latent, one edit away from firing.

---

## 3. High

### G2 · A shallow merge-tip checkout turns every gate green

`crates/codelore-lib/src/repo/gix_repo/history.rs:475` — `if !include_merges && commit.parent_ids().count() > 1 { return Ok(None); }`, with `include_merges: false` by default (`options.rs:565`).

gix reads `.git/shallow` and filters grafted parents out of the walk, so the walk itself does not error — it yields exactly the tip. When that tip is a merge, the merge filter discards it, and the ingest sees zero commits. From there: `query_live_paths` is `changes INNER JOIN commits` and returns empty; `current_head_rev` returns `Ok(String::new())` rather than an error; `import_graph` with `n == 0` returns `propagation_cost: 0.0, cycle_count: 0`; every gate finds nothing to violate. `check.rs:261-278` sees an empty violation list and a zero degraded count, prints `codelore check: PASS (0 files evaluated)`, writes `result=pass` to `$GITHUB_OUTPUT`, and returns `Ok(())` — **exit 0**.

The validator built exactly this state with real git:

```
--- .git/shallow ---   bc8748056b043a69e135e5adb2fe597e4db02aab
--- parent count at tip ---   2
af53d17d1e3d64679d1691e75f82b65a2edb397a ABSENT
93f9312df986d618e4b55b8e9e229445cae2008a ABSENT
```

This is not an exotic configuration. `actions/checkout@v4` defaults to `fetch-depth: 1`, and a `pull_request` job checks out `refs/pull/N/merge` — a merge commit. Under `--format sarif`, or when CI reads `steps.x.outputs.result`, the `(0 files evaluated)` tell is invisible.

**Correction to the original framing.** The original claim was that `analyze`'s empty-repo banner catches this while `check` lacks it. That is wrong. `preflight_and_open_repo` (`analyze.rs:1093-1190`) rejects only `Preflight::EmptyRepository`, which `banner.rs:48-50` defines as "`git init` with nothing staged." HEAD *does* point at a commit here. **Nothing catches it, in any command.**

**Mitigating context, stated honestly:** every shipped doc and `examples/.github/workflows/codelore-pr.yml` sets `fetch-depth: 0`, so this requires a user misconfiguration. But `docs/github-action.md:186` warns only that "shallow clones return empty / partial analyses" — it never says the gate goes *green*, which is the part that matters.

**Fix — one line.** `IngestStats.commits_ingested` already exists (`facts/ingest/mod.rs:65`), is already written (`consumer.rs:151`), and is already asserted in tests (`ingest_test.rs:116,495`). No production code reads it. Gate on it: if zero commits were ingested while HEAD resolves to a real commit, that is a hard error, not a pass. Add a shallow-clone preflight (`.git/shallow` exists → warn loudly) as defence in depth.

### A6-2 · A 0-byte ratchet file silently rebaselines the regression gate

`crates/codelore-lib/src/quality_gates/ratchet.rs:153` uses `fs::write(&path, format!("{header}{body}"))` — that is `open(O_WRONLY|O_CREAT|O_TRUNC)` followed by `write_all`. A runner cancellation, OOM kill, or ENOSPC between the two leaves a zero-byte file. Reproduced deterministically:

```
before: 104 bytes
after O_TRUNC, before write: 0 bytes
after failed write: 0 bytes
toml.loads('') -> {}
```

`RatchetSnapshot` (`ratchet.rs:70-82`) carries `#[serde(default)]` with every `RatchetTable` field an `Option<f64>` and no `deny_unknown_fields`, so `""` parses **cleanly** into an all-`None` snapshot. `read_snapshot` returns `Ok(Some(..))`. `evaluate_ratchet` skips every metric whose snapshot value is `None`. The next `codelore check --ratchet` therefore reports **"improved"**, rewrites the floor from the current — possibly already regressed — run, and exits 0.

The compounding detail: `.codelore-ratchet.toml` is a **committed** file (the generated header says so at `ratchet.rs:149`). The reset gets committed, and the regression gate stops gating *for everyone on the team*. And G2 composes with this — a zero-commit run yields all-`None` metrics, so it wipes the ratchet too.

**Fix.** Route the write through the existing `atomic_publish` (`output/mod.rs:52-101`), which is already correct and already used by every `analyze` output path. Separately, treat a zero-byte or all-`None` snapshot as *corrupt* rather than as *absent*: an empty ratchet should be a loud error, since "no floors configured" and "the floors were destroyed" must not be the same state. A6-5 (no signal handlers anywhere) is the enabling condition for this, not an independent defect.

### A1-2 · `MAX(commits.date)` is used as "now", at roughly 14 sites

`crates/codelore-lib/src/analyses/code_familiarity.rs:71-78` anchors on `SELECT MAX(date) AS max_d FROM commits`, then selects `active_authors` as those with `date >= max_d - INTERVAL '{wd} days'`.

One commit with an author date of 2099-01-01 — via `git commit --date`, clock skew on a contributor's machine, or a bad import — collapses `active_authors` from 3 to 1 (measured). Every real author's decay term `EXP(-date_diff('day', co.date, max_d) / 220)` underflows to zero, `active_k_sum` goes to ~0, `familiarity_pct` goes to ~0, and a configured `code_familiarity_min` fails a healthy repository. **No clamp exists at ingest** — `append_commit` (`consumer.rs:206-217`) writes the author timestamp verbatim.

Consumer chain: `check.rs:645-665` → `evaluate_familiarity_rows` → `violations.extend(fam_v)` → `check.rs:261` → exit 1.

**Three corrections to the original claim.** First, `code_familiarity_min` is **not** defaulted to 70.0 — it is `Option<f64>`, default `None`, opt-in. Do not publish the 70.0 figure. Second, the pattern is far broader than the six sites originally listed: it is roughly **fourteen**, additionally including `coordination_needs.rs:213`, `cycle_health.rs:197`, `delivery_metrics.rs:404`, and `effort_exposure.rs:181,363,382,404,665`. Third, `code_age.rs`'s immunity is confirmed but for a different reason than stated — it anchors on wall-clock / `--age-time-now` *and* filters `commits.date <= anchor`.

**This reaches a second gate.** `effort_exposure.rs:179-181`'s `win` CTE shares the anchor, so one future-dated commit collapses the window to that single commit, making `total_churn` and `total_commits` that commit's alone and `churn_share_pct` garbage — which lands on `max_red_effort_pct` at `check.rs:642`. The original finding scoped this to `code_familiarity_min` only.

**Fix.** One shared helper emitting `LEAST(MAX(date), CAST(now() AS TIMESTAMP))`, applied at all sites, plus a one-time ingest warning when `MAX(commits.date) > now()`. The codebase is currently internally inconsistent — `knowledge_islands.rs:120-140` and `code_age.rs` use a wall-clock anchor, everything else uses the data-controlled one. Pick one and enforce it.

### A1-4 · `effort_exposure` compares a band-restricted numerator against an unrestricted denominator

`crates/codelore-lib/src/analyses/effort_exposure.rs:198-203` — `band_churn` does `INNER JOIN eh_bands_v1 b ON b.path = c.path`. `:206-210` — `total_churn` does not. `eh_bands_v1` is populated only from code-health rows plus `complexity_metrics` SLOC (`:85-118`), so lockfiles, markdown and JSON are structurally excluded from the numerator while remaining in the denominator.

Executed: a red file with 300 churn, a green file with 200, and `Cargo.lock` + `README.md` contributing 4,500. `churn_share_pct(red)` computes to **6.0** and passes `max_red_effort_pct = 30.0`. The band-consistent value is **60.0** and should fail. The asymmetry is visible directly in the output — `loc_share_pct` sums to 100.0 while `churn_share_pct` sums to 10.0.

The gate gets monotonically more permissive the more lockfile churn a repository carries.

Consumer chain: `quality_gates/evaluators.rs:474` → `evaluate_effort_exposure_rows` → `check.rs:642` → exit 1.

**Fix.** Add `INNER JOIN eh_bands_v1 b ON b.path = c.path` to `total_churn`. Note that `commit_share_pct` at `:213` has the identical mismatch (`band_commits` at `:194-197` joins, `total_commits` at `:205` is a bare `COUNT(*) FROM win`) — display-only today, so Low on its own, but fix both together since it is the same edit.

### A1-7 · The alias table cannot represent name+email mailmap resolution

`crates/codelore-lib/src/facts/ingest/consumer.rs:121-123` keys the alias map on email alone, first-wins:

```rust
alias_map.entry(event.author_email.clone()).or_insert((canonical, bot));
```

**This is stronger than an `or_insert` slip.** `facts/schema_v1.sql:114-118` declares `author_aliases (raw_email TEXT PRIMARY KEY, canonical TEXT NOT NULL, is_bot BOOLEAN NOT NULL DEFAULT FALSE)`. The table *structurally cannot* hold two canonicals for one email.

Meanwhile `repo/git_cli_repo/mod.rs:152-157` caches on `(author_name, author_email)` — with an in-code comment stating that "a single email can resolve differently depending on the name it ships with" — and `repo/gix_repo/mod.rs:228-248` passes the real name so name+email mailmap rules match. `consumer.rs:206-209` then writes `commits.canonical_author = canonical_author.unwrap_or(author_email)`.

Scenario: a `.mailmap` with two name+email rules sharing one commit email (`Alice Smith <alice@c> Alice <shared@c>` and `Bob Jones <bob@c> Bob <shared@c>`). `commits.canonical_author` holds both `alice@c` and `bob@c`; `author_aliases` holds only whichever was walked first. Every commit under the loser's canonical is dropped from `contrib`, inflating the winner's `k_norm` toward 1.0 — which also manufactures spurious `island_paths` (`top_k >= 0.8`) and moves `familiarity_pct`.

Consumer chain: `analyses/knowledge/shares.rs:105-112,128` → `code_familiarity.rs:60` → `check.rs:645` → exit 1. Also reaches `bus_factor.rs:73-77,88`, display-only.

**Fix.** The schema change is the fix: key `author_aliases` on `(raw_name, raw_email)` to match the resolution the repo layer already performs. This is a `schema_v*` bump, so it wants the cache-epoch treatment.

---

## 4. Retractions

These were published or nearly published and are wrong. Recording them is the point of running validators.

**A5-1 — REFUTED, and it was inverted.** The claim was that `check` treats a degraded sentinel as PASS. In fact `fail_on_degraded` defaults to **`true`** (`quality_gates/config.rs:127-133`, `default_fail_on_degraded() -> bool { true }`) and a degraded verdict pushes a `GateViolation`. Degraded fails by default. The original claim asserted the opposite of the shipped behaviour.

**A6-6 / M1 — REFUTED, no trigger exists.** The claim was a race between concurrent MCP `spawn_blocking` tasks over `atomic_publish`'s per-pid temp name. `mcp.rs` contains no `atomic_publish`, `File::create`, `fs::write` or `fs::rename` call at all — **the MCP server writes no output files**, so the path is never reached from it.

**SC-1 — REFUTED, premise is wrong.** The claim was unbounded AST recursion in the `codelore-rca` visitors causing SIGSEGV. The visitors are iterative `TreeCursor` walkers. The only self-recursive function in rca, `dump_tree_helper`, is referenced from nowhere in `codelore-cli` or `codelore-lib`.

**Extension claim #10 — partially wrong.** "Print evidence chains in text-mode `check`" survives as a proposal, but its supporting claim that `evidence_for_path` has a single caller is false: `diff_output.rs:473` is a second caller, and is itself a working precedent for the plumbing.

**`analyze.rs:91` — REFUTED.** The line is `use_canonical_lineage: !args.no_canonical_lineage && !args.code_maat_compat`, not a literal `true` as previously described. (The *default* is still effectively true via `options.rs:567`, which is what makes A1-19 live — but the site itself was mis-described.)

**A1-23 — the standing retraction from prior reports holds.** The validator could not construct a NULL. Both production writers bind non-`Option`; the only NULL-capable INSERTs are `cfg(test)`. The negative result stands.

**Six errors in the extension audit**, caught and corrected: `from_path` has one caller, not zero; SZZ links are consumed, not discarded; `ai_attribution` has four SQL sites, not three; `evidence_for_path` has two callers, not one; `god-classes` under `--group-file` returns an **empty result set**, not zeros (the `>= DEFAULT_MIN_TOTAL_FAN = 10` predicate filters every row out); and CodeScene's public claim is "25+ factors", not "25–30".

**Three errors in the brief that commissioned that audit**, also corrected: code health has **8** smells, not "25+" (that was CodeScene's number); CodeScene's CodeHealth MCP has **24 tools — 18 that work with any valid licence against local code, 6 requiring a CodeScene Core connection**, not "2 behind a license" and not "14 standalone"; and MTTR is not a live DORA metric.

---

## 5. Two structural root causes

These are worth more than any single finding above, because each one explains a class.

**The degraded sentinel is self-defeating.** `check.rs:457-463` defines degraded as `code_health.is_empty() && SELECT COUNT(*) FROM complexity_metrics > 0`. But `complexity_metrics` is itself derived from `changes ⋈ commits` — the same join that produced the empty result. When the ingest is blind, *both* sides are empty, the conjunction is false, and the sentinel cannot fire. **The mechanism designed to catch "the analysis silently returned nothing" is structurally incapable of firing for the exact blindness class it exists to catch.** G2 is the concrete instance; the same shape will recur for any future ingest-level failure. The sentinel needs an independent witness — ingest counts, or the HEAD tree file count — not a derived table.

**The stability pin cannot exercise the term that is actually unstable.** See §6 below. The general lesson: a property test that constructs its "improvement" by mutating one input table cannot observe a property that depends on a *different* input table. Pin tests must perturb the same surface a real user does.

---

## 6. Medium

### A1 / #172 · "Stable under improvement" is false as stated

`CHANGELOG.md:9` claims, unqualified: *"Improving one file therefore leaves every other file's anchored score unchanged, so an absolute ceiling on it is stable under improvement."*

`hotspot_score_anchored = 10 · pr_rev · cp²` retains **one repo-relative population term**: `pr_rev = PERCENT_RANK() OVER (ORDER BY revs)`. The validator executed `PR_REV_SQL` verbatim from `hotspots.rs:352-361` against real DuckDB 1.5.4 and ported `anchored_score` verbatim from `hotspots.rs:429-432`, at `min_revs = 5`.

**Counterexample 1 — one improving commit.**

```
BEFORE  (n=4)                        AFTER one refactor commit on src/a.rs (29→30 revs)
  src/b.rs  20  pr .0000 cp .50  0.0000    src/b.rs  20  pr .0000 cp .50  0.0000
  src/a.rs  29  pr .3333 cp .60  1.2000    src/c.rs  30  pr .3333 cp .90  2.7000
  src/c.rs  30  pr .6667 cp .90  5.4000    src/a.rs  30  pr .3333 cp .60  1.2000
  src/d.rs  40  pr 1.000 cp .55  3.0250    src/d.rs  40  pr 1.000 cp .55  3.0250

  src/c.rs   5.4000 -> 2.7000   delta -2.7000   <-- CHANGED, and it was never touched
```

**Counterexample 2 — an extract-module refactor trips the ceiling.** Any real improvement is itself a commit, and an extraction adds a file that eventually clears `min_revs`. For a file at rank `k` of `n`, `PERCENT_RANK` moves `(k−1)/(n−1) → k/n`, and `k/n > (k−1)/(n−1)` for all `k < n`, so every non-top file's `pr_rev` **rises**.

```
BEFORE (n=5)                          ceiling hotspot_anchored_max = 7.4 -> PASSES
  src/f4.rs 40 pr .7500 cp .99  7.3507   <- worst

AFTER src/f1.rs refactored, extracting src/f1_helper.rs which reaches 5 revs (n=6)
  src/f2.rs 20 pr .4000 cp .45  0.8100   delta +0.3038  <-- untouched
  src/f3.rs 30 pr .6000 cp .50  1.5000   delta +0.2500  <-- untouched
  src/f4.rs 40 pr .8000 cp .99  7.8408   delta +0.4901  <-- untouched

  worst 7.3507 -> 7.8408; ceiling 7.4 => gate FAILS
```

This is the exact failure mode #172 exists to eliminate, and it is the exact configuration CodeLore ships for itself — `hotspot_anchored_max = 9.9`, set "just above the measured worst anchored score (9.76)".

**Why the pin cannot see it.** `hotspots.rs:568-614` `anchored_score_is_stable_when_worst_file_improves` improves via `UPDATE complexity_metrics SET cognitive = 5 WHERE path = 'src/worst.rs'` (`:591-596`). It never writes `changes`, so `file_revs` — and therefore `PERCENT_RANK() OVER (ORDER BY revs)` — is held constant by construction. The pin can only exercise the `cp` term, which is genuinely corpus-anchored. The `pr_rev` term is repo-relative and is the entire defect.

**Corrected wording for the CHANGELOG — the claim is not worthless, it is unqualified.** It is true under the qualifier *"holding the revision population fixed"* and false without it. Either add that qualifier, or anchor `pr_rev` against the corpus too and make the claim honestly. The second is the better feature; the first is the honest ship-today.

### S3 · The colour-lens switch freezes the page — and the proposed fix is a no-op

`90_toggles_utils.js:39` calls `startViewTransition(fn)` with **no `scope` argument**, so `10_helpers.js:353-357` falls through to `document.startViewTransition(updateFn)` — the document-scoped path whose own comment at `:348-350` says it "blocks every other widget until the crossfade settles."

Measured on Chromium 141, 1200 hotspot rows:

```
WITH document.startViewTransition:      max rAF gap 4549.8ms / 3166.4ms   sum longtasks 4173 / 3941
WITHOUT (helper's sync fallback):       max rAF gap  416.7ms /  316.6ms   sum longtasks  428 /  325
```

The transition contributes roughly **10×** the re-render's own cost. At 8 rows it is 0.53–0.60 s; at 1200 rows it is 3.5–5.3 s with zero animation frames delivered. The 1200-row case is representative: `analyze.rs:1340` caps only entity-ownership at 200 files; `hotspots` at `:1350` has no cap.

**The claimed remediation is provably a no-op.** The validator rebuilt the bundle with `:39` patched to pass the hotspot container as `scope` and re-measured: `max rAF gap 4766.5ms / 3099.9ms` — statistically identical. `Element.startViewTransition` is Chrome/Edge 147+, unsupported in Firefox and Safari, so on Chromium 141 the helper's `if (scope && typeof scope.startViewTransition === 'function')` branch is never taken. `20_hotspots.js:818,827` **do** pass a scope and exhibit identical document-wide behaviour, which corroborates this.

**The fix is to shrink the transition, not to add an argument.** Cap the rendered hotspot rows, or skip the transition above a row threshold, or drop it on this control entirely. Also note the earlier "5.7 second" figure is not a constant — it is dataset-dependent and only reached at large repo scale. Do not publish it as a fixed number.

### S2 · A truncated artifact renders as a healthy empty dashboard

`00_setup_boot.js:60-65` catches a parse failure, `console.error`s, and returns from the IIFE; `:55-58` does the same for a missing data block. Rendered result: **0 of 23 widget bodies**, 6,377 visible characters of chrome, and **no error banner**. Header, theme toggle, all six nav chips, the "Overview" heading, the "Quality dimensions" title and its full explanatory paragraph — then pure white. Indistinguishable from "this repo has no findings."

Anyone with a truncated artifact — a CI upload cut short, an email gateway, a partial download — sees a confident, complete-looking, entirely empty dashboard.

**Fix.** In both guard paths, write a `role="alert"` banner into `<main>` rather than only logging.

### S4 · The a11y alternative to the health lens tells a different story

The canvas health lens **is** correct — `20_hotspots.js:303-315` sources the composite `data.code_health` band. But the "Keyboard-accessible file list" at `template.html:1494-1501`, explicitly introduced as the conformant alternative to that canvas, badges each file by the `cognitive_health` proxy (`template.html:1544-1547`).

`hotspots.rs:285` computes `GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 − 0.40 * norm_cx)))` where `norm_cx = cognitive / MAX(cognitive) OVER () ∈ [0,1]`. The value is therefore **arithmetically bounded to [60, 100]**. The `≤40 → badge-error` branch is **unreachable dead code**. Same repo, same files:

| surface | red | yellow | green |
|---|---|---|---|
| composite `code_health` (the canvas lens) | **76** | **233** | 122 |
| `cognitive_health` badge (the keyboard list) | **0** | **1** of 861 | 860 |

The comment at `template.html:1538-1542` acknowledges the mismatch and ships it anyway. Screen-reader and keyboard users get the over-optimistic story.

**Fix.** Badge from `data.code_health` via `bandByPath` — the same source the lens uses — and delete the `≤40` branch.

### The remaining Mediums, in brief

**A2-1 · Positive-path leakage in the calibration split.** `calibrate_defects.rs:360-428` — positives are one row per `SzzLink`, deliberately not deduplicated by path (the docstring says so), while negatives *are* deduped and disjoint. A path fixed both early and late lands in both halves of the temporal split, and `auc_for` scores from `intensities[path]`, a per-path constant — identical feature vector, identical label, both sides. With 50 links over 40 paths where one path carries 30 of them and no real signal, the measured delta moves from +0.0029 to **+0.1699**, crossing the 0.02 acceptance margin and applying overfit weights in **49 of 60 seeds instead of 17**. Reaches `code_health_min` → exit 1, but only when calibration is explicitly wired via `--defect-calibration`, which bounds it.

**A8-1 · Reachability closure — downgraded from High after measurement.** The gate path genuinely does build reflexive forward and reverse closures as `Vec<HashSet<usize>>` (`evaluators.rs:91-130` → `import_graph.rs:326-384`), and it is on the gate path whenever `max_dependency_cycles` or `max_propagation_cost` is set. But measured density on realistic import graphs is 0.04–2.06% of c², not the claimed near-worst-case: about **0.7 GB at 60k nodes**, not tens of GB. Reaching 11 GB needs a 100-layer deep chain. Still worth a node cap with a clean error, but not the OOM story originally told.

**A8-2 · `cycle_health` per-candidate rebuild.** `cycle_health.rs:143-149` calls `graph_metrics(&graph_without_node(&graph, cand))` inside the per-candidate loop; `:289-305` reconstructs the entire graph each time; `TRIAL_REMOVAL_BOUND = 64` at `:46` bounds cycle *size*, not *count*; and `out.truncate(limit)` at `:168-170` runs **after** the loop, so `--rows` never bounds the work. The shipped `action.yml` `analysis` input is a free-form string, so `analysis: cycle-health` runs this in CI and can blow the job timeout — though it cannot move a gate exit code.

**A1-19 · `changes_lineage` duplicate `(rev, path)` — mechanism corrected.** The original mechanism was wrong: `path_lineage.old_path` **is** unique (`ROW_NUMBER() OVER (PARTITION BY orig ...) WHERE rn = 1`), so the `LEFT JOIN` at `lineage.rs:116-117` cannot duplicate a row. The real mechanism is **key collision in `COALESCE`** — when a single commit touches two paths that canonicalize to the same name, two distinct source rows land on one `(rev, path)` key. It needs only one rename original, not two. Executed: commit r1 touches A and B; A→C, C deleted, B→C; `changes_lineage` yields `(r1,C)` twice, `COUNT(rev)=5` vs `COUNT(DISTINCT rev)=4`. **Live by default** — `use_canonical_lineage` defaults true and `check.rs:91` uses `..Options::default()`. Inflated `revs` can lift a file over `code_health.rs:222 HAVING revs >= ?`, admitting it to scoring.

**A4 · The Dependabot auto-merge step can never succeed.** `.github/workflows/dependabot-auto-merge.yml:69` passes four positionals to `gh api ... --jq --arg sha "$HEAD_SHA" '...'`, where `--jq` takes one. There is no `continue-on-error`, and the default `bash -e` fails the `merge-on-green` job every time it runs.

**A7-1 · Base-cache digest omits version and epoch.** `diff.rs:500-506` folds only `min_revs` and sorted `exclude`; `cache.rs:42-57` folds `CARGO_PKG_VERSION` **and** `CACHE_EPOCH` — and `cache.rs:18-32` documents that `schema_v16` exists *because clone fingerprints changed*. Every `RevAnalyses` field carries `#[serde(default)]`, so a value-semantics change keeps the JSON parseable and the stale cache is accepted. Result: every clone family reads as new, `should_fail` fires, `exit(4)` on an unchanged branch, self-perpetuating until the cache is manually evicted. **Damper:** `--base-cache` appears in no `action.yml` step, no example workflow, and no `actions/cache` pairing — the only documented pairing is with `--cache-dir`, which *is* protected. That is why this is Medium and not High.

**A2 · Python `ElseClause` misses a `boolean_seq.reset()`.** `codelore-rca/src/metrics/cognitive.rs:303-308` omits the reset that its own `ElifClause` at `:301` and the Rust, C/C++, JS/TS/TSX and Java `Else` arms at `:387/:444/:481/:547` all perform — all five carrying the identical explanatory comment. Verified empirically with the crate's pinned grammar (`tree_sitter==0.23.2`, `tree_sitter_python==0.23.6`): the minimal case scores **3** in Python where every other supported language scores **4** for structurally identical code. The six `*_boolean_sequence_across_else_if` tests have no Python member, which is why it went unnoticed. Not cosmetic — the number differs across languages for the same construct.

---

## 7. Downgraded to Low — real mechanics, no gate consumer

The severity rule this project uses is consumer blast radius. Each of these was verified mechanically sound and then downgraded because nothing reads it. Stating the reason plainly matters more than the downgrade.

| ID | Mechanism (verified real) | Why Low |
|---|---|---|
| A1-1 | `--time-bucket` makes `code_health`'s `author_revs` join match zero rows (proven, 0 rows), deleting the author-fragmentation term | `--time-bucket` exists only on `AnalyzeArgs`; `check`/`gate`/`diff`/`mcp` all build `Options` with `..Options::default()` (`time_bucket: None`), and code-health has no SARIF emitter |
| A1-3 | `code_familiarity`'s denominator excludes files with no in-window commits — it inflates on exactly the unowned legacy code it exists to find | Needs `--after`/`--before`, which exist only at `analyze.rs:100-101` |
| A1-5 | `refactoring-targets` reads raw `complexity_metrics` under `--group-file`, zeroing every effort denominator; the inline comment "the grouped table omits `loc`" is factually wrong (`grouping.rs:337` provides it) | No gate reads `refactoring-targets` — `analyze`/MCP display only |
| A1-6 | Rename-staleness plus a missing `&Options` | Only consumer is a dashboard tile on an `.or_else` fallback path |
| A1-12 | `date_diff('day', …)` counts calendar-boundary crossings, so same-day rework 23h59m later is missed while rework 2 minutes later across midnight counts | `rework_pct` has no gate consumer |
| A2-2 | `predicted_pc_drop` subtracts across different denominators, inverting sign when Δccd < 2·ACD | 0 of 31 realistic cycles go negative; no gate consumer |
| A2-3 | Reachability is reflexive so `min(vfi) = min(vfo) = 1` in every acyclic case; all-shared collapse reproduces in 3 of 9 synthetic graphs | Trigger is `ref_in == 1` alone, not "both medians are 1" as claimed; no gate consumer |
| G1 | Shallow non-merge tip hard-errors at `find_parent_commit` → exit 3 | Fail-loud, and documented verbatim in four shipped docs including a troubleshooting row with the exact error string |
| A6-1 | Non-atomic `--base-cache` write | A partial file fails `serde_json::from_str` → warns and recomputes. Self-healing |
| A6-3 | Non-atomic defect-artifact write | `load` is fail-loud by explicit design → exit 4 |
| A6-4 | `diff --output` uses bare `File::create` while every `analyze` path uses `atomic_publish` | Real asymmetry, but no automated consumer reads the diff report file |
| A6-5 | No signal handlers anywhere | True, but it is A6-2's enabling condition, not an independent defect |

Three DuckDB semantics questions were settled by execution rather than documentation, and one of them invalidates a class of prior reasoning: `date_diff('day', a, b)` counts **boundary crossings**, not elapsed 24-hour periods; `/` on two INTEGER columns is **true division returning DOUBLE**, not truncating (so any prior finding predicated on integer truncation in DuckDB is invalid, and `0/0` yields `nan`); and `GREATEST(x, NULL)` **ignores** NULLs, returning NULL only when every argument is NULL.

---

## 8. Remaining SPA and accessibility findings

The decomposition itself is clean. Static analysis of the concatenated artifact with acorn confirmed the concatenation order matches the numeric prefixes exactly and is deterministic (`spa.rs:59-70` is a literal `concat!` of ten `include_str!`s — no glob, no directory walk). The a11y baseline is genuinely good: no unlabelled images, canvases or buttons; no heading skips; correct tab nesting; `lang="en"`; `prefers-reduced-motion` CSS present; one each of the main, nav, header and footer landmarks; focus visibility correct; 375×812 and 1440×900 clean; dark mode clean.

The gaps, in severity order:

**S5 (Medium-High)** — 7 of 8 circle-pack lenses encode 100% of their meaning in colour with no on-screen key, failing WCAG 1.4.1. `renderBivariateLegend` (`20_hotspots.js:524`) is the only key in the widget and shows only for `bivariate`. Measured across all eight lenses: `legend=block` for bivariate, `legend=none` for the other seven, with 0 characters of non-canvas body text in every case. The wording exists only in a hover tooltip on the tab — unavailable to touch users and to anyone already looking at the chart. Two of the affected lenses are ones the guided tour drives users into.

**S6 (Medium)** — `template.html:1456` states "green ≥71, yellow 41–70, red ≤40"; `bands.rs:18,23,40-47` implements green `>= 70`, yellow `>= 40`, else red. A score of exactly 70 reads green in code and yellow in the tooltip; exactly 40 reads yellow in code and red in the tooltip. Worse, the payload already ships `options.health_green_min: 70` and `health_yellow_min: 40`, and four JS sites read them — the tooltip is the only surface that hardcodes.

**S7 (Medium)** — No loading state. First contentful paint at 168 ms, 23 empty widget bodies with zero skeleton or `aria-busy` elements at 229 ms, fully rendered at 3,192 ms. Three seconds of a fully-chromed dashboard with 23 empty rectangles and nothing saying "working" — indistinguishable from S2's permanent failure.

**S8 (Medium)** — No skip link between `<body x-data>` at `template.html:1215` and `<header>` at `:1227`, on a page with **704 focusable elements**; and 2 of 3 tables have no `th[scope]`.

**S9 (Low-Medium)** — At 768×1024 (iPad portrait), `group-overview` and `widget-kpi-tiles` overflow: scrollWidth 735 against clientWidth 712.

**S10 (Low-Medium)** — The decision-critical green↔red axis of `BIVARIATE_PALETTE` **survives CVD well** (deuteranopia ΔE 29.2/30.5/35.5, protanopia 34.1/39.7/46.2) because the palette ramps lightness rather than hue alone. That is good design and worth keeping. But there are cross-band collisions: under deuteranopia `yellow-high` vs `red-low` is ΔE **5.4** — near-identical colours in *different health bands* — and under protanopia `green-high` vs `yellow-low` is ΔE **6.7**. Within-band activity steps are compressed even for normal vision (ΔE 8.1–10.9).

**S11 (Low today, PLAUSIBLE)** — A second latent TDZ hazard of exactly S1's class: `initHotspotColorToggles()` at exec-line 616 transitively reaches `BIVARIATE_PALETTE` (concat 1388) and `currentHotspotColorMode` (766). It does not fire today. One edit that makes those paths run at boot and it becomes S1 again. This is the argument for fixing S1 structurally — a lint rule or a boot-order test — rather than by patching two `let`s.

**S12 (Low-Medium)** — Files with no `code_health` row render `rgba(140,140,140,0.55)` in both the health and bivariate lenses, the same grey the author/ai/clones lenses use for "no data". The 3×3 legend has no grey cell and no "no data" caption. "No data" reads as "fine."

---

## 9. Extension plan

The request was to extend existing features, not to add pillars. Every item below names the file it extends; anything that could not name one was cut. Ranked by payoff ÷ effort.

**Do first — these are bugs wearing feature clothing.**

1. **Wire `--arch-rules-file`.** `arch_rules/mod.rs:83` has **zero callers**. The flag is documented and does not exist. XS effort.
2. **Emit `security-severity` on SARIF rule properties.** The computed 0–10 scale currently has no effect on GitHub's alert ranking. XS.
3. **`check --format json`.** `check` is the only member of the gate trio with no machine-readable format. XS.

**Then — highest signal per unit of work.**

4. **Persist SZZ links and inject defect history into `change_context`.** `defect_calibration/szz.rs:308` already produces `Vec<SzzLink>` during calibration and discards it afterwards. This is the highest-signal fact CodeLore computes and throws away. Telling an agent "this file has been the origin of 3 defects fixed in the last 90 days" is worth more than any health score. S effort.
5. **PR comment via `action.yml` + `diff --format markdown`.** Zero new Rust; the highest-visibility surface CodeLore does not currently occupy. XS/S.
6. **Route `evidence_for_path` into text-mode `check` and `gate`.** Already computed for SARIF only; `diff_output.rs:473` is a working precedent for the plumbing. S.
7. **`region` on hotspot and check SARIF results.** Must ship together with fingerprint canonicalization or alerts churn twice. S.
8. **Bumpy Road smell.** `total_nesting` is already computed and unused. This is the single new smell that would move the most files' scores. Output-breaking — forces a `calibrate-defects` re-run. S.
9. **Named gate directives** (`[directives]` in thresholds). S.
10. **`suggest_reviewers` as a 12th MCP tool.** Would beat CodeRabbit's 2026-07-13 file-contributor shipment. S/M.

**Explicitly not worth it**, with reasons, because saying so is part of the job: watch/daemon mode (saves milliseconds, costs a daemon and a whole stale-verdict bug class); SARIF `fixes` (GitHub ingests SARIF 2.1.0 and `fixes` is absent from its supported-property tables — `relatedLocations` and `codeFlows` are supported and `diff_output.rs` already exploits both); `outputSchema` on the text MCP tools (breaks the token contracts that make them good); number-of-functions and total-complexity smells (double-count with `god-class`); LCOM (needs member-access edges the `entities` table does not have); onboarding recommendations (no validation oracle — violates the grounded-metric brand).

---

## 10. Competitive position

**Correct the record first.** Any README or marketing copy citing CodeScene's CodeHealth MCP as "2 tools behind a license" or "14 standalone" is wrong. The accurate figure is **24 tools: 18 that work with any valid licence against local code, 6 requiring a CodeScene Core connection**. `docs/reports/2026-07-21-status-and-competitive-landscape.md:75` carried the false version and **has been corrected in this delivery**, with the retraction left inline rather than silently overwritten; the same file's repowise MCP tool count was corrected from nine to ten at line 110. Likewise, quote CodeScene's own "**25+ factors**" rather than "25–30"; the separately verified figure of 15 publicly enumerated factors is sound and is the more useful number.

**DORA is five metrics.** Change lead time and deployment frequency (throughput); change fail rate, **failed deployment recovery time** (which replaced MTTR in 2023), and **deployment rework rate** (added 2024). Any proposal targeting "MTTR proxies" targets a metric that no longer exists under that name.

**The pattern worth copying** comes from jscpd v5 (full Rust rewrite, 2026-05-26, 24–37× faster, 223 languages) and `ast-grep`: both ship an **MCP server *and* an installable agent skill *and* a compact token-efficient reporter**. CodeLore has the first, and arguably the best version of the third in the class. The missing piece is a published, installable agent skill.

**Where CodeLore is already ahead**, and should defend rather than dilute:

*Working-tree gating.* `gate_changes` projects **uncommitted** edits against HEAD and re-evaluates thresholds on every call (`mcp.rs:1272`, deliberately un-memoized). CodeScene's `analyze_change_set` is branch-versus-base-ref; repowise's `get_change_risk` is commit-or-diff-range. Nobody else gates the dirty tree.

*Exactness of the projected delta.* The composite is linear, so the engine substitutes a `complexity_metrics_projected` temp table and re-runs the same `run_code_health_scoped` twice — history terms freeze automatically rather than being approximated. The one carve-out (`shotgun-surgery`) and the `PERCENT_RANK` population ripple are both documented in the spec rather than hidden.

*Token budgets as tested contracts.* ≤150 tokens per file for `change_context`, ≤80 base plus ≤40 per finding for `gate_changes`, pinned by tests. Sonar advertises −36%, repowise 61–89%, jscpd ships an "AI Reporter" — none of them publishes a *contract*.

*MCP descriptions written for the consumer*, with cost disclosure, sibling cross-references, honest-absence forms named inline, cap disclosure, and explicit "`codelore check` is authoritative" divergence notes. Materially better than machine-generated schemas.

*Own-repo defect calibration.* AG-SZZ plus ROC AUC, precision@k, Wilson 95% CIs, root-commit-SHA repo identity, foreign-artifact rejection, and monotonicity validation with a CI drift test. **Nobody else in the class ships calibration honesty machinery at all.** A2-1 above is a defect *inside* that machinery, which is a much better place to have defects than not having the machinery.

*Statistical discipline.* Fisher exact in log space with correct two-tail handling and `None` on invariant violation; BH-FDR; Wilson intervals; genuine Leiden; iterative Tarjan.

**One unoccupied space:** no product ships formal socio-technical congruence scoring. The only 2026 shipments nearby are CodeRabbit's file-contributor signal (2026-07-13), Greptile's shared-contributor clustering (2026-06-02), and repowise's AI-Debt Radar. CodeLore is blocked on it for a specific, fixable reason — the team map overwrites `canonical_author` at ingest.

---

## 11. Not established

**PRs #162 and #171 cannot be verdicted from this tree.** A tree-wide grep for `#162`, `PR 162`, `pull/162` (and the same for 157, 171, 172) across all `*.md`, `*.rs`, `*.js`, `*.toml` and `*.yml` returns **zero hits**, and the snapshot carries no `.git` directory to diff against. Any claim about what those PRs did or did not fix would be invention. They are recorded as not-established rather than as passing.

**One genuinely stale documentation count.** `docs/codebase_analysis.md:36` reads `G[54 behavioral analyses]` inside a Mermaid node; the registry has **56**. The guard test at `crates/codelore-lib/tests/doc_analysis_count_test.rs:66-110` misses it because `stale_count_in_line` does not tolerate an intervening word between the number and "analyses". Fix the helper, not just the number — otherwise the guard keeps passing over the same shape.

---

## 12. Prioritised worklist

**Now.**

1. **S1** — SPA TDZ. Ship the `scheduler`-deleted regression test with the fix. Every Safari user currently sees a broken dashboard.
2. **G2 + N1 together** — gate on `IngestStats.commits_ingested`. One line, already computed, closes a green-on-zero-data hole; the same signal is what N1's shallow-checkout disclosure needs, so fix the class once rather than twice (§14).
3. **A6-2** — route the ratchet write through `atomic_publish`; treat an all-`None` snapshot as corrupt rather than absent.
4. **A4** — the Dependabot workflow step is failing its job on every run.

**Next.**

5. **A1-4** — one `INNER JOIN`; fix `commit_share_pct` in the same edit.
6. **A1-2** — the shared `LEAST(MAX(date), now())` helper across all ~14 sites, plus the ingest warning.
7. **A1-7** — re-key `author_aliases` on `(raw_name, raw_email)`; schema bump.
8. **A1/#172** — either add the "holding the revision population fixed" qualifier to the CHANGELOG, or corpus-anchor `pr_rev`. Rewrite the stability pin to perturb `changes`, not just `complexity_metrics`.
9. **S2, S4** — the error banner and the a11y badge source. Both small, both correctness-of-what-users-see.

**Then.** A1-19, A2-1, A7-1, A8-1/A8-2, the rca Python `ElseClause` reset (with the missing Python member of the `*_boolean_sequence_across_else_if` family), S3's real fix, and S5–S12.

**Structural, and worth more than any single item.** Give the degraded sentinel an independent witness so it can fire for the class it was built to catch. And adopt the rule that property pins must perturb the same surface a user does — both of this cycle's root causes are instances of a test that cannot observe what it claims to guarantee.

---

## 13. Method and limits

Four audit agents, then four adversarial validators with `REFUTED` as the default verdict. DuckDB was available and was used to settle semantics questions by execution rather than by reading documentation. The SPA was rebuilt byte-faithfully and rendered in headless Chromium; the vendored libraries were SHA-256-matched against the `build.rs` pins before anything was measured.

**What was not done, stated plainly.** The workspace could not be compiled here — `rustc 1.95.0` is installed, `codelore-rca` requires 1.96, and `static.rust-lang.org` is unreachable from this container — so no finding rests on a `cargo` run. Concurrency findings are reasoned from code structure; none was raced under load. `--group-file` findings assume grouping is in use, and how commonly it is set in practice was not determined. Several vendor documentation sites returned proxy denials to direct fetches; those were routed through permitted paths or left unverified rather than worked around, and nothing here rests on a source that could not be read.

**The cycle's own error rate is the most useful number in this report.** Across the four audits, validation refuted 3 claims outright, inverted 1, corrected the mechanism of 2 more, downgraded 12, elevated 2, and caught 9 factual errors — 6 in an audit and 3 in the brief that commissioned it. Roughly a third of what the first pass produced did not survive contact with a determined attempt to disprove it. That ratio is the argument for keeping the validation stage, and for treating any single-pass finding list — including a future one — as provisional until something has tried to break it.

---

## 14. Delta against `5cd8982` (#173), verified at delivery

`origin/main` moved one commit ahead of the audit target while this report was being written: `5cd8982 feat(gates): two-band new-code-period gate scope (#173)`, +1084/−42 across eleven files. Because #173 is a *gate* change and touches `effort_exposure.rs` — the home of High finding A1-4 — the finding set was re-checked against it rather than shipped stale. Nothing is retracted. One High finding's reach grew, and one new Medium is added.

**A1-4 survives verbatim.** #173's edits to `effort_exposure.rs` are confined to the improving-churn machinery from line 394 down; the band-share SQL is untouched. At `5cd8982` the numerator still carries the band restriction the denominator lacks:

```sql
band_churn    AS (SELECT b.band, COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS churn
                  FROM {src} c INNER JOIN win USING (rev)
                  INNER JOIN eh_bands_v1 b ON b.path = c.path GROUP BY b.band),
total_churn   AS (SELECT COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS v
                  FROM {src} c INNER JOIN win USING (rev))          -- no band join
total_commits AS (SELECT COUNT(*) AS v FROM win)                     -- counts unbanded commits too
```

Both `churn_share_pct` and `commit_share_pct` are affected, and `max_red_effort_pct` still under-fires by the ratio of banded to total churn. The §12 worklist entry stands unchanged.

**A1-2's blast radius grew.** `new_code.rs::born_touched_flags` anchors the born/touched partition on `(SELECT MAX(date) FROM commits) - INTERVAL '{wd} days'` — twice in one statement — and `window_start_rev`, now `pub` and shared, uses the same expression a third time. A single future-dated commit therefore no longer just collapses two gates; it now also shifts which files a brand-new gate considers born versus touched. The `LEAST(MAX(date), now())` helper recommended in §12 item 6 should land before, not after, this surface grows again.

### N1 (Medium, new) · The `[new_code]` gate cannot fire under a default CI checkout, and the MCP twin does not say so

`run_new_code_scope` short-circuits to `NewCodeScope::default()` — `window_start_present: false`, both bands empty — whenever `window_start_rev` finds no commit strictly older than the window. `check.rs:658` then evaluates nothing and records `verdict: "skipped"`, and `check.rs:336` prints a stderr warning. The exit code is unaffected.

That is the correct design for a genuinely young repository, and the author documented it honestly in the commit message. The problem is that a truncated checkout is indistinguishable from a young repository at this query, and truncated is the CI default. `actions/checkout` fetches depth 1 unless told otherwise; with one commit in the fact store, `date < MAX(date) - INTERVAL '90 days'` matches nothing, so the gate skips on every run. Measured against this repository's real history: 686 commits across 52 days, about 13.2 per day; the most recent 50 commits span 6 days and the most recent 20 span roughly one. A `fetch-depth: 50` job — already more generous than the default — leaves any `window_days` above 6 with no pre-window baseline. Covering the recommended 90-day window would take on the order of 1,190 commits of depth.

The disclosure also describes the wrong cause. "Repository history is shallower than the {N}-day window" reads as a fact about the repository; on a five-year-old codebase behind `fetch-depth: 1` it is a fact about the checkout, and the maintainer who configured `born_health_min` has no reason to connect the two.

The MCP path is worse, because it discloses nothing at all. `mcp.rs:969-982` applies the same `if scope.window_start_present` guard with no else branch and a comment deferring disclosure to the CLI — *"the authoritative `codelore check` discloses that skip."* An agent calling `check_gates` never sees `codelore check`'s stderr. It receives an empty violation list and reports the gates green, with no field distinguishing "the new-code gate passed" from "the new-code gate did not run." `check_gates` and `codelore check` are documented as twins; here they disagree about whether the user is told anything.

**Fix.** Surface skip as structured data on both paths — a `skipped` list beside `violations` in the MCP response — and make the reason discriminating: compare the ingested commit count and history span against the configured window, and when the shallow-history condition coincides with the shallow-checkout signature, say so and name `fetch-depth`. This is the same one-line signal G2 asks for (`IngestStats.commits_ingested`, computed and asserted in tests but with no production consumer), which is the point worth drawing out.

**Why this matters beyond the finding.** N1 is the second instance of G2's class — a truncated checkout silently converting a configured gate into a no-op — and it arrived in code written after G2's first instance existed. That is the strongest available argument for §5's structural recommendation: patch the class, not the instance. A shallow-checkout detector that runs once at ingest and is consulted by every gate would have made #173 correct on arrival.

**What #173 gets right,** since a delta section that lists only defects misrepresents the change. Section-absent behaviour is byte-identical and was proven so against a clean-tip binary. The touched band reuses the effort decomposition's window-start scan instead of adding a second full-tree health pass, and the born band is a pure lookup against code-health rows the gate already computed. Extracting `net_movement` as a signed quantity, with `file_improved` redefined as `net_movement > 0`, is a real simplification that gives two gates one shared signal. `born ⊂ touched` is handled correctly by branch ordering. The `f64::EPSILON` fail test lets a genuine zero pass, and the upstream delta-health banding — not the epsilon — is doing the real noise filtering, which the doc comment states accurately. The commit message's closing note that the gate is *not* adopted for this repository, with the reason given, is the kind of disclosure most projects omit.
