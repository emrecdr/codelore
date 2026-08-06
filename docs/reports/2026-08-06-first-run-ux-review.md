# CodeLore — first-run UX review (external, 2026-08-06)

**Reviewer:** an experienced engineer meeting CodeLore for the first time with a specific
goal — *"keep track of our codebase and architecture health"* — evaluating it against a real
service (a FastAPI/Postgres task service, 41 commits, 103 files at HEAD).
**Version reviewed:** 0.26.0, installed via Homebrew. The installed binary and the repo's
`Cargo.toml` are both `0.26.0`, so every finding below is against current source, not a stale
build.

## 0. What this is

A **first-use experience** review, scoped to one job-to-be-done: *set up ongoing
codebase + architecture health tracking*. It is not a capability review — the capability is
there and it is deep. This is about the path from `brew install` to a health signal you trust
and re-run.

`docs/reports/deep_analysis_report.md` is the canonical home for F-findings and is
excellent at what it does: latent defects, verified, with fix recipes. It tracks *whether the
machine is correct*. This tracks *whether a new user can get the machine pointed at their
repo*, which is a different failure surface.

Checked before writing, so this review does not re-raise settled matters:

- **Roadmap §"Deliberately out of scope"** — 9 entries, each read in full. One (`cs rules-config`)
  is close enough to a proposal here that §6 argues the boundary explicitly.
- **Roadmap Tiers 1–5** — Tier 3 (adoption levers) and Tier 5 (community/docs) both exist and
  both carry items. Neither carries a first-run/getting-started item; §8 substantiates this.
- **`deep_analysis_report.md`** — searched for overlap with every finding below. Two prior
  findings (**F249**, **F222**) turn out to be *precedents* for §3 rather than duplicates of
  it, which materially strengthens that finding and changes how it is argued.
- **Onboarding-term sweep across `docs/*.md` + `CHANGELOG.md`** — 22 hits, every one of them
  about the `team-composition` analysis (contributor onboarding as a *measured subject*) or
  about unrelated "scaffolding"/"first run" phrasings. Zero refer to CodeLore's own first-use
  path.

One finding was **withdrawn** after validation contradicted it; it is kept in §9, because the
way it was wrong is itself the most actionable thing in this document.

## 1. Headline

**CodeLore's honesty discipline is its best feature, and it stops one step short of the front
door.** The project has a stated principle — a zero-row result must explain itself, *"an honest
absence, not an error"* (`advanced-usage.md:480`) — and has implemented it **four separate
times**: the blind-ingest guard (F249), the SPA zero-filter empty state (F222), the HTML
emitter's empty-state text (`output/html.rs:288`), and the `defect-validation` hint. Every one
of those lands somewhere other than the default CLI path a new user's first command actually
takes.

The result is the one outcome the project's own values are built to prevent: **a confident,
green, empty answer.**

Second: for the specific goal of *tracking* health, every part exists — `health-trend`,
`architecture-trend`, `check`, `--ratchet`, `gate`, the GitHub Action — and **no document
assembles them into a workflow.** The user has to invent the pipeline from a 57-analysis
catalogue.

## 2. Method — reproducible

```console
$ codelore --version                       # 0.26.0, Homebrew; matches repo Cargo.toml
$ cd <real service repo> && codelore analyze          # bare default
$ codelore check                                       # the "track health" reflex
$ codelore analyze --analysis health-trend             # ...the thing actually wanted
```

Plus a controlled cold-start on a throwaway repo (3 files, 3 commits, then 6) to isolate
young-repo behaviour from repo-specific noise, and four source-level probes (`cache.rs`,
`facts/mod.rs`, `output/banner.rs`, `analyze.rs`) to confirm each observed behaviour is the
code's intent rather than an artifact of this machine.

Every count in this document was produced by a command, not an impression.

## 3. F1 — A filtered-to-empty result reports success and says nothing (effort: S · highest impact)

**Evidence.** The README's own literal "your first analysis" command, run on a young repo:

```console
$ codelore analyze --analysis hotspots --repo . --min-revs 5 --rows 10
entity,revisions,cognitive,cognitive-health,hotspot-score,mi,mi-rank,mi-band,ai-pct,hotspot-score-anchored
```

One header row. No data. Under a TTY the framing is actively reassuring — banner
`Status: ✓ ready`, footer `✓ hotspots completed in 98ms`. Exit code 0. Nothing on stderr.

The cause is `--min-revs 5` — the documented default, *and* spelled out explicitly in the
README command. No file in a young repo has five revisions. `--min-revs 1` returns all three
files immediately, confirming the mechanism. `code-health` — the analysis whose name matches
the stated goal — is empty on the same repo for the same reason.

**Why this specific finding, and not "add nicer output".** The project already decided this
class of silence is unacceptable and fixed it four times:

| Where | What it does | Surface |
|---|---|---|
| **F249** (fixed v0.25.0) | `ensure_ingest_witnessed` — hard error when HEAD is real but the walk ingested nothing | CLI |
| **F222** | inline *"No paths match '…'"* empty-state row | SPA |
| `output/html.rs:288` | *"No rows produced by this analysis. This may mean the thresholds filtered everything out…"* | HTML |
| `defect-validation` | *"no defect-calibration artifact configured. Run `codelore calibrate-defects …`"* — verified live | CLI |

F249's own wording names the failure mode exactly: *"render confident empty reports over a
blind (fetch-depth:1 / all-excluded) ingest."* This is its **complement**, not a duplicate:
`ensure_ingest_witnessed`
tests `commit_count() == 0`, so it correctly does **not** fire here — ingest saw all 3 commits
and all 3 files. The emptiness is introduced *downstream*, by the `min_revs` predicate. The
principle was established; the filter path was never brought under it.

Note too that `html.rs:288` has **already written the sentence a new user needs** — *"the
thresholds filtered everything out"*. It is on the format almost nobody's first command uses.

**Why the footer doesn't save it.** `output/banner.rs` supports a row count
(`Footer.rows: Option<usize>` → `" — 12 rows"`), but `analyze.rs:411` hardcodes `rows: None`:

> *"Row counts plumbed through every (format, analysis) match arm is a bigger refactor;
> deferred. The duration + analysis-name line carries the bulk of the post-run UX value."*

That reasoning holds for a run that produced rows. For a run that produced none, duration +
name carry *negative* value: they are the part that says everything worked.

**Proposal.** Do **not** do the deferred refactor. On the empty path only — where the count is
already known to be zero because nothing was written — emit one stderr line naming the binding
filter and its current value: `0 rows — every entity fell below --min-revs 5 (highest in this
repo: 1). Try --min-revs 1.` Same shape as the `defect-validation` hint, same principle,
same voice. `§12 Troubleshooting`'s 9 rows include `clone-coupling returns 0 rows on a small
repo` and `Hotspot scores are all 0.0`, but none covers header-only output, and no row names
`--min-revs` as a cause — so the answer is currently neither at the failure point nor in the
table a user would consult after it.

**Out-of-scope check: passes.** Not a new analysis, not a score, not a composite, not an LLM
surface. It is the existing honest-absence convention applied to one more code path.

## 4. F2 — `check` and `check --ratchet` disagree about how to treat an unconfigured repo (effort: S)

Both are one flag apart in the same subcommand. On the same repo:

```console
$ codelore check
codelore check: no thresholds configured (no `.codelore-thresholds.toml` at repo root); vacuously passing.
$ echo $?
0
```

```console
$ codelore check --ratchet
✅ ratchet initialized — tracking 0 metric(s): (none). Configure max_red_effort_pct /
max_dependency_cycles gates to ratchet effort and cycles. Commit `.codelore-ratchet.toml`
to enable regression detection.
```

The ratchet message is **the standard**: it names what is missing, names the specific keys that
would fix it, and names the next action. It is the best first-run message in the tool.

`check` — the command a user reaching for "track our health" types first — names the problem and
stops. It then exits 0. For someone wiring a CI gate, that is a green build that measures
nothing, and nothing in the output suggests otherwise. The exit-code table in
`advanced-usage.md` documents this (*"0 — All gates pass (or no thresholds configured — vacuous
pass)"*), so the contract is deliberate and correct; the **message** is what falls short of its
sibling.

**Proposal.** Give `check`'s vacuous-pass message the ratchet's ending: name two or three
starter gate keys and the fact that the file goes at the repo root. If §6 lands, point at it
instead. Keep exit 0 — the contract is right.

## 5. F3 — The stated goal has no documented path (effort: S, docs only · highest goal-relevance)

For *"track codebase and architecture health"*, CodeLore's answer is genuinely strong. On the
evaluated service, `health-trend` returns a per-commit time series with `arch-health`,
`code-health`, `combined-health` and green/yellow/red bands; `architecture-trend` returns
propagation cost, cycle count and largest-cycle over time. Both worked first try, no
configuration, no flags. This is the feature that sells the tool for this job.

Their visibility, measured:

| Term | README (672 lines) | `advanced-usage.md` (1,741 lines) |
|---|---:|---:|
| `health-trend` | **1** | 3 |
| `architecture-trend` | **2** | 3 |
| `ratchet` | **1** | 10 |
| `code_health_min` | **0** | 13 |
| `hotspot_anchored_max` | **0** | 1 |
| `max_dependency_cycles` | **0** | 5 |
| `max_propagation_cost` | **0** | 3 |
| `max_red_effort_pct` | **0** | 8 |

**Not one gate key of `.codelore-thresholds.toml` appears in the README.** The README's single
`ratchet` mention is mid-sentence inside a dense paragraph at line 208, as part of a pointer to
`§11.8`.

*"Your first 5 minutes with CodeLore"* (README:297) is four commands — `summary`, `hotspots`,
`ownership`, `clone-coupling`. All four are one-shot exploration. **None concerns tracking
anything over time**, and none is `check`. The section is well-built for "show me my worst
files"; it does not serve "help me watch this over the next year".

`docs/github-action.md` — the natural home for health-over-time in CI — contains **zero**
occurrences of `trend`, `over time`, or `track`. Its patterns are hotspots SARIF, scheduled
knowledge-loss reports, live clones, multi-analysis, code-maat compat, and the three gate modes.

**Proposal.** One README section, *"Tracking health over time"*, ~20 lines, four steps:
baseline with `health-trend` / `architecture-trend` → write `.codelore-thresholds.toml` → wire
`codelore check` in CI → add `--ratchet` so it tightens on improvement. No new code. It
connects five shipped features that currently have to be discovered independently.

## 6. F4 — `.codelore-thresholds.toml` has no derivation path, though the maintainer performs one (effort: S–M)

To gate anything, a user must author this file. There is no scaffold: 13 subcommands, none
writes one, and `args.rs` contains no init/scaffold surface. `advanced-usage.md §5` is titled
*"Configuration: `.codeloreignore` + thresholds"* but documents the **analysis knobs**
(`min_revs`, `min_coupling_pct`, `fisher_significance`, …) — a different set of "thresholds"
than the gate keys. A newcomer searching for how to configure the gate file lands there and
finds the wrong table.

Meanwhile this repo's **own** `.codelore-thresholds.toml` is the best onboarding artifact in the
project. Every gate carries the measured value it was derived from and why the margin exists —
*"Worst file today: crates/codelore-cli/src/main.rs at ~35 … the churn terms in the composite
move with every nearby commit, so a floor inside the metric's day-to-day noise band flags
routine merges rather than real decay."* That is a **procedure**: measure the current worst, set
the bound just past it, document the margin.

It is referenced once in the README (line 212) — as evidence the project gates itself. It reads
as a trophy, not a template. And it cannot be used as a template: `code_health_min = 32.5` and
`hotspot_anchored_max = 9.2` are calibrated to *this* repo's measurements; copied verbatim into
another codebase they are meaningless.

**Proposal.** `codelore init --thresholds` (or `check --init`): run the same measurements at
HEAD, emit `.codelore-thresholds.toml` with each bound placed just past the measured worst, and
**write the measured value into the comment above it** — exactly the file this repo already
maintains by hand. This is not a generic scaffold; it is a measurement, which is the brand.

**Out-of-scope check — argued, because the nearest entry looks close.** The roadmap
deliberately excludes a *"`cs rules-config` CLI command family clone"*. Its binding reason is
specific: *"Adding a parallel `codelore rules` family would duplicate the existing path and
create a 'legacy thresholds.toml vs new rules-config' migration trap."* This proposal adds no
parallel family and no second format — it produces the **existing** file, in the existing
schema, consumed by the existing `check`. It removes a path rather than adding one. The entry's
own premise (*"CodeLore already has `.codelore-thresholds.toml` + `codelore check` covering the
same surface"*) assumes the user can produce that file; this closes the assumption. If the
maintainer still reads the entry as binding, F3's documented procedure is the fallback and
delivers most of the value at zero code.

## 7. F5 — Two small correctness-of-surface defects (effort: XS each)

**a. `profile` promises a number it does not print.** `args.rs:247`: *"Print operational
telemetry — **cache size**, schema version, and per-analysis SQL preview."* The actual output
prints version, schema, analysis count, formats, pinned deps, cache **root**, SPA flag — no
size. §9 shows what that omission costs.

**b. Maintainer-internal rationale is in user-facing help.** `codelore --help`, first line of
the first subcommand:

> `analyze  Run an analysis and emit results. Boxed because `AnalyzeArgs` carries the widest
> flag surface of any subcommand — inlining it would bloat every `Command` value to its size`

A note about `Box`ing a clap variant, in the first thing every user reads. The doc comment
(`args.rs:229-230`) is doing double duty as a code comment and as help text; moving the second
sentence to a plain `//` line above it fixes it without losing the rationale.

## 8. Is this already tracked? — no, and the reason is circular

Roadmap **Tier 5 (community/docs)** carries six items — comparison matrix, "Anatomy of a
hotspot" tutorial, case studies, ADRs, code-maat migration guide, glossary. All are conceptual
or positioning documents. None is a first-run path or a setup workflow.

Roadmap **Tier 3 (operational — adoption levers)** carries four integration items (reusable
GHA, GitHub App, VS Code extension, container variants), prefaced: *"Lower priority until
real-world traction is measured."*

That ordering is worth naming. First-run experience is **upstream** of traction: the users who
would generate the traction signal are the ones meeting §3's silent empty result and §4's
green-but-measuring-nothing gate. Deferring first-run work until traction arrives makes traction
the gate on its own precondition. Every item in this review is S-or-smaller and none competes
with a Tier 1 differentiator.

## 9. Withdrawn after validation — and why it is the most useful part

**W1 — "The cache grows unboundedly; ship `codelore cache --prune`."**

The observation was real and reproducible: `~/Library/Caches/codelore/` held **2.0 GB across
7,989 directories, 6,643 of them (83%) empty**, with no cache subcommand among the 13. A
6-file toy repo consumed **5.78 MB per `.duckdb`**, five files deep. It looked like a leak, and
for a tool whose whole proposition is *run me on every commit forever*, an unbounded cache is a
serious adoption objection.

It is wrong. `facts/mod.rs:459-460`:

```rust
cache::prune_repo_cache(repo_dir, 5);
cache::prune_global_cache(cache_root, 2 * 1024 * 1024 * 1024);
```

Per-repo LRU capped at 5 entries; global LRU capped at 2 GiB. The measured 2.0 GB **is the cap,
working exactly as designed** — and the 5-file directory I measured was the per-repo cap
holding at 5, i.e. the eviction I was about to report as missing, observed doing its job.
Controlled tests confirm the rest: an ordinary clean-repo run creates no new directory, and a
dirty-worktree run writes nothing at all (it declines the cache write by design). The 6,643
empty directories are consistent with this machine's own `cargo test` runs — 67 test files
build tempdir repos, each hashing to a fresh path-keyed directory — not with end-user activity.
Two residues survive, both cosmetic: `prune_*` deletes `.duckdb` files and never the emptied
directories (`cache.rs` contains no `remove_dir`), so directory count grows without bound while
bytes stay capped.

**The signal (this is the actionable part).** A reviewer *with the source open and time to
grep* concluded "unbounded 2 GB leak" and was wrong. A normal user has neither. They will see
2 GB in their cache directory, and the one command built to answer the question —
`codelore profile`, whose help **promises cache size** (§7a) — prints a path and no number.

So F5a is not a typo fix. **Print the size, and print it against its caps** —
`Cache: 1.9 GiB / 2.0 GiB cap (LRU, 5 entries per repo)`. Five lines of code that convert the
tool's single most alarming-looking artifact into a demonstration of good hygiene, and
pre-empt the exact false alarm documented above.

## 10. Suggested sequence

| # | Item | Effort | Why this order |
|---|---|---|---|
| 1 | F1 empty-result hint | S | Highest-frequency first-run outcome; the principle and the wording already exist in-tree |
| 2 | F3 "Tracking health over time" README section | S, docs | Zero code; makes `health-trend`/`check`/`--ratchet` findable for the stated job |
| 3 | F5a `profile` prints size vs caps | XS | Pre-empts §9's false alarm; makes an existing help promise true |
| 4 | F2 `check` vacuous-pass message | S | Bring it up to its own sibling's standard |
| 5 | F4 `init --thresholds` | S–M | Largest win, most design; F3 delivers the interim procedure |
| 6 | F5b help-text leak | XS | Two lines |

None requires an ADR. None adds an analysis, a score, a composite, or an LLM surface. None
creates a parallel path — F1, F2 and F5 make existing paths honest; F3 documents shipped
features; F4 generates the existing file in the existing schema.

## 11. What not to change

**The ratchet's initialization message** (§4) — it is the standard the rest should be measured
against, not a candidate for editing.

**`codelore explain`** — `explain code-health` returns citation, exact formula with every smell
weight, source file, and a pointer to the foundations chain. It makes the auditable-formulas
promise tactile in one command. It is the strongest answer to "why should I trust this number?"
and it is under-sold: the README's first-5-minutes path never invokes it.

**The honest-absence convention itself** — `defect-validation`'s *"zero rows + a hint"*, the
`ensure_ingest_witnessed` error, the Wilson intervals on corpus percentiles, the
association-not-causation framing on defect calibration. This is the project's spine, and every
recommendation above is an argument to apply it in one more place, never to relax it.

**The dirty-worktree cache-write refusal** — it fires on every run in a repo with uncommitted
changes and is easy to mistake for noise. It is correct: caching HEAD-time complexity under a
dirty tree would silently poison every later comparison, which is precisely the failure a
health-tracking tool must never have.
