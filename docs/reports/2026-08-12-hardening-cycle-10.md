# Hardening cycle 10 — a real High, in the room no one had re-entered

**Anchor:** `e62763c` (main; v0.27.3 + the cycle-9 report merge, #254) · **Baseline:** `66da4d6` (cycle-9 anchor, v0.27.3) · **Code delta:** none — the only commit since cycle 9 is the cycle-9 report landing (#254); `git diff v0.27.3..main -- ':!docs/reports'` is empty.

Audited from `git archive main` read-only; working tree clean but for the untracked `HANDOFF.md`. Nothing compiled (toolchain pin unreachable). Because the code has not moved since v0.27.3, this cycle spent its budget where the standing brief now points — *"all consumers of every change… UI/UX page designs… all possible issues, new or pre-existing"* — on the **least-recently-audited surface in the tree: the SPA JavaScript**, untouched by deep review since cycle 5. That rotation paid: one genuine High, the first new High since the cycle-6 wave.

---

## 0. What this cycle actually is

Two prior cycles closed with "no new High, no new Medium," and the honest reading of that was not "the tree is clean" but "the audit kept re-reading the same well-guarded rooms." Cycle 9 §2 said as much: guards now cover the surfaces incidents taught, so the audit should move to what guards cannot see. This cycle acted on it — a fresh-eyes pass over the ~30 SPA JS widgets, the output emitters (CSV/Markdown/GHA), a high-risk sample of analysis SQL, the ingest edges, and identity/clones — and the payoff is **F1: a stored-XSS in the architecture dashboard** that has been latent since the architecture widgets shipped, in a room the previous nine cycles walked past.

It is also, precisely, a **recurrence of the cycle-5 class**: that cycle found one widget out of step with the others on escaping (a dead `onclick` built by concatenating row data into an attribute). This is the same defect — one widget that never adopted the house `escapeHtml` — in a different file, with a live sink instead of a dead one. A class that recurs in a new location three cycles after its first instance was fixed is the signature that it needs a *guard*, not another point fix (§4).

Separately: the cycle-9 report landed on `main` via #254, carrying the errata and — answering the question left hanging at the end of last turn — the counter-report's truncated "two recommendations" adjudicated in full as **E7–E9**. I verified all three against the tree; all three hold, and **two of them correct my own recommendations** (§3). That is recorded here rather than buried, because a report that audits others owes the same standard to itself.

---

## 1. Findings

### F1 — HIGH (new) — Stored XSS in the architecture graph and DSM-matrix tooltips

`crates/codelore-lib/src/output/spa/js/40_architecture.js` builds two ECharts `tooltip.formatter` functions that concatenate **raw repository path and module strings** into tooltip HTML with no escaping:

- Force-graph formatter (`:287-300`): `p.name` (node = module path), `p.data.source`, `p.data.target` (edge endpoints) — e.g. `'Imports: ' + p.data.source + ' &rarr; ' + p.data.target` (`:294`), and `return p.name + '<br/>role: ' + …` (`:297`).
- DSM-matrix formatter (`:915-935`): `order[r]`, `order[c]` (axis labels = module paths) — e.g. `return order[r] + ' &rarr; ' + order[c] + '<br/>' + …` (`:921`, `:935`).

An ECharts *function* formatter's return value is inserted as tooltip innerHTML verbatim — the built-in token filtering that protects `{b}`/`{c}` templates does **not** apply to function returns (which is why this code can emit `<br/>`, `<strong>`, `&harr;`). So any HTML metacharacter in a module path is live.

Three facts turn this from theory into a High:

1. **The strings are attacker-influenceable.** Nodes and axis labels are repository directory/file path segments. `<`, `>`, `"` are legal in path names on Linux and macOS and are tracked and walked by gix, so a repository can contain `src/x<img src=q onerror=…>/mod.rs`; the imports analysis derives a module name from that path and it reaches the tooltip unchanged.
2. **`40_architecture.js` is the *only* interactive widget with zero `escapeHtml` calls.** The helper is defined at `10_helpers.js:220` and used pervasively by every sibling — 15× in `30_coupling_trends.js` and `14_widgets_summary.js`, 8× in `20_hotspots.js`/`12_drawer.js`/`10_helpers.js`, down the list — and **0× here**. This is not a design choice; it is the one file that never adopted the convention.
3. **There is no CSP backstop.** `template.html` ships no `Content-Security-Policy` (`http-equiv` absent), so an injected `<img onerror>` executes; a strict CSP would have demoted this to defense-in-depth. There is none.

Delivery is the SPA's own headline use: the project publishes a self-analysis dashboard to `gh-pages` on every push (`ci.yml:383`) and markets the report as shareable. A deployment that analyzes an untrusted repository — a hosted "CodeScene-alternative" scan, or the GitHub Action running on a fork PR — and publishes or shares the resulting SPA gives a viewer's browser arbitrary JS execution in the dashboard's origin (cookie theft, defacement, pivot) on hover of the malicious node. **Precondition, stated honestly:** the attacker must control a path in the *analyzed* repository, and the dashboard must be viewed; a dashboard of your own trusted repository is latent, not live — the same "needs a malicious input" shape as the H1 Action-injection this engagement ranked High in cycle 6.

**Scope is tight and verified — it is only the tooltips.** I checked the file's other sinks: the force-graph node *label* formatter (`:333`) returns `p.name`'s basename but ECharts renders graph labels as **canvas text**, not HTML — not a sink; every load-time `innerHTML =` assignment (`:487, :505, :589, :784, :802`) interpolates only static strings, theme tokens (`getCssVar`), toggle labels, and generated element IDs — no row data. So there is no *load-time* (no-hover) XSS; the two hover tooltips are the whole finding.

**Fix (mechanical, matches the house pattern):** wrap each interpolated path/module value — `p.name`, `p.data.source`, `p.data.target`, `order[r]`, `order[c]` — in the existing `escapeHtml(...)`. Five call sites. And close the *class*, per §4.

### F2 — LOW (new) — A `|` in a git tag corrupts the release-cadence Markdown table

`crates/codelore-lib/src/output/markdown/delivery.rs:86` writes the release-cadence row unescaped: `writeln!(w, "| {} | {} | {} |", row.tag, row.date, gap)`. A git tag may legally contain `|` (`git check-ref-format` permits it), which breaks GFM table column alignment for any consumer rendering the Markdown. The fix is already imported and used **in the same file** for its other tables — `escape_md_cell(&row.canonical_author)` at `:30`, `escape_md_cell(&row.path)` at `:56` — just not for the tag column. Low: it corrupts a table, executes nothing (Markdown is not evaluated), and needs a `|`-bearing tag. Fix: `escape_md_cell(&row.tag)`.

Everything else the sweep touched came back clean or self-refuted (§4 records the probes).

---

## 2. Currency (re-verified at report time — per the cycle-9 method rule)

Checked live against the crates.io API on **2026-08-12**, not carried from a prior briefing:

- **rmcp** max-stable `3.1.2` (unchanged since the 2026-08-07 publish). Workspace requires `"3.1"` (`Cargo.toml:77`), so adoption is `cargo update -p rmcp`, lockfile-only.
- **duckdb** crate max-stable `1.10505.0` — the pin is current; no engine bump pending.
- Rust pin `1.96.0` → `1.97.1` remains the live recommendation (1.98 ~2026-08-27 by cadence); zizmor and Trusted Publishing remain the two CI-hardening options, unchanged and unadopted.

No currency claim in this section is older than this report.

---

## 3. On the cycle-9 landing and E7–E9 (the audit auditing itself)

The counter-report's cut-off "two recommendations that don't survive contact with the tree" turned out to be three, and the landed cycle-9 report (#254) adjudicated them as E7–E9. I re-verified each:

- **E7 — the schema-vs-default parity guard I listed as *open work* was already shipped.** `mcp_test.rs:260-273` iterates every tool's `limit` description and asserts it neither says `"default: all"` nor omits `"50"` — exactly the guard, landed in #244, the same commit whose problem statement I quoted while recommending it. Verified. My recommendation read a commit's prose and not its tests.
- **E8 — "finishing `outputSchema` is mechanical" was wrong.** The ten remaining tool returns are heterogeneous (bare arrays, objects, arrays carrying a trailing `{omitted,total,note}` summary, a plain-text briefing); the summary shape needs redesign to fit a schema. Per-tool design work, legitimately deferred. Confirmed by the return-type variety in `mcp.rs`.
- **E9 — the M8 prescription I carried for four cycles is incomplete.** Ingest runs under `self.blocking(...)` → `tokio::task::spawn_blocking` (`mcp.rs:420`), and a `spawn_blocking` task cannot be aborted — so "thread `RequestContext` and check the token" lets a handler stop *waiting* while the permit and the work run to completion. Verified. A real M8 fix must first decide what cancellation releases (the permit, the wait, or nothing); it is a design question, not a wiring one. **M8 is reframed accordingly below.**

All three hold; two correct me. The cycle-9 method rule they produced — *a recommendation is a claim about the tree's future and carries the same burden as a claim about its present, including that it is not already implemented* — is now something I owe as much as anyone, and F1's recommendation (§1) and the guard proposal (§4) were checked against it before writing.

---

## 4. The class behind F1, and the guard that would close it

F1 is the third appearance of one class: **row data (paths, author names) reaching an HTML sink without `escapeHtml`.** Cycle 5 fixed the first instance (the `onclick` attribute) and cycle 5's own report noted the file used `addEventListener` everywhere else — i.e. it was already an out-of-step-widget observation. Three cycles later the same class surfaces in a different widget. Point-fixing F1's five call sites leaves the next widget free to reintroduce it.

The project's signature move — demonstrated by `tag_trigger_pattern_test`, `changelog_release_section_test`, the pin-agreement test, the pseudo-path iterate-everything test — is to convert an incident class into a discriminating guard. F1's class is guardable the same way: a test (or a `build.rs` lint over the embedded JS) that asserts **no SPA widget interpolates a known-raw field into a `formatter` return or an `innerHTML` assignment without a wrapping `escapeHtml`**. The heuristic that finds F1 is cheap — flag `innerHTML =` / `formatter: function` bodies that concatenate an identifier not passing through `escapeHtml`/`Number`/a numeric `.toFixed` — and it would have caught this the day the architecture widget was written. That is the higher-leverage deliverable than the five-site fix itself.

**Probes that held (recorded, because absence-of-finding is evidence too):** CSV emitters quote cells containing `= + - @` / commas / quotes (spreadsheet-injection and delimiter-safety both covered); the GHA `::error::` writer escapes `%`/newline per the workflow-command rules; the Markdown writers escape paths and authors *except* the F2 tag; the analysis-SQL sample (change-coupling, soc, main-dev, code-age, communities, crossing, stale-code, lead-time) guards its divisions and does not turn `LEFT JOIN`s inner via right-table `WHERE`s; the force-graph node labels are canvas (not a sink); the one other unescaped SPA attribute (a marginal-owner `title`) is a hardcoded constant, not row data; ingest path-normalization and the empty/single-commit edges hold. The sweep default-REFUTED five further candidates before they reached this page.

---

## 5. Open items (the whole list, M8 reframed)

- **F1 fix + the escapeHtml guard** (§1, §4) — the one thing this cycle adds that should land.
- **F2** — one-line escape (§1).
- **M8 — MCP cancellation, reframed per E9:** not "thread `RequestContext`" but "decide what a cancelled call releases, given the work runs under non-abortable `spawn_blocking`." The honest options are a cancellation-aware permit (drop the permit when the caller cancels, let the detached ingest finish and populate the cache anyway) or accepting that cold calls are uncancellable and documenting it. A design note, not a code stub.
- **`outputSchema` for the remaining 10 tools** — per-tool design (E8), not mechanical; the `{omitted,total,note}` summary shape is the crux.
- **Gitlink differential fixture** — the last unprobed content class; one commit with a mode-`160000` entry.
- **Currency** (§2): rmcp `cargo update`, rustc `1.97.1`, zizmor, Trusted Publishing.

No item here is a regression, and none moves an exit code except F1's security surface.

---

## 6. Housekeeping

- **Branch list is `main` + `gh-pages`** (the latter actively published, per the E1 correction — not stale). All prior report/fix branches merged and deleted; cycle 9 landed via #254.
- **This cycle's artifacts** (`cl273.tar`, `delta9.patch` reused; `agent10.md` findings) sit in the audit workspace and `_to_delete/cycle9-audit-artifacts/` on the device; `rm -rf _to_delete` when convenient (the bridge cannot unlink). `HANDOFF.md` remains untracked and yours. `tmp_obj_*` strays clear with `find .git/objects -name 'tmp_obj_*' -delete`.
- **This report** is committed to branch `docs/hardening-cycle-10`, based on `main` (`e62763c`), for landing via PR per the #243/#254 convention.

---

## 7. Method

Because the code had not moved, the pass was a surface rotation rather than a delta audit: a fresh-eyes sweep of the SPA JS, output emitters, an analysis-SQL sample, ingest edges, and identity/clones, each candidate default-REFUTED and each survivor re-read first-hand against source before landing here (F1's sink verified as an ECharts function-formatter HTML insertion, its escaping absence confirmed against the house pattern, its no-CSP backstop confirmed in `template.html`, its scope bounded by checking the file's other sinks; F2 confirmed against its own in-file siblings). The E7–E9 adjudication was run against the tree, not accepted from the landed report. Currency was re-verified live at report time. Findings: one High, one Low, one guard proposal, ~15 clean probes, five refutations — a shape that says the tree is still hard to find defects in, and that the one place it wasn't is the place the audit had not looked since cycle 5.
