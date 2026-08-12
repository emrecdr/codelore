# Hardening cycle 11 — the fix is right; the guard is narrower than the class

**Anchor:** `6b6cc6d` (main; v0.27.4 + #260) · **Baseline:** `e62763c` (cycle-10 anchor) · **Delta:** 7 commits (#255–#260) spanning the v0.27.4 cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `6b6cc6d`; tree clean but for the untracked `HANDOFF.md`. Cycle 10 reported a stored XSS (F1) and argued the deliverable was **a guard, not five point fixes**. Both shipped. This cycle verified the fix, then spent its budget adversarially testing **the guard itself** — by re-implementing its matcher and running it against inputs it does not list. That produced the finding.

The first draft of this report reached the right conclusion by the wrong route. Its finding — that the guard misses the author sinks — survived validation; its explanation of *why*, and the fix it prescribed, did not. Both are corrected below, and §4 records the miss. Findings carry ledger IDs rather than cycle-local names — the omission #259 had to repair for cycle 10. The correction was established by compiling and running the guard, which the audit pass could not do.

---

## 0. What this cycle actually is

The remediation is better than what cycle 10 asked for, in three specific ways worth naming before the finding:

1. **It was validated against a running binary, not just source.** #257 reproduced the payload end-to-end — "a fixture repo with such a directory emits the payload unaltered into the dashboard's JSON block" — which is a stronger standard than the audit pass could apply.
2. **It found a defense I had not evaluated, and correctly ruled it insufficient.** The emitter rewrites `</` → `<\/` in the JSON block. #257 identifies this as transport-level — it stops `</script>` breakout and "has no jurisdiction over what happens after `JSON.parse`", where the string re-enters concatenation intact, and a payload without `</` passes it untouched. That reasoning is right, and it is the argument for fixing at the sink. I had checked for a CSP and not for this.
3. **#260 then closed a duplication *inside the guard*** — the guard listed the ten widget files in its own table, duplicating the emitter's list; it now reads the directory, with a non-empty assertion so a broken path resolver cannot masquerade as a clean run. That is the project's signature move applied reflexively, to its own new test.

F2 (the Markdown tag column) landed in the same commit. Currency closed itself: the dependency group took **rmcp to 3.1.2** (`Cargo.lock`), matching the live registry as of today.

So: the code is correct. The finding below is about the *guard's reach*.

---

## 1. Finding

### F297 (Fixed — Unreleased) — MEDIUM — Four of the five author sinks are invisible to the guard, and the marker list — not the accessor list — is why

All five author sinks in the dashboard are correctly escaped today; each was read at source. **There is no live XSS.** The defect is that the guard built to stop this class from recurring does not watch the field class most likely to carry it.

The guard tests two things about a statement: that it *builds markup* (`HTML_MARKERS`, a ten-substring list) and that each *raw-string accessor* in it sits inside `escapeHtml(...)` (`RAW_STRING_ACCESSORS`, matched as a plain substring). A sink escapes notice if **either** test fails to see it. Measured per sink, by compiling the guard and running it against each sink regressed into unescaped form:

| Sink (real site) | Form | statement seen as markup? | dotted anchor present? | verdict |
|---|---|---|---|---|
| `12_drawer.js:406` `ki.main_author` | field | **no** | no | MISSED |
| `14_widgets_summary.js:181` `r.main_author` | field | **no** | no | MISSED |
| `20_hotspots.js:772` `rowAuthor` | local | **no** | no | MISSED |
| `12_drawer.js:433` `partnerAuthor` | local | yes | no | MISSED |
| `12_drawer.js:482` `c.author` | field | yes | yes | CAUGHT |

**Three of the four misses are marker-list failures**, not accessor failures: those statements build their markup out of `<h4>`, `<dl>`, `<dt>`, `<dd>`, `<tr>` and `<td>`, none of which appear in `HTML_MARKERS` — so the statements were never examined at all, and *every* accessor in them, including the plain `.path` at `20_hotspots.js:772`, went unchecked. Only `partnerAuthor` fails for the accessor reason.

The accessor gap is real but narrower than "snake_case compounds": matching is by dotted prefix, so `.entity` already covers `.entity_a` and `.entity_b` (28 uses) because the base name comes first. What is missed is the **qualifier-first** compound — `.author` does not occur in `main_author`, since the character before `author` is `_`.

**Why author names are the sharpest case.** Cycle-10's F1 required a path containing `<`, legal only on some filesystems and requiring the attacker to control a directory name. Author names are easier, but not in the way the first draft claimed: `git commit --author='<img src=x onerror=…> <a@b.c>'` **fails** — `fatal: empty ident name` — because git parses the leading `<` as the start of the address, and a mid-string `<` is consumed as the delimiter, landing the remainder in the *email* field. What git does accept, verbatim in both name and email, is a **quote**:

```
git commit --author='x" onerror="alert(1) <a@b.c>'   →  %an = x" onerror="alert(1)
```

That is an attribute-context breakout, and it needs no angle brackets at all. It matters because author identity is rendered into an HTML **attribute** — `20_hotspots.js:772` builds `data-primary-author="…"` — and that sink is one the guard could not see on either axis. It is safe today only because `escapeHtml` escapes `"` and `'` as well as `<`/`>`.

**Fix (landed).** Three coordinated changes; the accessor change alone closes nothing, which was measured before adopting it.

- **Markup detection.** An opening tag *inside a string literal*, rather than a substring anywhere in the statement. Both halves of that rule pay for themselves: without the literal anchor, `evt.target` beside a `j < n` comparison reads as markup; without a tag-name check, the literal `'<anonymous>'` — the X-ray widget's placeholder for an unnamed function — reads as an opening tag. Each was an observed false positive, not a hypothetical.
- **Escaping detection.** Judged by walking back to the innermost unclosed `(`, not by testing the text adjacent to the accessor. `escapeHtml(a || b)` escapes both operands with one call; the adjacency test read the second as a bare sink and reported a correct file.
- **Accessors.** `_author`, `_path`, `_name` for qualifier-first compounds. Suffix compounds need no entry.

Measured on the widget corpus: **zero false positives**, and with both `main_author` sinks unescaped the previous guard passes while the new one fails naming both lines. Every design that also reaches the two camelCase locals was tested and turns the guard red on a clean tree — flagging a comment, and `partnerAuthor`'s own truthiness test — so **locals remain out of reach**. That is the across-statement-boundary limit the guard's doc already states, and it is now stated as covering this case explicitly rather than by implication.

### F298 (Fixed — Unreleased) — LOW — `preceding` could panic on a multi-byte terminator

`preceding` located the start of an identifier chain with `rfind(…).map_or(0, |i| i + 1)`, stepping one byte past a terminator that may be several bytes wide, then slicing there. The widgets contain `—` and `·`; a terminator like that would split mid-character and panic the guard rather than report on the file. No statement in the corpus reaches it — a probe over every accessor match found zero non-ASCII terminators — so it was latent, and it is normally the kind of thing this report records rather than fixes. It was fixed here because widening markup detection increases the number of statements walked, which is what changes its exposure.

---

## 2. The CSP question, against a decision already on the record

The draft recommended a `script-src 'unsafe-inline'` policy as the missing structural backstop. **That policy was already researched and deliberately rejected**, in the ledger's "Deferred from this cycle" note landed with the cycle-10 findings (#259) — on the day of this cycle's anchor. Its reasoning: `'unsafe-inline'` permits inline event handlers, so the policy would have given F295's payload no protection while reading, in review, like a mitigation. The generalisation recorded there stands: *a control that cannot block the finding that motivated it is not defence in depth, it is decoration.*

Re-proposing it without citing that decision is the failure mode the ledger exists to prevent, and this report reproduced it.

One axis the recorded rejection did not weigh: it scored the policy on whether it blocks *execution*, and a `default-src 'none'` policy with no `connect-src` and no remote `img-src` also constrains *exfiltration* — the difference between a payload that reads the dashboard and phones home, and one that can only deface. That is a real distinction, and it does not rescue `'unsafe-inline'`; it is an argument for a policy, not for that policy.

The ledger already names the viable form: a **hash-based policy**, which the template can carry because it has zero inline `on*=` attributes, so `'unsafe-hashes'` is not required. Its cost is a per-emit digest of every inline block including the interpolated data — an emitter change. **The open decision is hash-based policy or nothing; `'unsafe-inline'` is closed.**

---

## 3. Verified closed / still open

**Closed this delta:** cycle-10 F1 (XSS — five sinks escaped, guard added, validated against a binary); cycle-10 F2 (Markdown tag column); the guard's own file-list duplication (#260); **rmcp currency** (lockfile now 3.1.2 = live latest). **Closed by this cycle:** F297 (guard reach), F298 (multi-byte terminator).

**Still open, unchanged:** M8 MCP cancellation (0 `RequestContext`; reframed per cycle-9 E9 as a design question — the ingest is under non-abortable `spawn_blocking`); `outputSchema` at **1 of 11** tools (one `Json<GateSummary>` against eleven `#[tool]` declarations; per-tool design work per E8); the **gitlink differential fixture** — note that gitlink *handling* exists in both backends and carries three unit tests, so what is absent is specifically a fixture in the differential bundle, which is why this has stayed cheap to defer; `zizmor` (0 references under `.github/`) and crates.io Trusted Publishing, both unadopted; Rust pin `1.96.0` with 1.97.1 available (1.98 ~2026-08-27).

---

## 4. Honesty ledger

- **This report's first draft mis-diagnosed its own finding.** It attributed all four missed sinks to the accessor anchor and prescribed widening it. Measured against the regressed tree, that fix catches **zero of the four**: three of them are marker-list failures. The draft had the evidence — it broadened `HTML_MARKERS` as one of its three probes, got three hits, judged them all false positives, and concluded "the marker list, while not exhaustive, hides no live defect." That conclusion is true and irrelevant: the probe was scored for *live defects* when the finding being reported was about *reach*. Right experiment, wrong question asked of the result.
- **The exploit command in the draft does not run.** `git commit --author='<img …> <a@b.c>'` is rejected outright. The claim survived into a severity argument ("strictly easier to weaponise") without anyone running it. The underlying point holds via the quote vector, which is in fact sharper — but that was luck, not method.
- **Cycle 10 under-specified its own recommendation.** I proposed a guard that flags "an identifier not passing through `escapeHtml`." What shipped matches a *curated accessor list* against a *curated marker list*, a defensible narrowing that avoids flagging every numeric field — but it introduced both gaps in §1. A recommendation that had named the field set and markup set it must cover would have surfaced this before the code was written. Prescriptions need their acceptance criteria stated, not just their mechanism.
- **I checked for a CSP and missed the `</` → `<\/` transport defense.** #257 found it, evaluated it, and correctly ruled it insufficient. A real gap in cycle 10's evidence gathering: I enumerated the sinks and the absent backstop, but not the partial defense already in the emitter.
- **Not a live vulnerability, and §1 says so first.** Every author sink is escaped today. Reporting a guard gap at Medium — rather than dressing it as a High by implying exploitability — is the honest severity under this engagement's consumer-blast-radius rule: nothing is exploitable now; a silent regression is.
- **Draft fields that do not exist.** The draft listed `last_author`, `top_contributor`, `file_a`, `file_b` and `touched_file` as payload string fields the fix would prospectively cover. None of them appear anywhere in the crate. `main_author` is real; `src_path` is real and its single use is plumbing into `modulePath()`, not a sink.
- **Limits.** #255/#256 are dependency bumps reviewed at manifest/lockfile level, not by auditing upstream diffs. The audit pass itself compiled nothing; every claim above that changed as a result of compiling is marked as corrected rather than silently edited.

---

## 5. Housekeeping

- Branches: `main` + `gh-pages` (actively published; 2 commits behind locally, which is the publishing job, not staleness — per the cycle-9 E1 correction).
- This cycle's artifacts (`cl274.tar`, `delta11.patch`) are in `_to_delete/cycle11/` on the device; `rm -rf _to_delete` when convenient. `HANDOFF.md` remains untracked and yours.
- **This report** lands via PR per the #243/#254/#258 convention. The F297/F298 fix lands separately as a code change, since the report is a record and the guard is not.

---

## 6. Method

Small delta, so the budget went to adversarially testing the new guard rather than re-reading the code it protects: the matcher was re-implemented from its Rust source and exercised against (a) the real widget corpus as a control, (b) a broadened marker set, (c) an expanded accessor set, and (d) the tree's actual author sinks rewritten into their unescaped form. Every candidate was default-REFUTED.

Validation then repeated the exercise **against the compiled guard**, which is what separated the two mechanisms the audit pass had conflated: each candidate fix was scored on false positives across the whole corpus *and* on whether it caught each regressed sink, and the tree was restored byte-identical after every run. The control that makes those numbers meaningful is that the port and the compiled guard agree on the clean tree — both report zero — and that the shipped guard demonstrably passes on a regressed tree the new one fails.
