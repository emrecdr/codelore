# Hardening cycle 8 — the delta that closed the book

**Anchor:** `e788aec` (v0.27.2) · **Baseline:** `a113566` (cycle-7 anchor, v0.27.0 + #223) · **Delta:** 20 commits (PRs #224–#241) spanning the v0.27.1 and v0.27.2 cuts, plus the new `v1` Action ref.

Audited from `git archive main` staged read-only into the audit workspace, with merge-safety probes run against the live repository objects. `main` = `origin/main` (last-known) = `e788aec`; the working tree is clean apart from the untracked `HANDOFF.md`. Nothing compiled — the toolchain pin (`1.96.0`) is unreachable from the audit host — so every claim is source-anchored. The report is short because the findings are few, and that is the report.

---

## 0. What this cycle actually is

Cycle 7 validated five in-flight fix branches and said: the fixes are right, rebase the two stale ones before merging, commit M1 with a test, implement M2 or rename the branch. This delta did all of it, and then kept going: the twenty commits close not only the cycle-6/7 High and Medium sets but most of the deferred-residual tail, land all four hardening reports on `main`, re-architect release provenance, and fix a defect no audit cycle had caught (F287). Two patch releases shipped same-day with CHANGELOG entries that — checked line by line against the code — are accurate, complete, and disclose their own prior failures.

**The merge landed whole.** All four cycle-7 §2 collateral probes pass on `main`: #223's zero-row-notice fix survived (the `--min-revs 1` remedy is absent from `analyze.rs`), `health_trend_test.rs` exists with its 114 lines of invariants, the README's `max_red_effort_pct` example is present, and #223's CHANGELOG entry is intact. The rebase-not-merge guidance was followed and executed correctly.

For the first time in eight cycles, the fresh-audit pass produced **no new High and no new Medium**. What remains is a short residual tail, one three-cycle streak worth naming, and currency options.

---

## 1. Verdict table — everything in flight, against v0.27.2

| Item | Verdict |
|---|---|
| C6-H1 Action `version` injection | ✅ Landed (#224) — both barriers in `action.yml` on `main` |
| C6-H2 trend workflow `format: html` | ✅ Landed (#224) — `markdown` + capability note in the guide |
| C6-H3 `--fail-on` exit-code docs | ✅ Landed (#225) — docs say exit 1 with the taxonomy reason |
| C6-H4 `delta_health` rename/deletion | ✅ Landed (#227) — base∪head union, residual disclosed |
| C6-H5 `--no-cache` on flagless surfaces | ✅ Landed (#225) — all three strings name `--cache-dir` |
| C6-M1 head-only witness double-scan | ✅ Landed (#228) **with the test cycle 7 asked for** (`head_only_ingest_persists_its_cache_entry`) |
| C6-M2 ratchet records unconfigured gate | ✅ Landed (#228) — `code_health_min.is_some()` guard (`check.rs:182`), README claim now true |
| C6-M3 SARIF `(skipped)` phantom URI | ✅ Landed (#225), then **class-closed** by #232 |
| C6-M4 MCP "Read-only" overclaim | ✅ Landed (#226) — exact recommended wording |
| C6-M5 / M6 Action-guide examples | ✅ Landed (#225) — `sarif` for check; `clones` in the matrix |
| C6-M7 publish probe treats 5xx as absent | ✅ Landed (#230) — status-branched: 200 skip / 404 publish / else **fail closed** (`release.yml:427-441`) |
| C6-M10 hotspots truncation disclosure | ✅ Landed (#226) — `{omitted, total, note}` via the shared helper |
| C6-M12 differential fixtures | ✅ **3 of 4** (#229) — real binary (NUL + high bytes), non-ASCII path *and* filename (`документы`), CRLF with `autocrlf` pinned off. Gitlink/submodule still absent (§3.3) |
| C6-M13 tag-pinned third-party actions | ✅ Landed (#230) — zero float remaining across all workflows, **plus a guard test** so the policy is enforced, not remembered |
| C6-M16 ledger `Fixed (Unreleased)` rot | ✅ **Root-caused and closed** (#231→#234→#237): re-stamp at cut, staged with the release commit, guarded by a completeness check. Verified consistent: 0 unbacked rows, 26 rows stamped to their shipped versions, `[Unreleased]` empty |
| C6-M9 residual (negotiated protocolVersion) | ✅ Landed — `mcp_test.rs:106` pins the **negotiated** revision with a comment explaining why |
| C7-§4.1 sentinel allowlist drift | ✅ Landed (#232) — canonical `PSEUDO_PATHS` + `is_pseudo_path()` in the gate layer; the SARIF emitter (`sarif.rs:796`) and `check.rs:984` both consult it; the regression test iterates the canonical list, and the CHANGELOG records that it was verified by adding a seventh sentinel and confirming zero other edits were needed |
| C7-§4.2 `health_trend_test.rs` deletion risk | ✅ Averted — file alive on `main`, invariants intact |
| C7-§2 branch hygiene (rebase not merge) | ✅ Followed — all four collateral probes pass; every `fix/*` and `docs/*` branch merged and deleted |
| F287 `@v1` does not resolve | ✅ Fixed (#241) — `v1` tag exists at v0.27.1 (byte-identical `action.yml`), re-pointed per release inside the ruleset window, non-fatal-but-loud failure path with exact repair steps |

Beyond the punch list, the delta also landed: SLSA provenance isolation (#239, §4), tag-gating for every publishing job (#240), a release-commit completeness guard (#237), and all four hardening reports on `main` (#236, #238) — closing the "reports live on one machine" housekeeping thread that ran since cycle 4.

---

## 2. Still open (carried, unchanged unless noted)

- **M8 — no MCP handler observes cancellation.** Zero `RequestContext`/token reads in `mcp.rs`; a cancelled cold call still holds one of four semaphore permits to ingest completion. Unchanged from cycle 6.
- **M11 (narrowed) — two tools still return unbounded arrays.** The cap-and-disclose regime now covers hotspots, `code_health`, `delta_health`, `refactoring_targets`, `finding_hotspot_overlap` and `function_xray` — but **`check_gates` and `gate_changes`** still emit one row per violation with no cap, which on a large repo with a tight gate is whole-population output into an agent's context.
- **M14 — `calibrate-defects` mining ingest remains unguarded** (the honesty floor on *weights* still bounds the damage; the ingest itself has no truncation witness).
- **M15 — `output/sqlite.rs` still has no offline hint** for the extension-download failure.
- **L1 — see §3.1; now a three-cycle streak.**

That is the whole open list from two cycles of findings. Everything else is closed.

---

## 3. Findings this cycle

### 3.1 (Low, third cycle) `refactoring_targets` still tells agents "(default: all)" — while its handler now caps at 50

`mcp.rs:467`: `/// Maximum rows to return (default: all).` This line has now survived cycle 6 (filed L1), cycle 7 (refiled), and — the new part — the #226 cap work, which routed `refactoring_targets` through `resolve_row_cap` (default 50, clamp 1..=500). So the self-description went from *stale* to *false*: an agent reading the schema plans around "all", gets 50. The mitigations are real — the response now carries the `{omitted, total}` object, so the truncation is disclosed after the fact — which is why this stays Low. But the fix is one line, the struct **directly above it** (`FunctionXrayParams`, `:460-461`) shows the exact correct wording, and a finding surviving three cycles while its surrounding code was twice reworked is a process observation as much as a defect: items filed Low appear to fall off the punch list entirely. Consider sweeping the remaining cycle-6 Lows (they are enumerated in that report's §4) in one mechanical pass.

### 3.2 (carried, sharpened) `gate_changes` is the last uncapped violation-shaped output

With #226's disclosure convention now uniform across the list tools, the two gate-verdict tools are the odd ones out (M11 above). `gate_changes` is the sharper of the two: it is the tool an agent calls *in a loop* while iterating on a PR, and its output scales with violation count. Routing both through `serialize_capped_rows` is mechanical; the helper exists and six tools demonstrate the pattern.

### 3.3 (Low) The differential oracle's fourth content class — gitlinks — is still unprobed

#229's fixture is real and well-built (the `autocrlf` pin and the `text`-attribute comment show the failure modes were understood, not just checked off). But of the four classes cycle 6 named, the submodule **gitlink** (mode `160000`) is the one still absent — and it is the class most likely to diverge between gix and git-CLI walkers, because a gitlink is a tree entry whose object is *absent from the repository*. One fixture commit with a fake gitlink entry closes it.

### 3.4 (informational) The SLSA L3 claim — assessed, and it holds up with one caveat

Cycle 6 flagged "SLSA L3" as an overclaim (the inline `attest-build-provenance` usage is Build L2); the research pass confirmed L2. #239's response was not to soften the comment but to **re-architect until the claim is true**: signing moved to two reusable trusted-signer workflows (`attest-artifact.yml`, `attest-digest.yml`) that contain **no `run:` step and no repository-authored code**; every build/package job is stripped to `contents: read`; workflow-level defaults no longer grant signing scopes (so a future job cannot inherit them); digests are computed *in the trusted job after download*, never passed across the job boundary; `release` depends on `attest` so a failed attestation blocks publication; and the CHANGELOG documents the verify command with `--signer-workflow` pinning. Checked against the SLSA v1.0 requirement it quotes ("prevent secret material used to sign the provenance from being accessible to the user-defined build steps"), the architecture satisfies it — this is the same isolation pattern the community `slsa-github-generator` uses. The one honest caveat: **L3 here is self-assessed.** A verifier whose policy accepts only the community generator's known builder identity will not recognize this signer; consumers must pin `emrecdr/codelore/.github/workflows/attest-artifact.yml` as the CHANGELOG instructs. That trade-off (own signer identity, documented verify path) is defensible and disclosed — recorded here as context, not as a defect.

### 3.5 (informational) The v1 floating ref: design reviewed, no defect

`cut-release.sh:755-779` re-points `v1` inside the same ruleset window the release tags use (required: `protect-release-tags` matches `refs/tags/v*` and forbids non-fast-forward *and* deletion, so out-of-window automation would have failed on the second release — the commit message shows this was reasoned through, not stumbled into). Failure to move `v1` is deliberately non-fatal but loud, with exact repair commands — the right call, since aborting would strand a release mid-flight, and a stale `v1` (consumers silently on the old version) is precisely the failure mode the warning shouts about. The residual skew window between binary publication and the `v1` re-point is seconds and inherent to floating refs. No finding.

---

## 4. The honesty ledger

**F287 is a miss this engagement owns.** Every documented invocation was `uses: emrecdr/codelore@v1` — thirteen references — while no `v1` ref existed; a copied workflow failed at resolution, before any of the audited mechanics could run. Two cycles examined `action.yml`'s *contents* (injection, inputs, install verification) and the examples' *flag correctness* (H2, M5, M6), and never checked that the ref the examples fetch **resolves**. The commit message's diagnosis is exact: CI exercises the action as `uses: ./`, which proves the mechanics and says nothing about consumer reachability. The generalizable lesson, added to the standing method: *for every documented invocation, verify the thing it names exists — not just that its contents are correct.* (This is the doc-equivalent of the "check every consumer" lesson from cycles 5–7.)

**The one-step-short pattern appeared again — and was then closed as a class.** #231 built the ledger re-stamp; the v0.27.1 cut then re-stamped the ledger but **did not stage the file** — a sixth file against a five-file `git add` list (#234's fix). That is the fourth consecutive cycle in which a fix stopped one step short of a consumer. The difference this time: #237 then added the guard (`git diff --quiet` on tracked files after the explicit `git add`) that makes the *class* unshippable, with a comment naming the ledger instance as the motivating case. This is what closing a pattern, rather than an instance, looks like.

**Refutations and checks that came back clean:** the rebased #224/#225 landings were spot-checked against the cycle-7-validated branch diffs — no drift in the load-bearing hunks; `is_pseudo_path` is consulted by *both* prior copy sites (emitter and evidence-collection loop), not just one; the #230 probe's `else` branch fails closed *without* publishing (re-read to confirm no fall-through); the M1 witness preserves `commit_count` gating on the full-ingest path (re-confirmed at `facts/mod.rs:443-448`); the 0.27.1 CHANGELOG was read entry-by-entry against the code and **no entry overclaims** — including honest residual disclosures in the `delta_health` entry (renames still read as remove-plus-add) and the head-only entry (the blind-ingest guard still holds).

**Limits.** Nothing compiled; the M1/M2 tests and the guard test are read, not run. `attest-digest.yml` (the container-side signer) was verified structurally via the CHANGELOG's description and its sibling's full read, not line-by-line. `origin/main` is last-known (the audit host cannot fetch); the local clean checkout equals it. rmcp latest re-verified against the crates.io API today (3.1.1, 2026-08-05); other §5 currency values carry from the 2026-08-06 verified briefing.

---

## 5. Improvement options (all currency/adoption, none defects)

- **rmcp `3.1.0` → `3.1.1`** (re-verified today via crates.io). One-line workspace bump.
- **Declare `outputSchema` / `structuredContent` on the eleven MCP tools** — zero adoption today (`mcp.rs` has no output-schema wiring). With annotations done (#226) and caps uniform (#226/#232), this is the remaining gap between this server and current MCP best practice, and the highest-leverage MCP item left: agents get typed results instead of parsing JSON out of text blocks. Pairs naturally with the rmcp bump.
- **`zizmor` in CI** — not adopted. The pinning *guard test* (#230) covers one of its audit classes; zizmor would also gate template-injection and permissions drift, the other two classes this engagement found by hand.
- **Trusted Publishing (OIDC) for crates.io** — the publish job still uses `CRATES_IO_TOKEN` (correctly env-guarded). `rust-lang/crates-io-auth-action` is GA; adopting it retires the standing secret and the token-emptiness conditional.
- **Rust pin `1.96.0` → `1.97.1`** (1.98 stable lands ~2026-08-20; the pin-agreement test makes the bump a six-site mechanical change).
- **Gitlink fixture** (§3.3) and **`gate_changes`/`check_gates` caps** (§3.2) — the two smallest items that close cycle-6 residuals for good.

---

## 6. Housekeeping

- **All prior housekeeping is done** — a first: the four report branches are merged to `main` and deleted (#236, #238), the five fix branches are merged and deleted, `_to_delete/` was cleared before this cycle, and the branch list is down to `main` + `gh-pages`.
- **`gh-pages` is 129 commits behind** — untouched since cycle 5's landing-page refresh; refresh when convenient, or note it as release-automated if that is intended.
- **This cycle's audit artifacts** (`cl272.tar`, `delta8.diff`, `delta8.log`, ~9.6 MB) are in `_to_delete/cycle8-audit-artifacts/` — the bridge cannot unlink; `rm -rf _to_delete` when convenient. `HANDOFF.md` remains untracked and yours.
- **`.git/objects/*/tmp_obj_*` strays** accumulate a few per bridge commit as before; `find .git/objects -name 'tmp_obj_*' -delete` clears them.
- **This report** is committed to branch `docs/hardening-cycle-8`, based on `main` (`e788aec`). Given the new convention of landing reports via PR (#236), it is left on its branch for you to merge the same way.

---

## 7. Method

Merge-safety probes against the live repo objects (the four cycle-7 collateral files); full tree staged from `git archive main` and read in the audit workspace; every landed fix re-verified at its final anchor (not assumed from the cycle-7 branch validation — the rebase could have drifted); the genuinely new code (#228–#241: witness, ratchet scope, fixtures, probe, pinning guard, ledger re-stamp chain, provenance isolation, tag-gating, v1 mechanics) read directly; CHANGELOG 0.27.1/0.27.2 checked entry-by-entry against the code; residuals swept by anchor; currency re-verified where the API is reachable (crates.io) and carried from the day-old verified briefing otherwise. Adversarial validation with a default verdict of REFUTED — which this cycle mostly meant *confirming closures* rather than killing findings, and the two candidates it did kill (a suspected v1-skew defect, a suspected pseudo-path consumer miss) are recorded above as design-reviewed non-findings.
