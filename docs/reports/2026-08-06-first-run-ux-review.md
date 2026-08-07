# CodeLore — first-run UX review

**Reviewed:** 0.26.0 (Homebrew), 2026-08-06 · **Re-verified against:** 0.27.0 (Homebrew and
`target/release`), 2026-08-06
**Reviewer:** an experienced engineer meeting CodeLore for the first time with a specific goal —
*"keep track of our codebase and architecture health"* — evaluating it against a real service
(a FastAPI/Postgres task service, 41 commits, 103 files at HEAD).

**Status: 5 of 6 findings resolved in 0.27.0, 1 deferred by decision, 3 residual tasks open.**
Jump to [§10 Remaining tasks](#10-remaining-tasks) for the actionable list.

---

## 0. What this is

A **first-use experience** review, scoped to one job-to-be-done: *set up ongoing codebase +
architecture health tracking*. Not a capability review — the capability is there and it is deep.
This is the path from `brew install` to a health signal you trust and re-run.

`docs/reports/deep_analysis_report.md` remains the canonical home for F-findings; it tracks
*whether the machine is correct*. This tracks *whether a new user can get the machine pointed at
their repo*, a different failure surface. Nothing here is filed as an F-number — see §10 for how
the residuals relate to the tracked backlog.

Checked before the original findings were written, so this review does not re-raise settled
matters:

- **Roadmap §"Deliberately out of scope"** — 9 entries, each read in full. One (`cs rules-config`)
  is close enough to §6's proposal that the boundary is argued explicitly there.
- **Roadmap Tiers 1–5** — Tier 3 (adoption levers) and Tier 5 (community/docs) both carry items;
  neither carried a first-run/getting-started item. See §8.
- **`deep_analysis_report.md`** — searched for overlap with every finding. Two prior findings
  (**F249**, **F222**) proved to be *precedents* for §3 rather than duplicates, which is how that
  finding came to be argued as a complement rather than a gap.
- **Onboarding-term sweep across `docs/*.md` + `CHANGELOG.md`** — 22 hits, every one about the
  `team-composition` analysis (contributor onboarding as a *measured subject*) or unrelated
  "scaffolding"/"first run" phrasing. Zero about CodeLore's own first-use path.

One finding was **withdrawn** after validation contradicted it; §9 keeps it, because the way it
was wrong drove the most valuable change in this cycle.

## 1. Outcome

| # | Finding | Status | Verified by |
|---|---|---|---|
| F1 | Filtered-to-empty result reports success in silence | **Resolved 0.27.0** | 0-row run now warns on the piped path; footer carries the row count |
| F2 | `check` vacuous-pass gave no next step | **Resolved 0.27.0** | message now names three starter gate keys; `gate` got parity |
| F3 | No documented path for the stated goal | **Resolved 0.27.0** | new README §"Tracking health over time"; Action guide gained the trend pattern |
| F4 | `.codelore-thresholds.toml` has no derivation path | **Deferred** | no `init`; the README section documents the procedure instead — see §6 |
| F5a | `profile` promised cache size, printed a path | **Resolved 0.27.0** | prints size against both caps |
| F5b | Maintainer rationale in `--help` | **Resolved 0.27.0** | plus `T8:`, `auto- discovered`, and ten backticked product names |
| W1 | "Cache grows unboundedly" | **Withdrawn** — was wrong | caps existed; the real residue was fixed anyway — see §9 |

**Residual tasks: R1, R2, R3 — see §10.**

## 2. Method — reproducible

```console
$ codelore --version                                   # confirm binary matches repo Cargo.toml
$ cd <real service repo> && codelore analyze           # bare default
$ codelore check                                       # the "track health" reflex
$ codelore analyze --analysis health-trend             # ...the thing actually wanted
```

Plus a controlled cold-start on a throwaway repo (3 files, 3 commits, then 6) to isolate
young-repo behaviour from repo-specific noise, and source-level probes (`cache.rs`,
`facts/mod.rs`, `output/banner.rs`, `analyze.rs`) to confirm each observed behaviour is the
code's intent rather than an artifact of one machine.

Re-verification ran the same reproducers against both 0.27.0 builds. Every count in this document
was produced by a command.

> **Check the binary, not the repo.** At first re-verification the repo was 0.27.0 while
> Homebrew still shipped 0.26.0, so the PATH binary showed none of the fixes. `codelore --version`
> against `Cargo.toml` is the first step of any re-check.

---

## 3. F1 — Filtered-to-empty result reported success in silence · RESOLVED 0.27.0

**Original evidence (0.26.0).** The README's literal "your first analysis" command on a young repo:

```console
$ codelore analyze --analysis hotspots --repo . --min-revs 5 --rows 10
entity,revisions,cognitive,cognitive-health,hotspot-score,mi,mi-rank,mi-band,ai-pct,hotspot-score-anchored
```

One header row, no data. Under a TTY the framing was actively reassuring — `Status: ✓ ready`,
`✓ hotspots completed in 98ms`, exit 0, nothing on stderr. Cause: `--min-revs 5`, the documented
default *and* spelled out in the README command. `--min-revs 1` returned all three files,
confirming the mechanism. `code-health` — the analysis whose name matches the goal — was empty the
same way.

**Why it was argued as a complement, not a gap.** The project had already fixed this class four
times: **F249** (`ensure_ingest_witnessed`, CLI), **F222** (SPA zero-filter empty state),
`output/html.rs:288` (HTML empty-state text), and `defect-validation`'s artifact hint. F249's own
wording named the failure mode — *"render confident empty reports over a blind (fetch-depth:1 /
all-excluded) ingest."* It correctly did **not** fire here: `ensure_ingest_witnessed` tests
`commit_count() == 0`, and ingest had seen all 3 commits. The emptiness was introduced
*downstream* by the `min_revs` predicate. The principle was established; the filter path was never
brought under it.

**Resolution, verified 0.27.0:**

```console
$ codelore analyze --analysis hotspots --repo . --min-revs 5 --rows 10
entity,revisions,cognitive,…
WARN codelore::analyze: hotspots: 0 rows — the analysis ran and matched nothing.
Options in effect: min-revs=5, rows=10. If that is unexpected, relax the thresholds
(e.g. `--min-revs 1`) and re-run.
```

Not TTY-gated — it fires on the piped path, which is where a confident empty answer does the most
damage. `--format ndjson`, which previously emitted zero bytes, now warns too. The footer also
carries the row count (`✓ revisions completed in 264ms — 2 rows`); the plumbing that had been
deferred as "a bigger refactor" stopped being one once the dispatch arms were folded into a macro.

**Residual: R1** — the remedy the message suggests is wrong for a substantial minority of
analyses. See §10.

## 4. F2 — `check` and `check --ratchet` disagreed about an unconfigured repo · RESOLVED 0.27.0

**Original evidence (0.26.0).** One flag apart, same subcommand:

```console
$ codelore check
codelore check: no thresholds configured (…); vacuously passing.        # exit 0, nothing further

$ codelore check --ratchet
✅ ratchet initialized — tracking 0 metric(s): (none). Configure max_red_effort_pct /
max_dependency_cycles gates to ratchet effort and cycles. Commit `.codelore-ratchet.toml`
to enable regression detection.
```

The ratchet message was the standard: it named what was missing, the keys that would fix it, and
the next action. `check` — the command a user reaching for "track our health" types first — named
the problem and stopped, then exited 0. For a CI gate, that is a green build measuring nothing.
The exit-code contract itself was correct and documented; the **message** fell short of its sibling.

**Resolution, verified 0.27.0** (`main.rs:234`), with `gate` brought to parity:

> `codelore check: no thresholds configured (no .codelore-thresholds.toml at repo root); vacuously
> passing. Add a [gates] section with code_health_min / max_dependency_cycles / max_red_effort_pct
> to make it bind on regressions.`

Exit 0 retained, correctly. 0.27.0 additionally made a vacuous pass write `violations=0` to
`$GITHUB_OUTPUT`, which it had been omitting.

**Residual: R2** — one of the three recommended keys is absent from the README. See §10.

## 5. F3 — The stated goal had no documented path · RESOLVED 0.27.0

**Original evidence (0.26.0).** The capability was strong and invisible. `health-trend` returns a
per-commit series of `arch-health` / `code-health` / `combined-health` with green/yellow/red bands;
`architecture-trend` tracks propagation cost, cycle count and largest cycle. Both ran first try, no
configuration, no flags.

| Term | README then | README now | `advanced-usage.md` |
|---|---:|---:|---:|
| `health-trend` | 1 | **3** | 3 |
| `architecture-trend` | 2 | **4** | 3 |
| `ratchet` | 1 | **4** | 10 |
| `code_health_min` | 0 | **1** | 13 |
| `max_dependency_cycles` | 0 | **1** | 5 |
| `max_red_effort_pct` | 0 | **0** ← R2 | 8 |

*"Your first 5 minutes"* was four one-shot exploration commands, none about tracking anything over
time, none `check`. `docs/github-action.md` — the natural home for health-over-time in CI —
contained **zero** occurrences of `trend`, `over time`, or `track`.

**Resolution, verified 0.27.0.** New `## Tracking health over time` section at README:337, directly
after "Your first 5 minutes", walking baseline → bounds → CI → ratchet. It teaches the *derivation
procedure* rather than shipping copyable numbers, and reframes this repo's own thresholds file as
"a worked example". `docs/github-action.md` now has 9 trend/track hits.

**Residual: R3** — the preceding section still hands off elsewhere. See §10.

## 6. F4 — `.codelore-thresholds.toml` has no derivation path · DEFERRED (by decision)

**Standing evidence.** To gate anything, a user must author this file. There is no scaffold: 13
subcommands, none writes one, and `args.rs` has no init surface — unchanged in 0.27.0.
`advanced-usage.md §5` is titled *"Configuration: `.codeloreignore` + thresholds"* but documents the
**analysis knobs** (`min_revs`, `min_coupling_pct`, `fisher_significance`, …), a different set of
"thresholds" than the gate keys; a newcomer searching for the gate file lands on the wrong table.

This repo's own `.codelore-thresholds.toml` remains the best onboarding artifact in the project —
every gate carries the measurement it came from: *"Worst file today: crates/codelore-cli/src/main.rs
at ~35 … the churn terms in the composite move with every nearby commit, so a floor inside the
metric's day-to-day noise band flags routine merges rather than real decay."* That is a
**procedure**: measure the current worst, set the bound just past it, document the margin. Its
numbers cannot be copied — `code_health_min = 32.5` is calibrated to this repo.

**Original proposal.** `codelore init --thresholds`: run the same measurements at HEAD, emit the
file with each bound just past the measured worst, and write the measured value into the comment
above it — automating what the maintainer already does by hand. Not a generic scaffold; a
measurement, which is the brand.

**Out-of-scope check — argued, because the nearest entry looks close.** The roadmap excludes a
*"`cs rules-config` CLI command family clone"*; its binding reason is *"Adding a parallel `codelore
rules` family would duplicate the existing path and create a 'legacy thresholds.toml vs new
rules-config' migration trap."* This proposal adds no parallel family and no second format — it
produces the **existing** file, in the existing schema, consumed by the existing `check`.

**Disposition.** Not built in 0.27.0. F3's README section now documents the procedure explicitly,
which was the stated fallback and captures most of the value at zero code. **This entry is left
open as a judgment call for the maintainer, not as an outstanding defect** — it is not counted
among the residual tasks in §10.

## 7. F5 — Two correctness-of-surface defects · RESOLVED 0.27.0

**a. `profile` promised a number it did not print.** In 0.26.0 `args.rs:247` advertised *"cache
size, schema version, and per-analysis SQL preview"* while the output printed the cache **root**
and no size. (The doc comment itself was rewritten in 0.27.0 — it now reads *"…pinned
dependencies, and cache size against its…"* at `args.rs:248` — so the original string will not
grep.) §9 shows what that omission cost.

Now: `Cache size: 1.9 GiB / 2.0 GiB cap (5 fact stores kept per repository, oldest evicted first)`
— summed on the same basis the pruner evicts against, with both bounds as named constants beside
the pruners so the enforced and reported values cannot drift.

**b. Maintainer-internal rationale in user-facing help.** `codelore --help` opened its first
subcommand with a note about `Box`ing a clap variant. Now `analyze  Run an analysis and emit
results`. 0.27.0 swept further than reported: a bare `T8:` ticket prefix on
`--departed-threshold-days`, `--explain`'s "subsequent point releases" promise, `--strict-grouping`'s
dated design justification, `--thresholds-file`'s `auto- discovered` line-break artifact, and ten
command lines rendering literal backticks because `clippy::pedantic`'s `doc_markdown` required them
(fixed via `clippy.toml`'s `doc-valid-idents` rather than by de-linting).

## 8. Was this tracked? — no, and the reason was circular

Roadmap **Tier 5 (community/docs)** carried six items — comparison matrix, "Anatomy of a hotspot"
tutorial, case studies, ADRs, code-maat migration guide, glossary. All conceptual or positioning
documents; none a first-run path.

Roadmap **Tier 3 (operational — adoption levers)** carried four integration items, prefaced *"Lower
priority until real-world traction is measured."*

First-run experience is **upstream** of traction: the users who would generate the traction signal
were the ones meeting §3's silent empty result and §4's green-but-measuring-nothing gate. That
ordering made traction the gate on its own precondition. 0.27.0 broke the circularity; the note is
kept because the ordering rule still governs the rest of Tier 3.

## 9. Withdrawn after validation — and why it drove the most valuable change

**W1 — "The cache grows unboundedly; ship `codelore cache --prune`."**

Reproducible and real-looking: `~/Library/Caches/codelore/` held **2.0 GB across 7,989 directories,
6,643 of them (83%) empty**, no cache subcommand among the 13, and a 6-file toy repo consumed
**5.78 MB per `.duckdb`**, five files deep. For a tool whose proposition is *run me on every commit
forever*, an unbounded cache is a serious adoption objection.

It was wrong. `facts/mod.rs`:

```rust
cache::prune_repo_cache(repo_dir, 5);
cache::prune_global_cache(cache_root, 2 * 1024 * 1024 * 1024);
```

Five fact stores per repo, 2 GiB globally. The measured 2.0 GB **was the cap working as designed**,
and the 5-file directory was the per-repo eviction I was about to report as missing, observed doing
its job.

**The signal.** A reviewer *with the source open and time to grep* concluded "unbounded 2 GB leak"
and was wrong. A normal user has neither — and the one command built to answer the question,
`codelore profile`, promised cache size and printed a path. That is why F5a was reframed from a
typo fix into *print the size against its caps*, which is what 0.27.0 shipped.

**What 0.27.0 then fixed that the withdrawal had dismissed as cosmetic.** Both residues were real:

- **Emptied directories were never reclaimed** — `prune_*` deleted `.duckdb` files and nothing
  removed the directory, so the tree gained one per repository ever analysed and never lost one.
  The bytes were negligible; the cost was the walk, which the global pruner paid twice per cache
  miss. Verified after: **7,989 → 1,364 entries, 6,643 → 1 empty.**
- **"LRU" was the wrong word**, in the code's own comments and in this document's first draft. Both
  pruners order by mtime and a cache hit opens read-only, so an entry's mtime is fixed at ingest
  and never refreshed by use: a frequently-read entry can be evicted ahead of a newer unused one.
  0.27.0 corrected the docs to **oldest-ingest-first**. *(This review said "LRU" too; corrected
  here.)*

**And a bug the review did not find.** `Options.repo_path` reached the cache key *as typed*, so one
repository at one HEAD with identical flags derived a different key per invocation style — each a
full-ingest miss consuming one of that repo's five slots. Reproduced after the fix: four spellings
of one path (`.`, absolute, trailing slash, `/./`) now yield **1** cache entry where 0.26.0 produced
4.

---

## 10. Remaining tasks

Three residuals, all introduced or exposed by the 0.27.0 fixes. Each is specified with its
evidence, the change, and a check that proves it done. None is a regression from 0.26.0 behaviour;
all are edges on work that otherwise landed cleanly.

### R1 — The empty-result message recommends a remedy that cannot work for a minority of analyses

**Severity: low-medium · Effort: S · Owner decision: message design**

`analyze.rs` closes the new zero-row warning with a fixed suggestion:

```
… Options in effect: {}. If that is unexpected, relax the thresholds (e.g. `--min-revs 1`) and re-run.
```

The **label** is deliberately neutral, and the reasoning above it is explicit:

> *"`min_revs` is the usual cause but is read by only 40 of the analyses, so calling the summary
> 'filters in effect' would overstate it for the rest … Naming a cause would be a confident lie on
> a meaningful subset; naming the settings is true for all of them."*

The closing sentence names one anyway. For the ~17 analyses that never read `min_revs`, relaxing it
can never produce a row.

**Verified instances:**

- `defect-validation` prints its own correct hint and then the generic line, which contradicts it:

  ```
  defect-validation: no defect-calibration artifact configured. Run `codelore calibrate-defects …`
  WARN … defect-validation: 0 rows … Options in effect: min-revs=5. … relax the thresholds (e.g. `--min-revs 1`)
  ```

  `defect_validation.rs` contains no reference to `min_revs`. The first message is right; the
  second sends the user to a dead end.
- `stale-code` gets the same suggestion; `stale_code.rs`'s only occurrence of `min_revs` is a
  `#[tracing::instrument] fields(...)` span attribute, not a filter.

Corroborating scale (approximate — file-level, so not a clean proxy for the comment's
analysis-level figure): of 64 files in `analyses/`, **22** mention `min_revs` only as a tracing span
field and **23** never mention it at all. The affected set may be larger than 17.

**Proposed change** — either half is worth doing alone:

1. Suppress the generic warning when the analysis has already emitted its own zero-row hint
   (`defect-validation` today; the same shape will recur as more analyses gain specific hints).
2. Emit the `--min-revs 1` example only when the analysis actually reads `min_revs`; otherwise stop
   after the options summary, or say *"this analysis does not filter on `--min-revs`; see
   `codelore explain <analysis>`"*.

The cleanest source of truth for (2) is the same per-analysis knowledge that decides whether the
span field is meaningful — no new registry needed if that is already derivable.

**Done when:** `codelore analyze --analysis defect-validation` on a repo with no calibration
artifact emits exactly one actionable message, and no zero-row run suggests `--min-revs` to an
analysis that ignores it.

### R2 — `check` recommends a gate key the README never explains

**Severity: low · Effort: XS (docs) · Location: `main.rs:234` ↔ `README.md`**

The 0.27.0 vacuous-pass message names three starter keys: `code_health_min`,
`max_dependency_cycles`, `max_red_effort_pct`. README occurrences: **1, 1, and 0**.

`max_red_effort_pct` is the subtlest of the three — churn share landing in red-band files, as a
share of health-banded churn only, with health-*improving* churn exempt — and it is the one a user
cannot look up where the CLI has just sent them. It is documented at
`docs/advanced-usage.md:230` (`#### max_red_effort_pct quality gate`).

**Proposed change:** add `max_red_effort_pct` to the README §"Tracking health over time" step-2
example with a one-line comment, or drop it from the CLI message in favour of a key the README
covers. Prefer the former — it is the gate most specific to an active remediation campaign, which
is exactly the reader that section is written for.

**Done when:** every gate key named in a CLI message appears in the README, or the message names
only keys that do.

### R3 — "Your first 5 minutes" hands off past the section that answers the next question

**Severity: low · Effort: XS (docs) · Location: `README.md:333`**

The section ends:

> *"Once you've run those four, you have enough signal to triage. From here, [the advanced
> guide](docs/advanced-usage.md) covers all analyses, every flag, configuration, CI integration, and
> tool-stack rationale."*

`## Tracking health over time` begins four lines later, at README:337. A reader who follows the
pointer leaves for a 1,741-line document at the exact moment the section immediately below answers
the question the new section was written to answer.

**Proposed change:** one clause — *"To watch these numbers over time rather than inspect them once,
continue below; for the full flag and analysis reference, see [the advanced guide]."*

**Done when:** the handoff names the next section before the reference doc.

### Related, in the tracked backlog — F248 rose in priority

Not a task from this review, but its weight changed as a result of it. **F248 (Active)** records
that `health-trend`'s integration test cannot detect `arch_health` regression: the only ≥2-commit
fixture (`biomarker_repo`) is six independent Rust files with no inter-file imports, so the import
graph is empty and `arch_health` is pinned at 100 across every sample.

0.27.0 promoted `health-trend` to **step 1** of the README onboarding path. A column no test can
watch decay is now the first number a new user is told to trust. The fix is already specified in
the finding: add an import-structured fixture whose later commits introduce a cycle, mirroring
`architecture-trend`'s `trend_captures_cycle_introduction_over_time`, then assert the newest
sample's `arch_health` is below an earlier one's.

### Not covered by this review

Scoped to one journey: install → health tracking, on macOS. Untested first-run surfaces, in rough
order of likely adoption weight:

- `codelore mcp` — agent/client setup, the first tool call on a cold cache
- `--format spa` — first open of the dashboard, and what a newcomer does with it
- `codelore diff` — first PR-mode run, including the worktree lifecycle
- Linux and Windows first-run, and the container entrypoint

If adoption runs mostly through the agent surface or CI, the MCP journey is the next one worth
walking.

## 11. What not to change

**The ratchet's initialization message** (§4) — the standard the rest was measured against, and the
model `check` and `gate` were brought up to. Not a candidate for editing.

**`codelore explain`** — `explain code-health` returns citation, exact formula with every smell
weight, source file, and a pointer to the foundations chain, in one command. The strongest answer
to "why should I trust this number?", and still under-sold: the first-5-minutes path never invokes
it.

**The honest-absence convention** — `defect-validation`'s zero-rows-plus-hint, the
`ensure_ingest_witnessed` error, Wilson intervals on corpus percentiles, association-not-causation
framing on defect calibration. Every recommendation in this document was an argument to apply it in
one more place, never to relax it. R1 is the same argument once more: the convention's value comes
from being right, so a hint that misfires costs more than the silence it replaced.

**The dirty-worktree cache-write refusal** — fires on every run in a repo with uncommitted changes
and is easy to mistake for noise. It is correct: caching HEAD-time complexity under a dirty tree
would silently poison every later comparison, precisely the failure a health-tracking tool must
never have.

**Reporting the settings rather than blaming a cause** (§R1) — the *label* is right and should stay.
R1 asks only that the closing suggestion be held to the same standard the label already meets.
