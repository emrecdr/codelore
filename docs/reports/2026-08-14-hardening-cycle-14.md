# Hardening cycle 14 — auditing the numbers themselves

**Anchor:** `fbc9c93` (main) · **Baseline:** `67548c2` (cycle-13 anchor) · **Delta:** 3 commits (#274–#276), no release cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `fbc9c93`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. This cycle ran **scipy against a faithful port of the statistical core** — which is where its budget went, and what its main result rests on.

> **Revised after an independent validation pass.** The headline held: the statistical core is correct, and the 27,936-table result reproduces exactly against an exact-rational oracle run over the *shipped* source rather than the port. Four claims did not hold and are corrected in place — the regression-test credit in §1, the "converged" call in §1, and both the reachability claim and the recommended fix in §3, whose finding is now **withdrawn**. §5 and §6 carry the rest. Corrections are written into the sections they belong to rather than appended, so the report reads as what is now known to be true; §5 records what changed and why.

The delta is three commits, all consequences of cycle 13. The guard vein that produced the last four cycles' findings has all but converged (§1), so this cycle went somewhere the audit had never systematically been: **the statistics that produce the numbers users act on.** That surface came back clean — cleaner, after validation, than the cycle first reported it — which is a result worth reporting as carefully as a defect would be.

---

## 1. The delta, and a guard that has converged

**#274 fixed my cycle-13 finding**, and found a defect in its own fix on the way: the offset table that maps a match back to a file line was indexed per *character* while `match_indices` returns *byte* offsets, so it drifted on any file containing a multi-byte character — "and these contain them in their prose." Right detection, wrong line reported. #274's commit message credits a regression test for catching that, and I repeated the credit; the tree does not support it. At `368f7d2` the self-test asserted booleans from the per-line matcher and never read an offset, so the byte/char fix shipped with no test at all. The test that reads the reported location — `a_flagged_construct_reports_where_it_is_written` — arrived one commit later, in #276, the commit this cycle was auditing when it copied the claim forward.

**#276 then found the fix had left dead code behind**: widening the guard to whole-file matching left the per-line matcher compiled and tested but uncalled, so "the pinned spellings proved a copy correct while the code deciding the gate went unexercised." Two implementations of one rule, either free to drift with CI green — inside the test written to prevent exactly that. Consolidating them surfaced a third thing: `bool_or(` is a substring of `havingnotbool_or(`, so `HAVING NOT BOOL_OR(is_bot)` matched both banned shapes at two offsets and was double-counted, and the sort-and-dedup that appeared to guarantee uniqueness was comparing distinct offsets and could not collapse them.

I verified the result: single matcher, whole-file normalisation, byte-sized offset table, dead code gone.

**I called this guard converged, and was one shape early.** Applying §12's standard to it once more, the evasions I can construct are `BOOL_OR(DISTINCT is_bot)`, a SQL comment inside the call, and — the one I missed — `MAX(is_bot)` grouped by canonical, which is the same collapse to the planner and the same misclassification to the user. That third shape is the one that matters, because the guard's own negative fixtures assert it *must not* be flagged: the anti-vacuity test teaches a future contributor that an equivalent construct is acceptable. No aggregate of any of these shapes exists under `analyses/` today, so this is a latent seam and not a live defect. The first two evasions were fair to leave unreported; calling the vein exhausted without noticing the third was not.

---

## 2. Fresh surface: the statistical core

Fourteen cycles have audited architecture, CI, MCP, the SPA, guards, docs and release plumbing. What none of them audited systematically is the arithmetic — the Fisher test that decides which change-couplings are real, the BH-FDR correction that decides how many survive, and the Wilson intervals that bound the corpus lens. These produce every number a user acts on, and a defect there is invisible to every guard in the tree because the code is correct-looking and the output is plausible.

**Fisher exact — validated against scipy, clean.** I ported `fisher_two_tail_pvalue` faithfully (log-space factorials, the same `k_min..=k_max` support, the same `observed + tol` two-tail rule) and compared it to `scipy.stats.fisher_exact` over **every 2×2 table with cells 0–12** that has non-degenerate marginals — 27,936 tables — plus skewed and large cases:

| Check | Result |
|---|---|
| Tables compared | 27,936 |
| Max absolute deviation | **3.997e-14** (floating-point noise) |
| Tables differing > 1e-9 | **0** |
| Edge cases (`(5,0,0,5)`, `(0,7,7,0)`, `(100,5,3,200)`) | all match to ≥1e-17 |
| Re-run on the **shipped source** vs an exact-rational oracle | 27,936 tables, max deviation **3.9968e-14**, **0** past `1e-9`, 31/31 module tests pass |

`(50000,10,10,50000)` was originally counted as a fourth edge case; it is not one. Both sides underflow to exactly `0.0`, so the agreement is vacuous.

The implementation is correct, including the two places these usually go wrong: the `(row2 + k) - col1` reordering that avoids u64 underflow when `row2 < col1` (documented in-line, and the naive form would panic in debug), and the additive log-space tolerance, which is a *relative* tolerance in probability space — the comment says so and is right.

**BH-FDR family construction — correct.** The classic error here is computing the family *after* filtering, which silently inflates discoveries. `select_significant` (`coupling.rs:616-631`) builds the p-value family from all Fisher-tested candidates and applies the cutoff afterwards, and the docstring names the invariant explicitly: "The family for FDR is exactly the pairs that produced a valid (non-degenerate) Fisher p-value — SQL-filtered-out and degenerate pairs were never tested and are correctly absent." That is the right family.

**Wilson — matches the canonical form exactly**, including the `k=0` / `k=n` edges handled by the formula rather than by special-casing, and the `wilson_ci_from_proportion` split so the interval wraps *the same* percentile estimate the row reports (so `low ≤ p̂ ≤ high` holds) rather than a re-derived one. The gate-adjacent consumer in `code_health.rs:755` passes the honest pool size and only after the language cleared the trust floor.

**Coverage limit on the above.** Fisher has a second consumer this audit did not trace: `function_coupling.rs:158` computes the same p-value across every function pair in a file — an O(n²) family, where multiple-testing inflation is at its worst — with no significance gate and no FDR path at all. That is defensible as designed rather than a defect: `function-coupling` is documented as a ranking sorted by p-value ascending, and `--fdr-correction` is scoped to `coupling` in both its clap help and `advanced-usage.md`, so nothing silently no-ops and no significance claim is made that the code does not honour. It is recorded here because §7 claims consumers of each primitive were traced, and this one was not. Wilson likewise has four consumers — `architecture_metrics.rs` (×2), `code_health.rs`, `effort_exposure.rs` — of which the paragraph above examined one; the other three hold up, with `effort_exposure` passing genuine integer `k`-of-`n` commit counts and `pool_sample_size` resolving to the pool length.

---

## 3. Finding — withdrawn on validation

### F — WITHDRAWN — BH's critical value is computed as `(k/m)*q`; the association order is observable, but no float form is exact and the recommended replacement is not more correct

`bh_fdr_threshold` (`stats.rs:299`) computes the critical value as `(k as f64 / m_f) * q`. Reassociating the same arithmetic changes the result at the last bit:

```
(14/40)*0.1 = 0.034999999999999996
14*0.1/40   = 0.035
p = 0.035  ≤ (14/40)*0.1 ?  False
p = 0.035  ≤ 14*0.1/40   ?  True
```

A p-value landing exactly on the critical value therefore falls on opposite sides depending on association order. That much is real and reproducible. The two claims I attached to it are not.

**"Not reachable by this analysis's own inputs" is false — it is constructible.** Take a genuine Fisher p-value that lands exactly on a flippable boundary and build the family that makes its rank decisive: at `m=45, k=3, q=0.05`, a real Fisher output of `3.333333333333334e-3` sits exactly on the critical value, and the shipped `bh_fdr_threshold` returns `1.666666666666667e-3` where the reassociated form returns the boundary value itself — two discoveries where the other form reports three. The analysis's own inputs can produce this. What they do not do is produce it by accident: 200,000 random families drawn from real Fisher outputs give zero differences, which is what the 600-family run was actually measuring. "Unreachable" and "never happens by chance" are different statements, and only the second one is true.

**"Cross-multiplying removes the question permanently" is also false, and the recommendation is withdrawn.** No float form is exact. Measured against exact rational arithmetic across all 8,548 boundary coincidences reachable from real Fisher p-values, which form is most accurate depends entirely on how `q` is read — as the float the program actually holds, or as the decimal the user typed:

| Form | vs exact, `q` = the float `0.05` | vs exact, `q` = the decimal `5/100` |
|---|---|---|
| shipped `(k/m)*q` | 68 disagreements | 1,399 |
| reassociated `k*q/m` | 431 | 940 |
| cross-multiplied `p*m <= k*q` | **9** | 1,372 |

No form wins under both readings. The "independent textbook step-up implementation" I measured against was itself one of these float forms, so "agrees with the reference in every case tested" established agreement with a particular rounding, not correctness — the same shape of error as validating a guard against a copy of itself. At this precision the question is not well-posed: every discrepancy is a sub-ULP artifact of representing `0.05` in binary.

**Disposition: no code change.** The honest version of the case *for* changing it is that cross-multiplication does win under the float reading, 9 disagreements against 68 — so this is not quite a lateral move, it is an improvement on one of two defensible oracles and a regression on the other. What settles it is the cost side. The behaviour is unreachable by accident (zero in 200,000 real families), and changing the comparison changes coupling output, which obliges a `CACHE_EPOCH` bump to orphan every cache built under the old form. Paying that to shift a sub-ULP artifact from one arbitrary rounding to another is not a trade this codebase should take. The shipped form and its docstring agree with each other and with the procedure they name; leaving them alone is correct.

Recording a finding against correct code is a worse outcome than recording none, which is why this is withdrawn rather than downgraded. The 27,936-table validation stands on its own: it found nothing, and nothing is the right answer.

---

## 4. Residuals and currency

**Open, unchanged:** the gitlink differential fixture (0 refs under `crates/codelore-lib/tests/` — carried since cycle 6, still the only open item with no decision recorded against it; the gap is narrower than the bare count reads, since `repo/git_cli_repo/tree.rs` unit-tests the `160000` drop directly and only the two-backend fixture lacks one); `outputSchema` at 1 of 11 MCP tools; M8 cancellation (0 `RequestContext`, a design question per E9); zizmor not yet a required context in `protect-main` (disclosed by the project — a one-line ruleset change).

**Awaiting a decision, from cycle 13:** whether the tested `cargo publish --no-verify` split is worth adopting — it makes Trusted Publishing compatible with Build L3, which #271 correctly judged incompatible as I had originally proposed it. No action needed if the answer is complexity; the record just shouldn't carry it as "incompatible."

**Currency:** rmcp `3.1.2` and zizmor `1.29.0` both current as of last cycle's live check; Rust pin `1.96.0` deferred to the next cut by documented convention, which remains the right call.

---

## 5. Honesty ledger

- **The headline result of this cycle is a negative one**, and it took most of the budget to get: the statistical core is correct to floating-point noise against a reference implementation. Fourteen cycles in, "I looked hard at the most consequential code in the product and found nothing" is worth more than another Low, and it is only worth anything if the method is stated precisely enough to re-run — which §2 is.
- **The one finding did not survive validation, and downgrading it was not enough.** I measured it at 5-in-20,000 with a tie-prone harness, re-measured at zero against realistic inputs, and called it informational — congratulating myself in this very section for the discipline of preferring the truer number. Both numbers were answers to the wrong question. The reachability claim I attached to the downgrade is false, and the one-line fix I recommended alongside it is not more correct than the code it would replace. Getting from a better-looking finding to a true one took one step more than I took.
- **I declared a vein exhausted (§1), and was one shape early.** The evasions I named against the bot-filter guard are real but unwritable; the one I failed to name, `MAX(is_bot)`, is both writable and explicitly blessed by the guard's own negative fixtures.
- **Limits, corrected.** The workspace was not compiled; `stats.rs` was validated by porting it to Python and running it against scipy and against an independently-written BH reference. I wrote that the port's fidelity was "established by the 27,936-table agreement itself" — that is circular. The agreement establishes port ≡ scipy; the conclusion needs port ≡ Rust, which was assumed, not shown. Closing it costs almost nothing: `stats.rs` is `std`-only and compiles standalone, and running the shipped source reproduces the table above against an exact-rational oracle (max absolute deviation `3.9968e-14`, zero tables past `1e-9`) and passes the module's own 31 tests. The result stands; the method claimed for it did not. The `(50000,10,10,50000)` edge case listed above also matches "to ≥1e-17" only trivially — both sides underflow to exactly `0.0`, so it validates nothing and should not have been counted as a case. The port covers `fisher_two_tail_pvalue`, `bh_fdr_threshold` and both Wilson entry points, not the whole module — `ln_factorial`'s lazily-grown table and the Kamei/quantile code were read, not executed. #274/#276 were verified by reading the final state, not by running `cargo test`.

---

## 6. Housekeeping

- Branches, enumerated rather than inherited. Local: `main`, `gh-pages` (actively published), `docs/hardening-cycle-12`, `docs/hardening-cycle-14`. Remote: `origin/main`, `origin/gh-pages`, and three merged-but-undeleted PR branches from this very delta — `origin/fix/bot-filter-whole-file-scan` (#274), `origin/docs/hardening-cycle-13` (#275), `origin/test/bot-filter-guard-one-matcher` (#276); those three are safe to delete. **`docs/hardening-cycle-12` remains unmerged** — flagged last cycle, still the only report branch that has not landed. Its content appears absorbed into the ledger's F-numbering, so dropping it deliberately is a fine answer; leaving it undecided is the one option worth avoiding.
- `_to_delete/` carries `cycle11/`, `cycle12/`, `cycle13/`, `cycle14/` — four, not the three I first wrote by extending the previous cycle's list instead of reading the directory. `rm -rf _to_delete` when convenient. `HANDOFF.md` remains yours.
- **This report** was first committed to branch `docs/hardening-cycle-14` (`cbdef09`), based on `main` (`fbc9c93`). That commit holds the pre-validation text; the corrected version above supersedes it and has not yet landed. Whichever route it takes, note that a docs-only squash onto `main` matches CI's `paths-ignore` and leaves `main`'s new HEAD with no run of its own — the same gap #275 left, worth a `gh workflow run CI --ref main` afterwards.

---

## 7. Method

Three commits read at source and verified against the findings they close. The cycle's substance was a first-principles audit of `stats.rs`: each primitive ported faithfully to Python, then exercised against an external reference (scipy for Fisher, an independently-written textbook step-up for BH) across an exhaustive small-table sweep and randomised families.

The validation pass then replaced the port with the shipped source — `stats.rs` is `std`-only, so it compiles standalone — and re-ran the sweep against an exact-rational oracle rather than a floating-point one, which is what a fidelity control has to be. The finding was re-tested in both directions: whether it could fire (it can, constructibly) and whether the recommended fix was more correct than the code (it is better under one reading of `q` and worse under the other, which is not the same as being a fix). Consumers were traced for BH and for the gate-adjacent Wilson call; §2 records the ones that were not.
