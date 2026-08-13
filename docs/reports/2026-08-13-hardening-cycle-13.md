# Hardening cycle 13 — a recommendation I got wrong for four cycles, and its resolution

**Anchor:** `67548c2` (main) · **Baseline:** `267426a` (cycle-12 anchor) · **Delta:** 4 commits (#270–#273), no release cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `67548c2`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled for the audit subject — but this cycle *did* run `cargo` locally against a throwaway crate to settle one question empirically (§2), which is a first for this engagement and is what the main contribution rests on.

Four commits, all of them consequences of the last two cycles: my bot-filter finding fixed (#272), zizmor adopted after it found three live defects (#273, #270), and the whole remaining backlog decided rather than carried (#271). The backlog decision is where this cycle's real content is, because **one of the decisions is that a recommendation I have been making since cycle 6 is wrong.**

---

## 0. What this cycle actually is

#271 took every item that had been sitting as "open, user's call" and decided it — "deferral here means decided-not-now for a stated reason, not undecided." That is the right disposition for a backlog this old. One of those decisions rejects a standing recommendation of mine with an argument I had not considered, and it is correct:

> Trusted publishing … needs `id-token` on the job running `cargo publish`, which builds the crate and so executes `build.rs`. Repository code holding an OIDC token can request any audience including sigstore, forging the provenance the pipeline exists to make unforgeable. Adopting it as written trades Build L3 for the removal of one long-lived secret.

I recommended Trusted Publishing in cycles 6, 8, 9, 11 and 12, and in the same reports I praised the SLSA L3 isolation architecture (#239) whose entire premise is that **no job holding signing credentials may execute repository-authored code**. `cargo publish` runs `build.rs`. The two recommendations were in direct tension for four cycles and I never noticed, because I was checking each against best practice rather than against each other — the precise failure the standing brief names when it says *all components, everything is fully aligned with each other*.

The good news is that the tension is resolvable, and §2 is the tested resolution.

---

## 1. Verification of the delta

**#272 — my cycle-12 finding, fixed properly.** The bot-filter guard now matches against a normalised copy (whitespace stripped, lowercased, table qualifier dropped via `rsplit('.')`), so `bool_or(is_bot)`, `BOOL_OR( is_bot )` and `BOOL_OR(a.is_bot)` are all recognised as the same query the planner sees. The self-test pins six equivalent spellings and three non-collapses including `BOOL_OR(is_merge)`. Tree is clean either way; detection was what was missing.

Their diagnosis of *why* §12's census missed it is sharper than my finding was: the census "enumerated guards carrying instance lists by searching for a `&[&str]` const, and this one holds its instances as inline literals in a `contains()` call. It found one syntactic shape of instance list rather than the class, which is the same defect the census exists to detect, now inside the procedure built to detect it." I reported the missing guard; they found the reason the procedure could not have found it.

**#273 / #270 — zizmor adopted, and it paid on contact.** Three live template-injection sites (F304): steps substituting a ref into a `run:` block, where the expression is spliced into the script text before bash parses it. The vector is not hypothetical in this repository — F296 was a `|` in a tag name corrupting a Markdown table, and the same permissiveness in a `run:` block is shell injection. All three now route through `env:`.

The configuration is the part worth praising: exactly one audit is configured, and it is configured to **agree** with the policy `workflow_action_pin_test` already enforces, with the reasoning written out in full. "Two gates disagreeing about pinning is worse than either alone: a contributor gets told to pin by one and told it is fine by the other." That alignment drops the report from 111/44-high to 74/7 while suppressing nothing. This is the correct way to add a second opinion to a codebase that already has one.

**#270 also caught something zizmor did not:** the container workflow published on a branch dispatch, because its tag rules were all release-shaped except `type=sha`, which matches any ref — so a manual run from a branch built, pushed, tagged and attested a genuinely pullable image from unreleased code. Publishing is now a property of the ref rather than the trigger, with the condition repeated per job so a `needs:` edit cannot silently re-open it. That is the same shape as the v1 incident (F289), found before it fired rather than after.

**Verified closed — cycle-6 M27 (accessibility).** All 18 real `<th>` elements in the widgets now carry `scope=`; the single bare `<th>` is inside a comment. Keyboard row activation (`wireRowKbActivation`) is now wired in four widgets, up from one when M27 was filed. *(My first count here said "6 missing scope" — the grep `<th` also matches `<thead`. Corrected before it reached this report; recording it because a miscount caught late is a finding about the audit, and this engagement's rule is that those get written down.)*

---

## 2. The contribution: Trusted Publishing **is** compatible with Build L3 — tested

The rejection in #271 is right about the mechanism and right to reject the recommendation *as I wrote it*. But the incompatibility is not inherent — it comes from one specific behaviour of `cargo publish`, and that behaviour is switchable.

`cargo publish` does two things: it packages the crate, then it *verifies* by building the packaged tarball. Only the second executes `build.rs`. I tested this directly, with a control:

| Command | `build.rs` executed? |
|---|---|
| `cargo package --no-verify` | **No** |
| `cargo package` (verification on) | **Yes** |

*(Method: a throwaway crate whose `build.rs` writes a marker file; `cargo 1.95.0`. The control matters — a "no" with no corresponding "yes" would just mean the harness was broken, which is the mistake this audit made in cycle 11.)*

So the resolution is a two-job split that preserves both properties:

- The **build job** — already isolated, already `contents: read`, already runs repository code — runs `cargo package` **with** verification. The packaged tarball is proven to compile, and the failure it guards against (an `include`/`exclude` bug shipping a crate that does not build from its own tarball) is still caught.
- The **publish job** holds `id-token: write`, mints the short-lived crates.io token via `rust-lang/crates-io-auth-action`, and runs `cargo publish --no-verify`. It executes no repository-authored code, so there is no `build.rs` to reach for the OIDC token, and the L3 property — signing credentials unreachable from user-defined build steps — holds exactly as it does for the attestation signers.

The long-lived `CRATES_IO_TOKEN` goes away without trading away Build L3. Two honest caveats: the verification build in the publish job is genuinely skipped, so the packaging check has to be *actually* wired into the build job rather than assumed (this is the one place the split can be got wrong); and the publish job must not add any step that runs repository code before `cargo publish`, which is a property worth asserting in `workflow_signing_isolation_test` alongside the scopes it already checks — that guard is the natural home for it, and F303 showed it is the guard that gets tested.

I am proposing this as a resolution to a decision that was made on a correct premise, not as a reversal of it. If the split is judged not worth the moving parts, "decided-not-now" is a fine answer — but the reason would then be complexity, not an incompatibility with L3.

---

## 3. New finding

### F — LOW (new) — The normalised bot-filter matcher is line-based, and the collapse it forbids is one the codebase's own SQL style would wrap across lines

#272 fixed the spelling axis completely. The remaining axis is layout: the guard iterates `text.lines()` and normalises **each line independently**, so a call split across lines is never assembled and the argument is never seen.

Tested against the shipped matcher (faithful port, with a control):

```
single line, qualified: BOOL_OR(a.is_bot)   → CAUGHT   (control — port is faithful)
multi-line:  BOOL_OR(\n    is_bot\n)        → MISSED
```

This is not a theoretical layout. It is the house style for long expressions in the very directory the guard scans — `analyses/ownership.rs:53-55` writes `SUM(` on one line, its argument on the next, and `)` on a third, in a query that is otherwise dense single-line SQL. A future `BOOL_OR(` whose argument is long enough to wrap would be formatted exactly this way, by the same convention, and would be invisible.

**Fix:** normalise the whole file once and record line numbers by byte offset — the same technique `spa_escaping_test` already uses to report a file line from a statement offset, so the pattern is in-tree and proven. **Severity Low** on this engagement's own rule: nothing is exploitable, the tree is clean, and the class is correctness (a canonical mixing human and bot aliases is misclassified, which moves ownership numbers feeding `code_familiarity_min`) rather than security. It is the same shape and severity as F300.

I want to be clear about what this is: applying §12's standard to the guard that §12's own procedure had missed, one level down. That the answer is "yes, one more layer" is not a criticism of #272 — it is what the standard is for, and the fix is smaller than the one it follows.

---

## 4. Residuals and currency

**Decided and deferred with reasons (per #271) — I concur with all of these:** Rust 1.97.1 to the next cut by documented convention; CSP open as hash-based-or-nothing, with the zero-inline-handlers premise re-verified (and a first grep that suggested otherwise correctly attributed to the pattern, not the claim); F215 closed as not worth a refactor at one site.

**Genuinely open:** the gitlink differential fixture (0 refs — carried since cycle 6, now by a wide margin the oldest item, and the only one with no decision recorded against it); `outputSchema` at 1 of 11 tools; M8 MCP cancellation (0 `RequestContext`, a design question per E9). **Disclosed by the project, worth restating:** zizmor is not yet a required context in `protect-main`, so it reddens a PR without refusing the merge — a one-line ruleset change, and the reason it was made a separately-named job.

**Currency, re-verified live 2026-08-13:** rmcp lockfile `3.1.2` = latest ✅; zizmor pinned `1.29.0` = latest ✅; Rust pin `1.96.0` against 1.97.1 stable, deferred by decision, not drift.

---

## 5. Honesty ledger

- **The Trusted Publishing recommendation was wrong for four cycles, and wrong in the specific way this engagement exists to catch.** I checked it against industry best practice (OIDC over long-lived secrets — true in general) and never checked it against the other thing I was recommending in the same reports (signing credentials unreachable from build code). Two individually-correct recommendations, mutually incompatible, unnoticed across cycles 6, 8, 9, 11 and 12. The brief's line — *make sure all components, everything is fully aligned with each other* — applies to the audit's own output, and mine was not. Standing rule: **before repeating a carried recommendation, check it against every other recommendation still carried in the same report.**
- **I miscounted the accessibility items and caught it myself** (§1). Recorded rather than silently corrected, per the convention both sides now follow.
- **This cycle's main contribution is a resolution, not a discovery.** The insight that OIDC and L3 conflict is theirs; mine is only that `--no-verify` separates the two behaviours, tested. Worth stating plainly so the credit lands where it belongs.
- **Limits.** The audit subject was not compiled; the `cargo` runs in §2 were against a throwaway crate on `cargo 1.95.0` (the workspace pins 1.96.0), establishing cargo's *behaviour*, not this workspace's build. The §3 evasion is established by porting the matcher and running it with a control, not by modifying the tree and running `cargo test`. zizmor's finding counts (111/44 → 74/7) are read from #273's commit message, not reproduced.

---

## 6. Housekeeping

- Branches: `main`, `gh-pages` (actively published), and **`docs/hardening-cycle-12` is still unmerged** — cycle 12's report has not landed on `main`, unlike cycles 9–11. Worth a PR or a deliberate decision to drop it; F304's numbering suggests the ledger already absorbed its content.
- `_to_delete/` now carries `cycle12/` and `cycle13/` (`cl276.tar`, `delta13.patch`). `rm -rf _to_delete` when convenient. `HANDOFF.md` remains yours.
- **This report** is committed to branch `docs/hardening-cycle-13`, based on `main` (`67548c2`).

---

## 7. Method

Four commits, all read at source. The delta's fixes were each re-verified against the finding they close — #272 by porting the new matcher and exercising it (which is also what produced §3), M27 by counting the actual elements (twice, the first time wrongly). The cycle's main question — whether the Trusted Publishing rejection is inherent or contingent — was settled by running `cargo` against a purpose-built crate in both configurations with a control, rather than by reasoning about documentation, because an untested prescription is exactly what the last two cycles caught me shipping. Currency re-verified live at report time.
