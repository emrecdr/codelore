# Hardening cycle 7 — validating the cycle-6 fixes before they land

**Anchor:** `a113566` (current `main` = v0.27.0 + PR #223) · **Subject:** five in-flight `fix/*` branches plus one uncommitted working-tree change, each remediating a cycle-6 finding.

This cycle is different from the six before it. The previous cycles audited a *released tag*. This one audits **work in progress**: my cycle-6 report was acted on, and the remediation exists as five unmerged branches and one uncommitted edit. So the question here is not "what is broken in the release" but "are these fixes correct, complete, and safe to merge" — which is exactly the discipline this engagement was built around, applied one step earlier in the pipeline than usual.

Everything below was read directly against the branch objects on disk (`git diff main..<branch>`, `git show`), read-only. No branch state was mutated, and the working tree — which you were actively editing during this pass, so treat its M1 state as a moving snapshot — was not touched. Nothing compiled: the toolchain pin (`1.96.0`) is unreachable from the audit host, so every verdict is anchored to source, not to a `cargo` run.

**The headline is good news with one sharp caveat.** The fixes themselves are, almost without exception, *correct and well-made* — accurate comments, real tests, honest disclosure of residual limits. Ten of the twelve cycle-6 items that were attempted are fully and correctly closed. The caveat is not in any fix's logic; it is in **branch hygiene**: two of the five branches were cut from v0.27.0 *before* PR #223 landed on `main`, and carry superseded work that will conflict with — or silently revert — #223 across four files if merged as-is. That is the one thing to handle before anything else (§2, §6).

---

## 1. Fix-validation table

Each cycle-6 finding, the branch that addresses it, and the verdict from reading the diff.

| Cycle-6 | Branch | Verdict |
|---|---|---|
| **H1** version-input injection | `fix/action-injection-and-trend-workflow` | ✅ **Complete** — two independent barriers |
| **H2** trend workflow `format: html` | `fix/action-injection…` (+ dup on mechanical-batch) | ✅ **Fixed** — doc → `markdown`, with a note; but see §4.2 test loss |
| **H3** `diff --fail-on` exit-code docs | `fix/audit-mechanical-batch` | ✅ **Complete** — docs state exit 1 + the taxonomy reason |
| **H4** `delta_health` rename/deletion | `fix/delta-health-rename-blindness` | ✅ **Complete** — base∪head, honest residual documented |
| **H5** `--no-cache` on flagless surfaces | `fix/audit-mechanical-batch` | ✅ **Complete** — all three strings → `--cache-dir` |
| **M1** head-only witness double-scan | *uncommitted, working tree* | ✅ **Correct as drafted** — not committed, not yet tested |
| **M2** ratchet code-health scope | *(branch named for it, not started)* | ⬜ **Not implemented** |
| **M3** SARIF `(skipped)` phantom URI | `fix/audit-mechanical-batch` | ✅ **Fixed + tested** — see §4.1 drift note |
| **M4** MCP "Read-only" overclaim | `fix/mcp-honesty-and-disclosure` | ✅ **Complete** — exact wording recommended |
| **M5** `check --format json` doc | `fix/audit-mechanical-batch` | ✅ **Complete** |
| **M6** knowledge-islands × SARIF matrix | `fix/audit-mechanical-batch` | ✅ **Complete** — replaced with `clones` |
| **M10** hotspots truncation disclosure | `fix/mcp-honesty-and-disclosure` | ✅ **Complete** — routes through `serialize_capped_rows` |

**Not addressed by any branch (still open from cycle 6):** M7 (publish idempotency `curl -sSf` treats 5xx as absent), M8 (MCP cancellation), M11 (unbounded `check_gates`/`gate_changes`/`function_xray` — the M10 sibling fix did not extend to them), M12 (differential binary/non-ASCII/CRLF/gitlink fixtures), M14 (calibrate-defects unguarded mining ingest), M15 (`sqlite.rs` offline hint), M16 (ledger cut-time automation — the ledger got a manual touch, the durable fix did not land), and the cycle-6 Lows (L1 `refactoring_targets` `"(default: all)"` still ships).

The quality of the ten completed fixes is worth stating plainly, because it is unusually high. H1 did **both** things I recommended (a semver regex at the boundary *and* env-routing the step outputs downstream) rather than the cheaper one. H4's comment names the exact two failure cases it fixes and then discloses, unprompted, the residual it does *not* fix (a rename still reads as remove-plus-add, because pairing needs git rename detection this path lacks). M1's rework preserves the H1 cache-poison guard for the full-ingest mode while fixing only the head-only mode. These are the marks of fixes made by someone who read the findings rather than pattern-matched them.

---

## 2. The one problem worth stopping for: two branches predate #223 and will fight it on merge

`main` is `a113566` = v0.27.0 (`9dd2539`) **plus PR #223**, which removed the zero-row notice's dead-end `--min-revs 1` remedy. Two of the five fix branches were cut from `9dd2539`, *before* #223:

| Branch | Merge-base with `main` | Contains #223? |
|---|---|---|
| `fix/action-injection-and-trend-workflow` | `9dd2539` (v0.27.0) | **No** |
| `fix/audit-mechanical-batch` | `9dd2539` (v0.27.0) | **No** |
| `fix/delta-health-rename-blindness` | `a113566` (main) | Yes |
| `fix/mcp-honesty-and-disclosure` | `a113566` (main) | Yes |
| `fix/head-only-witness-and-ratchet-scope` | `a113566` (main) | Yes |

The two pre-#223 branches share an **identical** superseded working state (their `analyze.rs` is byte-for-byte the same blob, `b57db07`), which means they were both cut from one common WIP base that carried an *earlier, inferior* attempt at the zero-row-notice fix. Diffed against today's `main`, that shared state reverts #223 across four files:

1. **`analyze.rs`** — re-adds the `--min-revs 1` remedy #223 deleted, with the old "40 of the analyses" comment #223 corrected to "17 genuine, 23 span-only, 23 never". Merging re-introduces the exact dead-end #223 removed.
2. **`crates/codelore-lib/tests/health_trend_test.rs`** — **deleted in full (−114 lines)**. This is not a test of the removed `format: html` path; it is core coverage — every health score in `[0,100]`, band mapping matches `health_band()`, `combined = mean(arch, code)` to 1e-9, and one-row-per-sample ordering. #223 *modified* this same file, so the merge is a modify/delete conflict, and the "resolve" that takes the branch side silently drops the coverage.
3. **`CHANGELOG.md`** — drops #223's own changelog entry (plus two unrelated 0.27.0 entries), because the branch's `[Unreleased]` predates them.
4. **`README.md`** — reverts the `max_red_effort_pct` gate-example line that was added to `main` after these branches were cut.

None of these four is part of what the two branches set out to fix. They are collateral from a stale base. The good fixes on those branches — H1 on action-injection; H3/H5/M3/M5/M6 on mechanical-batch — are entangled with this collateral on the same commit.

**The fix is a rebase, not a merge.** `git rebase --onto main 9dd2539 fix/audit-mechanical-batch` (and the same for action-injection) replays only each branch's genuine changes onto current `main`; the superseded `analyze.rs`/test/CHANGELOG/README hunks either drop out or surface as conflicts you resolve *toward main* (i.e. keep #223). After rebasing, re-confirm `git diff main..<branch>` touches only the files each fix actually needs — for action-injection that is `action.yml`, `analyze.rs` (the H2 trend piece is doc-only, so analyze.rs should show *no* change post-rebase), `ci.yml`, and the trend docs; for mechanical-batch it is `advanced-usage.md`, `facts/mod.rs`, `sarif.rs` + its test, and `github-action.md`. If `health_trend_test.rs` still shows as deleted after the rebase, that deletion is being carried deliberately and should be reverted — the coverage is real.

The three post-#223 branches (delta-health, mcp-honesty, head-only-witness) are clean: they sit on current `main`, touch disjoint regions of `mcp.rs`/`facts.rs`, and will merge without conflict.

---

## 3. What is finished, what is in flight, what is untouched

**In flight (one edit, correct, uncommitted).** M1's witness rework lives only in the working tree of `fix/head-only-witness-and-ratchet-scope` (which is otherwise identical to `main`). It is correct: it makes the cache-write witness mode-aware —

```rust
let witnessed = if opts.head_only_ingest {
    db.complexity_row_count()? > 0   // the table a head-only scan actually fills
} else {
    db.commit_count()? > 0           // unchanged: the H1 poison guard for full ingest
};
```

— adding a small, well-documented `complexity_row_count()` helper. This is exactly the mode-aware witness cycle 6 recommended: the full-ingest path keeps `commit_count`, so a truncated checkout still bails (H1 stays closed), while a healthy head-only ingest now persists instead of re-scanning. One edge worth a line of thought before you commit: a *source-less* head-only repo (docs-only) has zero complexity rows and would still take the re-ingest-to-memory path — harmless (no error, just no cache persist), and not a case `calibrate` targets, but if you want it airtight, witness on `complexity_rows > 0 || imports_rows > 0`. **Commit it with a regression test** — the probe test that was in the tree during this pass (`zz_m1_probe.rs`) had already been removed by the time I looked, so as of now M1 ships without coverage.

**Named but not started.** The branch is called `…-and-ratchet-scope`, but nothing touches `check.rs`, so **M2 is not implemented**. M2 is the README-contradicting one: `check --ratchet` records a code-health floor even with no code-health gate configured (`check.rs:171-193` reads the unconditional scan, unlike the two ledger-gated metrics beside it). It is a real exit-code-moving item and it is still fully open. Either implement the `thresholds.gates.code_health_min.is_some()` guard, or drop `…-and-ratchet-scope` from the branch name so the scope is honest.

**Untouched from cycle 6.** The residual list in §1 — M7, M8, M11, M12, M14, M15, M16, L1 — has no branch. None is a regression; they are the deferred tail. M16 (the ledger) is the one to watch: `deep_analysis_report.md` was hand-edited this cycle, but the *cut-time re-stamping automation* that would stop the `Fixed (Unreleased)` rot from recurring (cycle-6 M16) did not land, so the rot will return at the next release cut unless the mechanism is built.

---

## 4. New findings this cycle

### 4.1 (Low) The SARIF pseudo-path allowlist is a 3-of-6 hardcoded copy of a set defined elsewhere

M3's fix is correct — `is_repo_wide` now covers `(repo-wide) || (degraded) || (skipped)` and ships with a test that iterates the sentinels. But it hardcodes an allowlist *in the emitter* (`sarif.rs:789`) that mirrors a sentinel set *minted in the gate layer*, and the gate layer mints **six** such pseudo-paths, not three: `(repo-wide)`, `(degraded)`, `(skipped)`, `(change-set)` (`evaluators.rs:302`), `(diff-summary)` (`evaluators.rs:163-200`), and a display-only `(none)`. The three the allowlist omits do not reach the check SARIF emitter *today* — `(change-set)` and `(diff-summary)` are gate/diff-scope and `gate` has no SARIF while `diff`'s SARIF is the hotspot-delta emitter with real paths; `(none)` is a human-readable placeholder, not a violation path. So there is **no live phantom-URI beyond the one M3 fixed** — I checked each, and this is a refutation as much as a finding.

The defect is the *pattern*: two sources of truth that can drift. A seventh sentinel, or a future routing of `(change-set)` into a SARIF surface, silently regresses the exact bug M3 just closed. This is squarely the "no fragile or duplicated logic" bar in the standing request. **Durable fix:** give the gate layer one `fn is_pseudo_path(&str) -> bool` (or a `PseudoPath` enum the violation carries), have every minting site and the SARIF emitter consult it, and point the M3 test at that canonical list rather than a local copy. Then a new sentinel cannot be born without the emitter knowing.

### 4.2 (Medium) 114 lines of real health-trend coverage are deleted as merge collateral

Covered mechanically in §2, called out on its own here because it is the kind of loss that a merge hides. `health_trend_test.rs` is not obsolete — it pins the score range, the band mapping, the combined-mean identity, and sample ordering for the *exact* analysis (`health-trend`) whose workflow H2 just repaired. Whatever the resolution of the branch-staleness problem, this file must survive it. If there is a genuine reason it was deleted (it stopped compiling against some other WIP change), that reason needs to be stated and the coverage re-homed, not dropped.

### 4.3 (informational) The two pre-#223 branches duplicate each other's doc churn

Both stale branches carry the same `deep_analysis_report.md` (−62) and `first-run-ux-review.md` rewrite. Once both are rebased onto `main`, decide which branch owns those doc edits so they are not applied twice (or conflicted). Not a defect; a merge-ordering note.

---

## 5. The honesty ledger

Each candidate above survived a validator whose default was REFUTED. What did not survive is as useful as what did:

- *"H1 is only half-fixed, like last cycle."* Refuted — this time both barriers landed: the resolve step rejects anything but `^v[0-9]+\.[0-9]+\.[0-9]+…$` before the value becomes a step output, **and** the install step reads `STEP_TAG`/`STEP_VERSION` from `env:` instead of splicing `${{ steps.resolve.outputs.* }}`. Either alone closes it; both is belt-and-braces.
- *"H4's union over-counts — every deletion now reads as a health win."* Considered and set aside. Removing code does reduce the risk surface, which is the model's intent; the union is strictly more correct than the head-only set it replaced, and the fix's own comment discloses the one residual (renames read as remove-plus-add). Not a defect.
- *"M3 stopped one step short — the `diff` SARIF emitter has the same bug."* Refuted by reading both emitters: `build_check_result` is the *only* gate-violation SARIF path; `diff`'s SARIF emits hotspot deltas with real file paths and never sees a gate sentinel, and `gate` emits no SARIF. M3's single-site fix covers the whole surface. (The residual is the drift *pattern*, §4.1, not a second live bug.)
- *"M1 weakens the H1 poison guard."* Refuted — M1 branches on `opts.head_only_ingest`; the full-ingest path still gates on `commit_count`, so a truncated non-head-only checkout still bails exactly as before.
- *"The fix branches conflict with each other on `mcp.rs`."* Refuted for the three clean branches — delta-health edits `mcp.rs:~880`, mcp-honesty edits `~724` and `~1660`; disjoint regions, clean auto-merge. The real conflict is the two stale branches vs `main` (§2), not the branches vs each other.

**Limits, stated plainly.** Nothing compiled, so the fixes are validated by reading, not by running their tests — most load-bearing here for M1 (uncommitted, no test as of this pass) and for the assertion that the rebased branches build cleanly. The working tree moved under me mid-audit (the M1 probe test appeared and vanished between two reads), so the M1 verdict is against a snapshot, not a committed object. And the currency claims in §7 are single- or few-source; each is marked and should be re-checked against its primary source before action.

---

## 6. The one thing to do first

**Rebase the two pre-#223 branches onto current `main` before merging anything.** Not because their fixes are wrong — H1 and the mechanical-batch corrections are among the best work in this engagement — but because merging them from a v0.27.0 base is the one action here that can *undo* shipped work: revert #223's zero-row-notice fix and delete 114 lines of live test coverage, both silently if a conflict is resolved toward the branch. `git rebase --onto main 9dd2539 <branch>`, then verify `git diff main..<branch>` is confined to the files each fix genuinely needs. The three post-#223 branches and the uncommitted M1 are safe to land as they are (M1 wants a test first). Sequence: rebase the two, commit M1 with a test, then merge all five — ideally squashed into a single `v0.28.0` so the changelog tells one story.

---

## 7. Improvement options (research-verified, August 2026)

Currency deltas on the surfaces this work touched. Versions cross-checked against primary sources where noted; treat the rest as leads.

- **`rmcp` 3.1.0 → 3.1.1** (2026-08-05, verified via the crates.io API). One patch behind. The larger MCP gap is not a version: **none of the eleven tools declares `outputSchema`/`structuredContent`**, and the annotation pass (M4/M7 last cycle) closed the read-only *claim* but the tools still don't advertise machine-readable output shapes. The current MCP spec revision is **2026-07-28**; CodeLore's stdio transport is insulated from its transport changes, so this is an annotate-and-schema task, not a migration.
- **Retire the long-lived `CRATES_IO_TOKEN` for crates.io Trusted Publishing (OIDC)** via `rust-lang/crates-io-auth-action` (GA, v1.0.4) — `id-token: write` scoped to the publish job only. This also sidesteps cycle-6 M7's fragile idempotent-publish probe by removing the standing secret.
- **Add `zizmor` to CI**, SHA-pinned (`zizmorcore/zizmor-action`, the de-facto linter at 1.29.0). Run against this tree it flags exactly the classes this engagement keeps finding by hand: template-injection (H1), unpinned third-party actions (cycle-6 M13), and over-broad permissions. It turns those from recurring manual findings into a gate.
- **Bump the Rust pin `1.96.0` → `1.97.1`** (current stable; 1.98 lands 2026-08-20) through the six pin sites the `rust_version_pins_test` now guards.
- **Correction to a cycle-6 note:** there is **no DuckDB 2.0** announced — latest is 1.5.5 (crate `1.10505.0`, current), storage format stable since 0.10. Disregard the cycle-6 "ahead of DuckDB 2.0" suggestion; it rested on an unverified premise. Keep the pin.
- **`actions/attest-build-provenance` provides SLSA Build L2, not L3** — this confirms cycle-6 L5; fix the release comment and don't claim L3 (which needs a hardened isolated builder).
- **Competitive:** CodeScene now ships an open-source CodeHealth MCP server and a new AGPL entrant (`repowise`) bundles a 9-tool MCP server — an MCP surface is now table stakes in this category. CodeLore's differentiator is already in-tree: the agent-facing PR-delta gate (`delta_health` + `check_gates`), which is the guardrail loop competitors are racing toward. Worth a roadmap line, not a fix.

---

## 8. Docs to update

| Doc | Change | Driven by |
|---|---|---|
| `CHANGELOG.md` | After rebase, ensure #223's entry survives and the H3 change is flagged as a behaviour change (exit 4→1), not just a Fixed line | §2, H3 |
| `crates/codelore-lib/src/output/sarif.rs` | Replace the hardcoded sentinel allowlist with a gate-layer predicate | §4.1 |
| `crates/codelore-cli/src/check.rs` | Implement M2 or rename the branch | §3 |
| release workflow comment | "SLSA L3" → "Build L2" | §7 |
| `docs/reports/deep_analysis_report.md` | Land the cut-time re-stamp mechanism, not just a manual pass | M16 |

---

## 9. Method and limits

The subject was five branches and one uncommitted edit, not a tag. Each fix was read as `git diff main..<branch>` against the finding it claims to close, then adversarially checked for completeness (does it close the *whole* surface?) and for new defects (does the fix introduce one?). Branch topology — the decisive fact for §2 — was established with `git merge-base` and blob-identity checks, not inferred from diffs. A research pass verified the §7 currency claims against primary sources (crates.io API, the MCP spec index) where possible and flagged the rest.

Limits: nothing compiled (toolchain pin unreachable), so verdicts are source-anchored and the "rebased branches build" claim is untested; the working tree moved during the audit, so M1 is validated against a snapshot; and I did not re-verify the eight untouched cycle-6 residuals beyond confirming no branch addresses them — their cycle-6 anchors stand.

---

## 10. Housekeeping

- **Six report branches now exist locally, none pushed:** `docs/hardening-cycle-4` through `-7` (this one), plus the caveat from prior cycles that cycle-4's branch must not be merged (stale base) and cycle-5's report already reached `main`. Cycle-6's report reached `main`? Verify — it was committed to `docs/hardening-cycle-6`; if you want it on `main`, cherry-pick the single file.
- **The five `fix/*` branches** are the live work; §2/§6 is their merge plan.
- **`_to_delete/`** now holds this cycle's audit artifacts (`cl270.tar`, patches) plus prior cycles' — the device bridge cannot unlink, so `rm -rf _to_delete` is yours to run. `HANDOFF.md` is untracked and yours to keep or discard.
- **This report** is committed to branch `docs/hardening-cycle-7`, based on `main` (`a113566`). It is not merged to `main`, and it deliberately does not sit on any `fix/*` branch so it stays independent of the rebase in §6.
