# CodeLore — first-run UX pass: open items

**Pass:** first-run UX review, 2026-08-06 (reviewed 0.26.0, re-verified 0.27.0, closed out against a
post-0.27.0 source build). **Scope reviewed:** install → ongoing codebase + architecture health
tracking, macOS.

**All user-facing findings are closed.** Six findings and three re-verification residuals were
resolved; the closed narrative has been trimmed from this document. What shipped is in
`CHANGELOG.md` (0.27.0 and Unreleased); the finding rows are in `deep_analysis_report.md` §9 as
**F273–F286**.

This file now carries only what is still open.

---

## 1. F276 (Active, LOW) — `evaluate_all_gates` discards measured values it already computed

**Location:** `codelore-cli/src/check.rs::evaluate_all_gates`

`eval_hotspot_gates` runs the hotspot scan unconditionally but returns only `hotspot_rows.len()`,
so the measured values behind `cognitive_max`, `hotspot_score_max` and `hotspot_anchored_max` are
computed and thrown away. Code-health rows *are* returned, so `code_health_min` and
`corpus_percentile_max` are already available.

**Why it is first-run work.** This is the prerequisite for the deferred F4 (§4 below). Widening the
return unlocks six gates cheaply. The remaining gates are a harder problem: `evaluators.rs` skips
**building the import graph at all** unless `max_dependency_cycles` or `max_propagation_cost` is
already configured — so a scaffold cannot measure what it is meant to propose.

## 2. F278 (Active, LOW) — the hygiene guard's ID vocabulary is `F`-plus-digits only

**Location:** `codelore-lib/tests/comment_hygiene_test.rs::is_task_id`

`is_task_id` matches a token of `F` followed by 1–3 digits. `T8:` and `(Task 13)` are both live in
the tree and both invisible to it — and the `T8:` instance reached published `--help` output before
it was caught by review rather than by the guard.

**Constraint that makes it non-trivial** (validated, not assumed): a naive `T<N>` rule collides with
domain vocabulary. `T1`/`T2`/`T3` are clone-type names (`clone_coupling.rs`: *"1.0 for T1+T2 exact
matches"*), and every ISO-8601 timestamp in the test fixtures contains `T00:`/`T10:`. A usable rule
has to be anchored — e.g. `T<digits>:` opening a comment or doc line — rather than a bare token
match. Widening the vocabulary without that anchor produces false positives on correct code.

## 3. F279 (Fixed — Unreleased, partial) — a ticket ID shipped in user-facing help

**Location:** `args.rs` (fixed); `analyze.rs`, `explain.rs`, `options.rs`, `clone_coupling.rs`
(remaining)

The user-facing instance is gone — `codelore analyze --help` no longer prints `T8: An author is
considered "departed"…`. The same violation remains in library doc comments and inline comments.
Not user-facing, and deliberately **gated on F278's anchored rule** so the instances are fixed and
guarded together rather than piecemeal.

## 4. F4 (Deferred, blocked on F276) — `.codelore-thresholds.toml` has no derivation path

To gate anything, a user must author this file by hand. There is no scaffold: 13 subcommands, none
writes one, and `args.rs` has no init surface.

**Why it stays open rather than closed.** The README's "Tracking health over time" section now
documents the *procedure* — measure today's worst, set the bound just past it, record the
measurement in a comment — which was the stated fallback and captures most of the value at zero
code. The remaining gap is automating it, and F276 is the technical prerequisite.

**Proposal, if picked up:** `codelore init --thresholds` runs the measurements at HEAD, emits the
file with each bound just past the measured worst, and writes the measured value into the comment
above it — automating what this repo's own `.codelore-thresholds.toml` already does by hand. Not a
generic scaffold; a measurement, which is the brand.

**Out-of-scope check — argued, because the nearest roadmap entry looks close.** The roadmap excludes
a *"`cs rules-config` CLI command family clone"*; its binding reason is *"Adding a parallel
`codelore rules` family would duplicate the existing path and create a 'legacy thresholds.toml vs
new rules-config' migration trap."* This proposal adds no parallel family and no second format — it
produces the **existing** file, in the existing schema, consumed by the existing `check`. It removes
a path rather than adding one.

## 5. Release status

R1/R2/R3 (**F284–F286**) are in source and in `CHANGELOG.md` under Unreleased, but **no shipped
binary carries them.** At close-out, Homebrew and the newest `target/release` build both predated
the change; verification required `cargo build --release -p codelore`. Cutting a release is what
puts them in users' hands.

> **Re-verification note.** Twice during this pass the binary lagged the repo — once Homebrew behind
> `Cargo.toml`, once `target/release` behind a source edit by 40 minutes. `codelore --version`
> against `Cargo.toml` is necessary but not sufficient; compare source mtimes against the binary
> before concluding a fix did or did not land.

## 6. Not covered by this review

Scoped to one journey: install → health tracking, on macOS. Untested first-run surfaces, in rough
order of likely adoption weight:

- `codelore mcp` — agent/client setup, the first tool call on a cold cache
- `--format spa` — first open of the dashboard, and what a newcomer does with it
- `codelore diff` — first PR-mode run, including the worktree lifecycle
- Linux and Windows first-run, and the container entrypoint

If adoption runs mostly through the agent surface or CI, the MCP journey is the next one worth
walking.

## 7. What not to change

Carried forward because each was a candidate for "improvement" that would have been a regression.

**The ratchet's initialization message** — it names what is missing, the keys that would fix it, and
the next action. It is the standard `check` and `gate` were brought up to.

**`codelore explain`** — citation, exact formula with every smell weight, source file, and a pointer
to the foundations chain, in one command. The strongest answer to "why should I trust this number?",
and still under-sold: the first-5-minutes path never invokes it.

**The honest-absence convention** — `defect-validation`'s zero-rows-plus-hint, the
`ensure_ingest_witnessed` error, Wilson intervals on corpus percentiles, association-not-causation
framing on defect calibration. Every finding in this pass was an argument to apply it in one more
place, never to relax it.

**Reporting the settings rather than blaming a cause** on a zero-row run — the label was right from
the start. F284 removed only the closing remedy, which was true for a minority of analyses; that was
the correct resolution, not a retreat from explaining zero rows.

**The dirty-worktree cache-write refusal** — fires on every run in a repo with uncommitted changes
and is easy to mistake for noise. It is correct: caching HEAD-time complexity under a dirty tree
would silently poison every later comparison, precisely the failure a health-tracking tool must
never have.
