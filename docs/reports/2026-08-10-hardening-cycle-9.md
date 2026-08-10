# Hardening cycle 9 — the incident, the refutation, and what held

**Anchor:** `66da4d6` (v0.27.3) · **Baseline:** `e788aec` (cycle-8 anchor, v0.27.2) · **Delta:** 13 commits — 12 PR merges (#242–#253) plus the v0.27.3 release commit.

Audited from `git archive main` staged read-only; `main` = `origin/main` (last-known) = `66da4d6`; working tree clean apart from the untracked `HANDOFF.md`. Nothing compiled (toolchain pin unreachable from the audit host); every claim is source-anchored. This cycle's brief added a line — *"all consumers of each and every change are checked"* — and this report is largely about that exact discipline: one incident where it failed (partly on this engagement's watch), and the guard rails that held when it did.

---

## 0. What this cycle actually is

Three stories, told honestly:

**The incident.** Re-pointing the new `v1` Action tag ran the entire release pipeline a second time — `release.yml` and `container.yml` triggered on `tags: ['v*']`, and `v1` is a `v*` tag. A GitHub Release literally named `v1` was published, `releases/latest` began returning it, the Action's own `version: latest` resolution rejected it, every consumer of the published Action failed, and the Homebrew tap was regenerated as "codelore 1" (F289; container-registry residue as F292). **Cycle 8 reviewed the v1 design and pronounced it "no defect." That verdict was wrong**, and §4 owns it. The remediation (#242, #246–#247) is exemplary: `v*.*.*` trigger globs, a guard test that reads `ACTION_MAJOR_TAG` out of `cut-release.sh` and asserts no workflow trigger glob matches it (cross-file coupling made testable, proved discriminating), the bogus Release and container tag deleted, the tap restored.

**The refutation.** #252 refutes M14 — carried by this engagement across three cycles as "the calibrate-defects mining ingest has no truncation witness" — and **the refutation is correct** (§4). The zero-witness cannot fire on that path, adding it would be dead error handling, and the tuning floor already bounds the risk with an artifact that is untuned *and says so*.

**What held.** Two of this engagement's earlier fixes were the reason the incident stayed recoverable: the H1 injection-hardening regex is what *rejected* `v1` as a version (turning silent wrong-binary downloads into loud failures), and the M7 status-branched publish probe is what kept crates.io — the only irreversible surface — untouched. The guards worked; the review missed. That asymmetry is the cycle's lesson.

Beyond those three stories the delta closed the remaining audit tail: **M11** (the last two unbounded MCP outputs, done properly — see §4 for why my "mechanical, reuse the helper" framing was wrong), **M15** (the sqlite offline hint, done elegantly — the extension `INSTALL` is issued as its own statement so the hint attaches to the step that actually fails, no error-text pattern-matching), and **L1** (the three-cycle "(default: all)" streak, dead). It also caught and repaired a CHANGELOG heading-absorption that would have re-shipped v0.27.2's notes under the next version (#253, now guarded by `changelog_release_section_test`), extended the hygiene guard (#245, #248), and landed the first `outputSchema` adoption (`check_gates` now returns `Json<GateSummary>`, #249).

**Fresh audit verdict: no new High, no new Medium — second consecutive cycle.** The findings below are residuals, one process observation, and options.

---

## 1. Verdict table

| Item | Verdict |
|---|---|
| C6-M11 unbounded `check_gates`/`gate_changes` | ✅ Closed (#244) — `check_gates` gains `limit` (default 50) with `violation_count` measured **before** truncation, so verdict and count are cap-invariant; `gate_changes` gains the third render cap with a `(+n more)` tail. #248 then extracted `push_truncation_tail` so the "pattern you must remember" that caused the gap cannot recur |
| C6-M15 sqlite offline hint | ✅ Closed (#251) — split-statement design; hint names network + writable cache + the copy-across workaround |
| C6/7/8-L1 `"(default: all)"` | ✅ Closed (#244) after three cycles — the schema now states the real default |
| C6-M14 mining-ingest witness | ✅ **Refuted (F294), adjudicated and accepted** — see §4 |
| F276 (their first-run-UX finding) | Refuted by #250 — their finding, their refutation; reviewed at commit level only (§5 limits) |
| F289 v1 tag fires publishing workflows | ✅ Fixed (#242) + coupling guard (`tag_trigger_pattern_test`); glob `v*.*.*` verified to match all real releases incl. pre-releases and no bare major |
| F292 container tag residue | ✅ Cleaned (#246–#247) |
| CHANGELOG heading absorption | ✅ Fixed (#253), restore verified against the `v0.27.2` tag blob, guarded by `changelog_release_section_test` |
| C8 outputSchema option | ⏳ Started (#249) — 1 of 11 tools (`check_gates` → `Json<GateSummary>`); a pilot, presumably deliberate |
| C6-M8 MCP cancellation | ⬜ Open — zero `RequestContext` reads; unchanged |
| Gitlink differential fixture | ⬜ Open — the fourth content class remains unprobed |
| rmcp 3.1.1 / rustc 1.97.1 / zizmor / OIDC | ⬜ Not adopted — see §3 |

---

## 2. Process observation, filed as a finding because it is one

**The project's signature move is now converting every incident class into a discriminating guard, and the audit's job is shifting accordingly.** The suite as of v0.27.3: pin-agreement across the five Rust pin sites; SHA-pinning enforcement for third-party actions; the ledger re-stamp with a release-commit completeness check; `ledger_stamp_test` (which caught F289's own mis-stamp); `tag_trigger_pattern_test` (cross-file: script constant vs workflow globs); `changelog_release_section_test`; the broadened comment-hygiene tokeniser; the canonical pseudo-path list with its iterate-everything test. Every one of these was proved discriminating at introduction (deliberately breaking it and watching it name the file). #248 even ran its own multi-agent review with an explicit rejection recorded ("shadowing `violations` would not have compiled — renamed instead"). The consumer-enumeration discipline this engagement kept preaching is now in the tree as executable policy — which is the strongest possible answer to this cycle's new brief line, and it means future audits should spend proportionally more time on the one thing guards cannot see: the *introduction of a new name, ref, or value whose pattern an existing consumer matches* (§4).

---

## 3. Open items and options (the whole list)

- **M8 — MCP cancellation** (last substantive cycle-6 residual): no handler observes the per-request token; a cancelled cold call holds a semaphore permit to ingest completion. With `Json<T>` adoption starting, threading `RequestContext` through the handlers is the natural companion change.
- **`outputSchema` for the remaining 10 tools** — the #249 pilot sets the pattern (`Json<T>` with the existing serde types); finishing the set is mechanical and is the highest-leverage MCP item.
- **Gitlink fixture** — one commit with a mode-`160000` entry; the last unprobed differential class.
- **Schema-vs-resolved-default parity guard** — #244's diagnosis of L1 was precise: *"nothing compared the advertised default to the resolved one"* for three cycles. That comparison is automatable in the file's own test style (parse each tool's param schema doc, assert against `DEFAULT_ROW_CAP`/clamp constants). It would close the L1 *class* the way #232/#242/#253 closed theirs.
- **Currency:** rmcp `3.1.0` → `3.1.2` (published 2026-08-07 20:40 UTC; the manifest already requires `"3.1"`, so this is `cargo update -p rmcp` — lockfile-only, not a manifest edit); rustc pin `1.96.0` → `1.97.1` now, with 1.98 landing ~2026-08-27 on the six-week cadence; `zizmor` in CI for the classes it does audit — template-injection, unpinned-uses, permissions (it would *not* have caught F289; see Errata E6); crates.io Trusted Publishing (retires the standing token; the env-guard conditional goes with it).

---

## 4. The honesty ledger

**Cycle 8 §3.5 pronounced the v1 design "no defect" — and F289 was sitting in it.** The review checked the re-point mechanics, the ruleset interaction, the failure path, even the skew window — everything *inside* `cut-release.sh` — and never asked what else in the repository consumes a `v*` tag push. The same cycle had audited `release.yml`'s tag-gating (`if: github.ref_type == 'tag'` — which `v1` satisfies). The incident commit is charitable ("individually reasonable and destructive only together, which no reviewer reading one file can see"), but cross-file consumer analysis is precisely this engagement's claim, and the user's brief now names it. Method rule added, the generalization of F287's lesson: **when a change introduces a new name, ref, or value, enumerate every consumer whose *pattern* matches it — triggers, globs, regexes, key prefixes — not just the consumer it was built for.** (F287: a documented name nothing provided. F289: a provided name too many things consumed. Same class, opposite directions.)

**The M14 refutation is accepted, and the miss is in the carry-forward, not the original finding.** Cycle 6's M14 said the zero-witness *could not work* on this path (`include_merges` ⇒ count ≥ 1) and recommended a meaningful commit floor instead. Compressed through two residual tables, that became "the mining ingest is still unguarded and unwitnessable" — which #252 rightly reads as "add the witness" and rightly refutes: the witness is dead code there, git flattens a depth-1 merge tip to a root commit, and the tuning floor already produces an artifact that is untuned *and says so*. The refutation's own residual paragraph — a "minimum commits worth mining" floor, deliberately not invented — *is* the original recommendation, consciously deferred under the project's minimum-code rule. Verdict: refutation correct against the claim as carried; the deferral is a legitimate judgment call; F294 logged so no fourth cycle re-reports it. Lesson: **residual tables must carry the finding's precise form, not a paraphrase** — this one degraded from "the standard witness can't work, use a floor" to "no witness", and the degraded form was wrong.

**My §3.2 "mechanical, reuse the helper" prescription was wrong at the source level.** #244's commit message corrects it: `serialize_capped_rows` serializes a slice, so it applied to neither remaining tool — `check_gates` returns a struct (the fix truncates a field, with the count measured pre-truncation so the verdict is cap-invariant — a better invariant than my sketch had), and `gate_changes` returns rendered text (the fix is a third render cap). The finding was right; the prescribed fix was not. Prescriptions deserve the same source-check as claims.

**What held, credited precisely:** the H1 semver regex rejected `v1` at resolve time — every Action consumer failed *loudly* instead of downloading a wrong artifact; the M7 probe skipped crates.io publishes because the versions were already live — the one irreversible surface survived the double-run; the release cut's CI gate ran before the tag dance and aborted the bad cut path; and `ledger_stamp_test` caught F289's own stamp mismatch. Defense in depth is why an incident stayed an anecdote.

**Limits.** Nothing compiled; the new guard tests are read, not run. #250 (F276 refutation) and the #245/#248 hygiene internals were reviewed at commit-message + spot-diff level, not line-by-line — both are their-finding/their-cleanup surfaces with their own recorded validation. `origin/main` is last-known (fetch blocked from the audit host). Currency values carry from the 2026-08-06 verified briefing except rmcp (re-verified 2026-08-07); re-verify before acting on the Rust date.

---

## 5. Housekeeping

- **Branch list is `main` + `gh-pages`** — all prior report and fix branches merged and deleted; cycle-8's report landed via #243.
- **This cycle's artifacts** (`cl273.tar`, `delta9.patch`, ~9.7 MB) are in `_to_delete/cycle9-audit-artifacts/`; `rm -rf _to_delete` when convenient (the bridge cannot unlink). `HANDOFF.md` remains untracked and yours. The usual `tmp_obj_*` strays clear with `find .git/objects -name 'tmp_obj_*' -delete`.
- **This report** is committed to branch `docs/hardening-cycle-9`, based on `main` (`66da4d6`), for landing via PR per the #236/#243 convention.

---

## 6. Errata — corrections from post-publication validation (2026-08-10)

A counter-report validated this cycle's claims independently. Eight load-bearing claims were confirmed with anchors. Five were refuted or corrected; all five verify against the tree or the registry and are **accepted**, and the adjudication caught a sixth error the counter-report did not list. The body above has been corrected in place on this unmerged branch; this section records what changed, because a corrected report that hides its corrections would fail the standard it audits by.

- **E1 (accepted — the sharpest one): `gh-pages` is not stale; the original claim read git's tracking arrow backwards.** `git branch -v` showed the *local* `gh-pages` ref `[behind 140]` its upstream — meaning `origin/gh-pages` had moved 140 commits *ahead*, i.e. the branch is being **actively published** (by `ci.yml:383`'s "Publish self-analysis dashboard demo" job on every push to main, plus `bench.yml` — evidence that was in this engagement's own cycle-6 read of `ci.yml`). The item is deleted from §3. The disclosure in §5 (fetch blocked, refs last-known) does not rescue the conclusion: the last-known state already said "remote ahead", which is the opposite of stale. Lesson recorded: **a tracking ref's `behind` describes the local copy, not the branch** — and "commits behind" is the wrong measure entirely for a publishing branch with independent history.
- **E2 (accepted, both counts): rmcp.** Latest is `3.1.2` (published 2026-08-07 20:40 UTC — about five hours *after* cycle 8's registry check, so cycle 8 was right when written and this cycle repeated it without re-checking). And the bump is not "one line": the workspace manifest requires `"3.1"` (`Cargo.toml:77`), a caret requirement that already admits 3.1.2 — the change is `cargo update -p rmcp`, lockfile-only. A version claim three days old is a stale claim; the engagement's own single-source rule applies to its own prior reports.
- **E3 (accepted): five Rust pin sites, not six.** The guard's module doc says five (`rust_version_pins_test.rs:3`); "six" conflated the guard's coverage with cycle-6 M19's original enumeration, which included the CHANGELOG mention the guard deliberately does not pin.
- **E4 (accepted): the anchor line double-counted** — "13 commits plus the cut" where the 13 *includes* the release commit (12 PR merges + the cut). Corrected.
- **E5 (accepted): Rust 1.98 lands ~2026-08-27**, not ~08-20 — the counter-report re-derived it from the six-week cadence off 1.97.0 (2026-07-16), which beats the briefing's single-source date this report had itself flagged for re-checking. The 1.96.0 → 1.97.1 recommendation stands.
- **E6 (self-caught during this adjudication): the claim that `zizmor` "would have flagged F289's trigger overlap class" was wrong.** zizmor audits workflow files for known defect classes (template-injection, unpinned-uses, excessive permissions, cache-poisoning); a semantically valid `v*` trigger glob colliding with a tag that a *shell script* moves is a cross-file semantic coupling outside its audit set — which is exactly why #242's bespoke `tag_trigger_pattern_test` was the right fix. The zizmor recommendation survives on its real merits; the F289 justification does not.

Standing corrections to the method, both self-inflicted-error classes now twice observed: **re-verify any currency claim at report time, not engagement time** (E2, E5), and **when quoting a git status/tracking figure, state what it measures before drawing a conclusion from it** (E1).

---

## 7. Method

Small delta, first-hand throughout: every commit read (message and diff), the two refutations adjudicated against the original findings' exact text, the incident chain traced end-to-end (trigger globs → guard test → ledger stamps → registry cleanup), final-state anchors verified in the extracted tree (caps, hint, schema pilot, guard-test files), residuals and currency swept by anchor, rmcp re-verified against crates.io earlier in the engagement window. Adversarial validation with default REFUTED — which this cycle ran in both directions: two of their refutations tested against my findings (one accepted, one noted as theirs), and my own cycle-8 verdict re-tested against the incident evidence and overturned. The report is shorter than its predecessors because the tree keeps giving the audit less to find — and because two of its sharpest findings this cycle were about the audit itself.
