# Hardening cycle 12 — right finding, wrong reason: adjudicating a correction of my own work

**Anchor:** `267426a` (main) · **Baseline:** `6b6cc6d` (cycle-11 anchor, v0.27.4 + #260) · **Delta:** 9 commits (#261–#269), no release cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `267426a`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled. The delta is unusual: **eight of the nine commits are self-directed correction** — of the guard I asked for, of my explanation of why it was needed, of the project's own ledger, and of a correction the project itself had shipped wrongly. So this cycle is mostly adjudication, and most of it lands against me.

---

## 0. What this cycle actually is

#262 landed my cycle-11 report with a correction attached: **the finding survived, the explanation did not.** I verified that correction line by line before accepting it, and it is right on every checkable point. The short version: I said four author sinks escaped the guard because of its *accessor* list and prescribed widening that list; measured against the real tree, **three of the four escaped because of its *marker* list** — their statements build markup from `<h4>/<dl>/<dt>/<dd>` and `<tr>/<td>`, none of which the guard recognised, so those statements were never scanned and no accessor inside them was ever examined. My prescribed fix would have caught **zero** of the three.

Worse, and this is the part worth keeping: **my own report contained the disproof.** I ran a broadened-marker probe as one of three, got three hits, judged them false positives, and concluded the marker list "hides no live defect." That conclusion was true and irrelevant. I asked *"do these markers reveal an unescaped string today?"* when the finding was about reach, and the question that mattered was *"do these markers change what the guard can see?"* Right experiment, wrong question asked of the result — #262's phrasing, and it is exact.

The rest of the delta is the project turning cycle-11's closing generalisation into a procedure and running it against its own thirteen guards (§12 of the ledger, F300–F303), then correcting its own count when the first pass reported four probed as exhaustive and it was six. That produced F303, which is the best finding in the delta by anyone (§3).

---

## 1. Adjudicating #262's corrections against me

Each checkable claim, tested here before acceptance:

| Correction | Verdict |
|---|---|
| Three of four sinks are **marker**-list failures, not accessor | ✅ **Confirmed.** Extracted the real statements: `12_drawer.js:404`, `14_widgets_summary.js:178`, `20_hotspots.js:771` contain **no marker at all** from the shipped list → never scanned |
| My exploit command doesn't run | ✅ **Confirmed empirically.** `git commit --author='<img …> <a@b.c>'` → `fatal: empty ident name`. Git rejects a name beginning with `<` |
| The real vector is quote-breakout in attribute context | ✅ **Confirmed.** `git commit --author='ev"il <a@b.c>'` is accepted verbatim; author renders `ev"il`. And it lands in `data-primary-author="…"` — attribute context |
| "snake_case compound" over-stated the gap | ✅ **Confirmed.** `.entity` *does* substring-match `.entity_a`. The gap is only where the compound's **first** segment is unlisted (`main_author`), not compounds generally |
| Five listed payload fields don't exist | ⚖️ **Retracted by the project itself** (#266) — all five are `pub …: String` on analysis row types; my original characterisation was correct |
| CSP re-proposed without citing its prior rejection | ✅ **Confirmed.** Ledger line 1361 records it researched and rejected at cycle 10: `unsafe-inline` "would have given F295's payload no protection while reading, in review, like a mitigation." Re-proposing it uncited is the failure mode the ledger exists to prevent |

On the CSP the project also did something I want to name, because it is the harder half of good adjudication: it kept the one axis of my argument that was new — that `default-src 'none'` with no `connect-src` constrains *exfiltration* even where it cannot block execution — while rejecting the policy I attached it to. "An argument for a policy, not for that policy. The open decision is the hash-based form or nothing." That is the correct disposition and I withdraw the recommendation as written.

**#266 is the commit I'd single out.** It retracts a correction the project had itself shipped against my report, and records *why* it happened: "a grep scoped to `src/output/` whose empty result was generalised to 'the crate.' A narrow probe reported as a broad conclusion is the failure mode the report is about, so producing one while correcting one is recorded rather than quietly fixed." Both sides of this exchange have now made the same error in the same week. That symmetry is worth more than either finding.

---

## 2. Verifying the fix (#261), and withdrawing my prescription for the residual

**The fix is better than what I proposed.** Rather than enumerating more tags, it generalised markup detection: `literal_tags()` extracts any `<name` whose `<` is preceded by a quote — so it only fires inside string literals, not on `a < b` — and tests it against a 33-tag allowlist, with `HTML_MARKERS` cut back to the entity/attribute cases tags can't cover. It also adds `_author`/`_path`/`_name` suffixes and `.function` (F300), and `is_escaped` was rewritten to walk paren depth to the *enclosing* call, so `escapeHtml(a || b.name)` now reads as escaped.

Verified by porting the current Rust faithfully and running it:

- **Current tree: 0 violations.** (My first port reported 1 — `30_coupling_trends.js:241` — which was my port being stale on the old `is_escaped`. The shipped guard is correct; I am recording the false alarm rather than omitting it.)
- **Against the four sinks regressed:** `drawer main_author` ✅ caught, `summary main_author` ✅ caught, `hotspots rowAuthor` ✅ caught, `drawer partnerAuthor` ❌ missed.
- **Isolated regression** (everything else escaped, only the local raw): `rowAuthor` ❌ **missed** — its catch above came from the `.path` sitting in the same statement, not from the author.

So the residual is precise: **the two locals remain invisible**, and one of them (`rowAuthor`, `20_hotspots.js:772`) is the sink in attribute context — the exact place the quote-breakout vector lands. Both are correctly escaped today; nothing is exploitable.

**I tested my cycle-11 prescription for this before repeating it, and it does not work.** Adding camelCase `Author`/`Path` accessors produces four hits on the current tree, all false positives: a comment, the literal column header `'<th scope="col">Path</th>'`, a ternary truthiness test, and the numeric local `activeAuthors`. Bare-word accessors collide with literal markup text and with non-string locals, and separating them needs a tokeniser — precisely the "convention guard, not a taint tracker" line the guard correctly draws. **Withdrawn.**

What I'd offer instead, sized to the actual residual: there are exactly **two** such locals in the tree. A targeted assertion that those two lines contain `escapeHtml(` pins the known instances at zero false-positive cost. It is an instance list — but an *honest* one, covering a class the general rule provably cannot reach, which is a different thing from an instance list masquerading as a rule. That distinction is §12's own, and it argues for stating the two sites in the guard's limits section rather than pretending the rule covers them.

---

## 3. New finding

### F — LOW (new) — The guard census in §12 missed `bot_filter_hygiene_test`, which is an instance list this engagement flagged four cycles ago

§12 ran cycle-11's generalisation as a procedure against "all thirteen guards in the tree," found six carrying instance lists, and — after #269 corrected its own count from four — probed all six. `bot_filter_hygiene_test.rs` **is named nowhere in §12**, and it is a textbook instance list:

```rust
if line.contains("BOOL_OR(is_bot)") || line.contains("HAVING NOT BOOL_OR") {
```

Two case-sensitive literals, scanned over one directory (`SCANNED = "crates/codelore-lib/src/analyses"`). Applying §12's own standard — *name a member of the class it polices that it would not catch; if that is easy, the rule is an instance list* — it is easy four ways: `bool_or(is_bot)` (SQL is case-insensitive, so the lowercase spelling is functionally identical and invisible), `BOOL_OR( is_bot )` (whitespace), `BOOL_OR(a.is_bot)` (qualified column), and the same pattern anywhere outside `src/analyses/`.

This is the guard cycle 6 filed as **M17**, recommending it be broadened to a tolerant pattern. It is unchanged. The class it polices is correctness rather than security — a canonical identity mixing human and bot aliases gets silently misclassified, which moves author/ownership numbers that feed `code_familiarity_min` — so **Low**, matching how F300 (identical shape) was rated. The tree is clean today: the only `BOOL_OR(is_bot)` occurrences are in `query.rs`'s explanatory doc comment, which is the documented exemption.

**Refuted while checking this:** M17's second half — that `pair_programming.rs` keeps a Rust-side bot filter that the guard cannot see — is no longer a defect. The file now carries an explicit rationale (`:16-22`): it reads `commits` directly rather than through `HUMAN_ALIASES_CTE`, so the `is_bot` checks "are the only thing keeping bots out of the pair counts." That is a documented design decision, not an oversight. Closed by rationale.

**Credit where it's due — F303 is the delta's best finding, by anyone.** The signing-isolation guard parsed `permissions:` line by line and so recognised one of three ways GitHub grants a scope; `permissions: write-all` grants every scope including the signing ones while naming none, and both the block parser and the flow map returned zero. As the commit notes, `write-all` is the reflex fix when an attestation step fails for want of a permission — so the blind spot sat exactly where the pressure to take it is highest, and taking it would silently drop the pipeline from SLSA L3 to L2. No workflow used it; the tripwire was what was missing. That is the guard-audit procedure paying for itself.

---

## 4. Residuals and currency

**Unchanged and open:** the gitlink differential fixture (0 refs — now by a wide margin the oldest untouched item, carried since cycle 6 M12); `outputSchema` at 1 of 11 tools (per-tool design work, E8); M8 MCP cancellation (0 `RequestContext`; a design question per E9, not a wiring one); `zizmor` unadopted; crates.io Trusted Publishing unadopted.

**Currency, re-verified live 2026-08-13** (per the standing rule — at report time, not engagement time): rmcp lockfile `3.1.2` = live latest ✅. Rust pin remains `1.96.0`; 1.97.1 is current and 1.98 lands ~2026-08-27, so the pin is now two releases behind and the bump is guarded by `rust_version_pins_test` across the five sites it names (`rust-toolchain.toml`, the workspace `rust-version`, `clippy.toml`'s msrv, the `Containerfile` ARG, and the action tags) — corrected from "six-site" on verification — this is the cheapest open item on the list.

---

## 5. Honesty ledger

- **My cycle-11 explanation was wrong and my report contained its own disproof.** The finding stood; the mechanism I gave for it was mis-attributed, and the fix I prescribed would have caught none of the three sinks it was aimed at. Cause: I asked a live-exploitability question of a reach probe. The standing rule I'd add: **when a finding is about a guard's reach, every probe must be scored on reach, not on whether it surfaces a live defect.**
- **My prescription was untested — again.** Cycle 11's ledger already recorded that cycle 10 "under-specified its own recommendation," and I then shipped another untested prescription in the same report. This cycle I tested it before repeating it, and it failed. That is the process working, one cycle later than it should have.
- **I recorded a false alarm rather than deleting it** (§2, the stale-port "1 violation"). Same reason the project recorded #266.
- **Limits.** Nothing compiled: all guard behaviour is established by porting the Rust to Python and running it over the real widget corpus, with the shipped guard's own result (0 violations, clean tree) as the control that the port is faithful — the check my cycle-11 port failed. #263/#265/#267/#268 were read at commit-message and spot-diff level, not line by line. The `bot_filter_hygiene` evasion cases are reasoned from the matcher source, not executed against a modified tree.

---

## 6. Housekeeping

- Branches: `main` + `gh-pages` (actively published — 5 behind locally is the publishing job, not staleness).
- `_to_delete/` now shows as untracked in `git status`; this cycle added `_to_delete/cycle12/` (`cl275.tar`, `delta12.patch`). `rm -rf _to_delete` when convenient — the bridge cannot unlink. `HANDOFF.md` remains yours.
- **This report** is committed to branch `docs/hardening-cycle-12`, based on `main` (`267426a`), for landing via PR per the established convention.

---

## 7. Method

The delta was small and mostly self-correction, so the budget went to adjudicating the correction of my own work rather than re-auditing code: every checkable claim in #262 was tested here before acceptance (statement extraction from the pre-fix tree for the marker/accessor question; a real `git commit` for both author-name vectors; substring tests for the prefix claim; a ledger read for the CSP rejection), and one was found already retracted by the project. The shipped guard was re-ported faithfully — validated against its own clean-tree result before any conclusion was drawn from it — and exercised against the four sinks in isolated regression. My own prescription was tested and withdrawn. The one new finding came from applying §12's standard to the guard census itself and noticing an absence.
