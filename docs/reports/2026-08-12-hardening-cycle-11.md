# Hardening cycle 11 — the fix is right; the guard is narrower than the class

**Anchor:** `6b6cc6d` (main; v0.27.4 + #260) · **Baseline:** `e62763c` (cycle-10 anchor) · **Delta:** 7 commits (#255–#260) spanning the v0.27.4 cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `6b6cc6d`; tree clean but for the untracked `HANDOFF.md`. Nothing compiled (toolchain pin unreachable). Cycle 10 reported a stored XSS (F1) and argued the deliverable was **a guard, not five point fixes**. Both shipped. This cycle verified the fix, then spent its budget adversarially testing **the guard itself** — by re-implementing its matcher and running it against inputs it does not list. That produced the one finding.

---

## 0. What this cycle actually is

The remediation is better than what cycle 10 asked for, in three specific ways worth naming before the finding:

1. **It was validated against a running binary, not just source.** #257 reproduced the payload end-to-end — "a fixture repo with such a directory emits the payload unaltered into the dashboard's JSON block" — which is a stronger standard than this audit could apply (no toolchain here).
2. **It found a defense I had not evaluated, and correctly ruled it insufficient.** The emitter rewrites `</` → `<\/` in the JSON block. #257 identifies this as transport-level — it stops `</script>` breakout and "has no jurisdiction over what happens after `JSON.parse`", where the string re-enters concatenation intact, and a payload without `</` passes it untouched. That reasoning is right, and it is the argument for fixing at the sink. I had checked for a CSP and not for this.
3. **#260 then closed a duplication *inside the guard*** — the guard listed the ten widget files in its own table, duplicating the emitter's list; it now reads the directory, with a non-empty assertion so a broken path resolver cannot masquerade as a clean run. That is the project's signature move applied reflexively, to its own new test.

F2 (the Markdown tag column) landed in the same commit. Currency closed itself: the dependency group took **rmcp to 3.1.2** (`Cargo.lock`), matching the live registry as of today.

So: the code is correct. The finding below is about the *guard's reach*, and it is the kind of defect only an adversarial test of the guard can surface.

---

## 1. Finding

### F1 — MEDIUM (new) — The escaping guard misses snake_case compound fields and locals, which is where the most attacker-controllable string in a repository lives

`spa_escaping_test.rs` matches accessors by dotted prefix (`RAW_STRING_ACCESSORS`, `:71-82`): `.path`, `.author`, `.name`, `.source`, `.target`, `order[`, and so on. The match is a plain substring search over the statement. That anchors on `.` — so it sees the **first segment after a dot** and nothing else. A field whose name is a snake_case compound does not contain the anchored form:

```
'.author' in 'ki.main_author'  →  False        (the preceding char is '_', not '.')
```

I re-implemented the guard's matcher (statement splitting including its HTML-entity rule, `preceding`, `is_escaped`, `is_lookup_key`) and ran the tree's **real author sinks** through it in their hypothetical unescaped form. Result:

| Sink (real site) | Form | Guard verdict |
|---|---|---|
| `12_drawer.js:406` `ki.main_author` | field | **MISSED** |
| `14_widgets_summary.js:181` `r.main_author` | field | **MISSED** |
| `12_drawer.js:433` `partnerAuthor` | local | **MISSED** |
| `20_hotspots.js:772` `rowAuthor` | local | **MISSED** |
| `12_drawer.js:482` `c.author` | field | CAUGHT |

**Four of the five author sinks in the tree are invisible to the guard.** All five are correctly escaped today — I verified each — so there is **no live XSS**. The defect is that the guard built to stop this class from recurring does not watch the field class most likely to carry it.

Why author names are the sharpest case: F1-of-cycle-10 required a path containing `<`, which is legal only on Linux/macOS filesystems and requires the attacker to control a directory name. A git author name has no such constraint — `git commit --author='<img src=x onerror=…> <a@b.c>'` is one flag, works everywhere, and is preserved verbatim through the walk into `main_author`, `last_author`, `top_contributor`. It is strictly easier to weaponise than the vector that produced the original High.

Two mechanisms are in play, and they need different answers:

- **Compound fields** (`main_author`, and prospectively `src_path`, `file_a`, `file_b`, `touched_file`, `last_author`, `top_contributor` — all string fields on payload row types). This is a **matcher defect**, not the documented blind spot: the module doc admits "a field added to the payload without being added below", but `main_author` *is* in the payload today and *is* rendered today. **Fix:** anchor on a word boundary rather than `.` — match `author`/`path`/`name` as a trailing segment after `.` **or** `_` — which covers every compound in one change and keeps the list short.
- **Locals** (`partnerAuthor`, `rowAuthor`). This *is* the documented across-statement-boundaries limit, and honestly stated. A cheap partial mitigation inside the existing design: also treat identifiers whose name ends in `Author`/`Path`/`Name` (case-insensitive) as raw accessors. It would catch both current locals and costs one more entry in the list. Full coverage needs taint tracking, which is correctly out of scope.

**Verification note (negative results matter).** I also tested the two other ways the guard could be too narrow, and **both came back clean**: broadening `HTML_MARKERS` with `<td, <tr, <th, <li, <a , <img, <code, </ …` produced three new hits, all false positives (`evt.target` on a DOM event; an already-escaped `params[0].name` behind a `<b>`); and expanding the accessor list with twenty other repo-derived payload fields produced two hits, both false positives (`.metric` prefix-matching `d.metrics` in a truthiness test; `.note` inside a comment). So the marker list, while not exhaustive, hides no live defect, and the accessor gap is specifically the compound-field one above. `.src_path`'s single use (`40_architecture.js:27`) is data plumbing into `modulePath()`, not a sink.

---

## 2. Recommendation: a CSP is now the missing structural backstop

Cycle 10 noted `template.html` ships no `Content-Security-Policy`; that is still true (`grep -ci content-security-policy` → 0). It mattered then as an aggravator. It matters more now as a *structural* argument: this class has produced two live defects in six cycles (cycle 5's `onclick`, cycle 10's tooltips), the guard that watches it has the reach gap in §1, and escaping-at-the-sink is therefore the sole line of defense, enforced by a convention guard that is explicitly "not a taint tracker."

The SPA is a single self-contained file with all JS inline and no network calls at runtime, which makes it an unusually good CSP candidate: a `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:">`-shaped policy costs one line and blocks the exfiltration half of any future injection (no `connect-src`, no remote `img-src` to beacon to), even though `'unsafe-inline'` is unavoidable for the inline bundle. It does not replace escaping and should not be sold as doing so — it converts "arbitrary JS with network egress" into "arbitrary JS that cannot phone home," which is the difference between a data breach and a defacement. Worth a deliberate decision either way, recorded.

---

## 3. Verified closed / still open

**Closed this delta:** cycle-10 F1 (XSS — five sinks escaped, guard added, validated against a binary); cycle-10 F2 (Markdown tag column); the guard's own file-list duplication (#260); **rmcp currency** (lockfile now 3.1.2 = live latest).

**Still open, unchanged:** M8 MCP cancellation (0 `RequestContext`; reframed per cycle-9 E9 as a design question — the ingest is under non-abortable `spawn_blocking`); `outputSchema` at **1 of 11** tools (per-tool design work per E8); the **gitlink differential fixture** (0 references — the last unprobed content class, and now the oldest untouched item on the list); `zizmor` (0 refs) and crates.io Trusted Publishing, both unadopted; Rust pin `1.96.0` with 1.97.1 available (1.98 ~2026-08-27).

---

## 4. Honesty ledger

- **Cycle 10 under-specified its own recommendation.** I proposed a guard that flags "an identifier not passing through `escapeHtml`." What shipped matches a *curated accessor list*, which is a defensible narrowing — it avoids flagging every numeric field — but it introduced the compound-field gap in §1. A recommendation that had named the field set it must cover (paths **and author names**) would have surfaced this before the code was written. Prescriptions need their acceptance criteria stated, not just their mechanism.
- **I checked for a CSP and missed the `</` → `<\/` transport defense.** #257 found it, evaluated it, and correctly ruled it insufficient. That is a real gap in cycle 10's evidence gathering: I enumerated the sinks and the absent backstop, but not the partial defense already in the emitter.
- **This finding is not a live vulnerability, and the report says so in the first paragraph of §1.** Every author sink is escaped today. Reporting a guard gap at Medium — rather than dressing it as a High by implying exploitability — is the honest severity under this engagement's own consumer-blast-radius rule: nothing is exploitable now; a silent regression is.
- **Limits.** Nothing compiled: the guard's behaviour is established by re-implementing its matcher in Python and running it over the real widget sources, not by `cargo test`. The re-implementation mirrors the Rust byte-for-byte in the parts that matter (statement split incl. the HTML-entity rule, `preceding` walk set, `is_escaped`, `is_lookup_key`), and it reproduces the shipped guard's baseline result — zero violations on the current tree — which is the control that says the port is faithful. #255/#256 are dependency bumps reviewed at manifest/lockfile level, not by auditing upstream diffs.

---

## 5. Housekeeping

- Branches: `main` + `gh-pages` (actively published; 2 commits behind locally, which is the publishing job, not staleness — per the cycle-9 E1 correction).
- This cycle's artifacts (`cl274.tar`, `delta11.patch`) are in `_to_delete/cycle11/` on the device; `rm -rf _to_delete` when convenient (the bridge cannot unlink). `HANDOFF.md` remains untracked and yours.
- **This report** is committed to branch `docs/hardening-cycle-11`, based on `main` (`6b6cc6d`), for landing via PR per the #243/#254/#258 convention.

---

## 6. Method

Small delta, so the budget went to adversarially testing the new guard rather than re-reading the code it protects: the matcher was re-implemented from its Rust source and exercised against (a) the real widget corpus as a control, (b) a broadened marker set, (c) a twenty-field expanded accessor set, and (d) the tree's actual author sinks rewritten into their unescaped form — the last being the test that produced §1's table. Every candidate was default-REFUTED; two of the three probes refuted themselves and are recorded as negative results, because a guard proven *not* to have a hole in two dimensions is worth as much as the hole found in the third. The XSS fix and #260 were read at source; residuals and currency were swept by anchor and re-verified live today.
