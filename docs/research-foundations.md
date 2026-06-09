# Research Foundations of CodeLore's Analyses

Behavioural code analysis isn't a folk practice — it sits on three decades of
empirical software-engineering research. This page maps every behavioural
analysis CodeLore ships to its underlying citation, what the signal *means*,
what "good" looks like in practice, and where the implementation lives.

> **Why this doc exists**: `code-maat`'s source files scatter citations across
> 13 Clojure files in free-text comments — valuable but ad-hoc, with no
> cross-referencing. CodeLore consolidates them here so contributors and
> users discover the academic provenance in one place. Each
> `analyses/*.rs` rustdoc header carries a single line linking back to its
> entry here (see PAR-9 in `docs/reports/deep_analysis_report.md`).

---

## How to read this page

Each analysis section has the same five-part shape:

1. **Citation** — primary paper / talk / book + year. Where multiple
   sources are foundational, the first is the load-bearing one.
2. **What the signal means** — one sentence framing the question the
   analysis answers, in plain English (not jargon).
3. **What "good values" look like** — practitioner heuristics. These are
   approximate; treat them as start-here numbers, not policy.
4. **Implementation** — link to `crates/codelore-lib/src/analyses/<file>.rs`.
5. **Why it matters** — when this signal is decisive vs decorative.

---

## Core behavioural signals (code-maat parity)

### `authors` — Number of distinct authors per file

**Citation**: Bird, C., Nagappan, N., Murphy, B., Devanbu, P., & Zeller, A.
(2011). "Don't Touch My Code! Examining the Effects of Ownership on
Software Quality." *FSE '11* (ACM SIGSOFT International Symposium on the
Foundations of Software Engineering), pages 4–14.
[doi:10.1145/2025113.2025119](https://doi.org/10.1145/2025113.2025119)

**What the signal means**: For each file, how many distinct people have
contributed? This is one of the most robust empirical predictors of
defect density in the software-engineering literature — files touched
by many authors are at higher risk than files with a dominant owner,
even controlling for size, complexity, and churn.

**What "good values" look like**:
- **1–2 authors**: clear ownership, low defect risk.
- **3–5 authors**: typical for hot-path infrastructure code; monitor.
- **6+ authors with no dominant contributor**: red flag — files in
  this region in the Microsoft Research replication had defect rates
  3–5× the baseline.

**Implementation**: `crates/codelore-lib/src/analyses/authors.rs`
([source](../crates/codelore-lib/src/analyses/authors.rs))

**Why it matters**: This is the signal that started behavioural code
analysis. CodeLore's identity layer (`.mailmap`-canonicalised authors,
`.codelorebots` bot detection, AI-author classification) lets us also
report `n_humans` / `n_bots` separately, which Bird et al.'s analysis
predated — modern repos with heavy Dependabot traffic would otherwise
report inflated author counts.

---

### `revisions` — Commit frequency per file

**Citation**: Nagappan, N., & Ball, T. (2005). "Use of Relative Code
Churn Measures to Predict System Defect Density." *ICSE '05*
(International Conference on Software Engineering), pages 284–292.
[doi:10.1145/1062455.1062514](https://doi.org/10.1145/1062455.1062514)

**What the signal means**: How many distinct commits has this file
appeared in? Files modified frequently are statistically more
defect-prone than stable files, independent of file size.

**What "good values" look like**:
- The metric is repo-relative — there's no absolute number. Use the
  **top 10–20 entries** as your investigation list.
- Pair with `hotspots` (`revisions × complexity`) for prioritisation.

**Implementation**: `crates/codelore-lib/src/analyses/revisions.rs`
([source](../crates/codelore-lib/src/analyses/revisions.rs))

**Why it matters**: Foundational input to hotspot analysis. Standalone
it answers "what changes most?"; combined with complexity it answers
"what's worth refactoring first?".

---

### `coupling` — Files that change together

**Citation**: Tornhill, A. (2015). *Your Code as a Crime Scene: Use
Forensic Techniques to Arrest Defects, Bottlenecks, and Bad Design in
Your Programs.* Pragmatic Bookshelf. Chapter on "Logical Coupling."

Underlying empirical work: Gall, H., Hajek, K., & Jazayeri, M. (1998).
"Detection of Logical Coupling Based on Product Release History."
*ICSM '98* (International Conference on Software Maintenance), pages
190–198.

**What the signal means**: When two files change in the same commit
together more often than chance would predict, they are *logically
coupled* — even if they share no syntactic dependency. Logical coupling
captures the parts of the architecture the compiler can't see.

**What "good values" look like**:
- **Degree (coupling %) ≥ 70%** with **shared revisions ≥ 10**: strong
  coupling, worth investigating for hidden abstraction.
- **Degree 30–70%**: occasional co-change; may be context-dependent.
- **Fisher p-value < 0.05**: pair survives statistical significance gate
  — co-change is not coincidence. CodeLore applies this filter by
  default (code-maat does not — see `--code-maat-compat`).

**Implementation**: `crates/codelore-lib/src/analyses/coupling.rs`
([source](../crates/codelore-lib/src/analyses/coupling.rs))

**Why it matters**: Logical coupling surfaces architecture decay that
static analysis misses. A pair of `.h` and `.cpp` files coupled at 100%
is expected; a pair of `auth.rs` and `billing.rs` coupled at 80% is a
red flag pointing at an abstraction debt.

---

### `soc` — Sum of Coupling (per-file coupling centrality)

**Citation**: Tornhill, A. (2018). *Software Design X-Rays: Fix
Technical Debt with Behavioral Code Analysis.* Pragmatic Bookshelf.
Chapter on "Sum of Coupling."

**What the signal means**: For each file, the total number of
co-changes summed across every commit. High SoC = "central node in the
change-coupling graph" — a file that participates in many co-changes
across many partners.

**What "good values" look like**:
- The metric is repo-relative; use the top-10 list as the investigation
  set.
- A file with high `soc` but low individual `revisions` indicates a
  passive hub — pulled in by many changes but rarely the driver.

**Implementation**: `crates/codelore-lib/src/analyses/soc.rs`
([source](../crates/codelore-lib/src/analyses/soc.rs))

**Why it matters**: Standalone `coupling` shows pairwise relationships;
`soc` shows centrality. The top-SoC file in a service-oriented
architecture is usually the inter-service contract.

---

### `code-age` — Months since last modification

**Citation**: Inspired by Dan North's talk *"Short Software Half-Life"*
(at multiple conferences, ~2014–2015) and developed into a quantitative
analysis in Tornhill (2015), *Your Code as a Crime Scene*.

**What the signal means**: For each file, how long since the last
commit touched it? The bimodal distribution is the interesting shape:
code is healthiest when it's either *very old* (commodity / stable
library) or *very young* (recently active, still in working memory).
The unhealthy region is "old enough to be forgotten, but still in
active use."

**What "good values" look like**:
- **0–3 months**: actively maintained; engineer can answer
  questions about it.
- **3–24 months**: warning region. Original author may have rotated;
  knowledge decays.
- **24+ months**: either commodity (good) or accidentally-abandoned
  (bad). Pair with `revisions` to distinguish: high revisions + high
  age = unmaintained hot path.

**Implementation**: `crates/codelore-lib/src/analyses/code_age.rs`
([source](../crates/codelore-lib/src/analyses/code_age.rs))

**Why it matters**: Half-life framing is one of the few empirical
arguments for *deleting* code rather than refactoring it.

---

### `abs-churn` / `author-churn` / `entity-churn` — Lines added/deleted

**Citation**: Nagappan, N., & Ball, T. (2005). "Use of Relative Code
Churn Measures to Predict System Defect Density." *ICSE '05*, pages
284–292. (Same paper as `revisions`.)

**What the signal means**: Raw line-level activity broken down by
date / author / file. Churn correlates with defect density even after
controlling for file size — but the *ratio* of churn to file size is
the more sensitive signal (relative churn).

**What "good values" look like**:
- Look for **bursts** (`abs-churn` trend lines) — periods of high
  combined add+delete are integration crunches.
- Look for **balance** (`author-churn`) — does a single author dominate
  recent additions? Pair with `main-dev` analysis.
- Look for **deletion ratios** in `entity-churn` — files with `deleted
  > added` over recent windows are being incrementally shrunk; usually
  a healthy refactoring signal.

**Implementation**: `crates/codelore-lib/src/analyses/churn.rs`
([source](../crates/codelore-lib/src/analyses/churn.rs))

**Why it matters**: Churn is the cheapest available signal; collecting
it is free (already in the diff). Use as a calibration baseline before
investing in the more complex signals.

---

### `communication` — Author co-edit graph (Conway's law)

**Citation**: Conway, M. E. (1968). "How Do Committees Invent?"
*Datamation*, **14**(5), April 1968, pages 28–31.

Empirical follow-up: Bird, C., Nagappan, N., Devanbu, P., Gall, H., &
Murphy, B. (2009). "Does Distributed Development Affect Software
Quality? An Empirical Case Study of Windows Vista." *Communications of
the ACM*, **52**(8), pages 85–93.

**What the signal means**: For each pair of authors, how often do they
edit the same files? This surfaces de-facto communication channels in
the codebase — Conway's law says these will (and should) mirror the
organisation chart.

**What "good values" look like**:
- High shared-work pairs are normal *within* teams.
- High shared-work pairs *across* nominal team boundaries indicate
  cross-team hand-offs that aren't visible in the org chart.
- Surprisingly *low* shared-work between people on the same team can
  indicate a knowledge-silo risk.

**Implementation**: `crates/codelore-lib/src/analyses/communication.rs`
([source](../crates/codelore-lib/src/analyses/communication.rs))

**Why it matters**: Conway's law isn't a hypothesis — five decades of
empirical evidence say organisational structure shapes architecture.
Reading the communication graph backwards lets you predict the
modularisation cost of a re-org before you make it.

---

### `ownership` (a.k.a. `fragmentation`) — Author concentration per file

**Citation**: Mockus, A., & Herbsleb, J. D. (2002). "Expertise Browser:
A Quantitative Approach to Identifying Expertise." *ICSE '02*, pages
503–512.

Fractal value computation: based on the Herfindahl–Hirschman Index
(HHI), classical industrial-organisation measure of market
concentration. See Hirschman, A. O. (1980), "The Paternity of an
Index." *American Economic Review*, **54**(5), page 761.

**What the signal means**: For each file, how concentrated is
authorship? Fractal value ∈ [0, 1) — 0 means a single author (full
ownership); approaching 1 means many authors with equal shares (full
fragmentation).

**What "good values" look like**:
- **Fractal < 0.3**: clear ownership; healthy.
- **Fractal 0.3–0.6**: mixed; depends on file role.
- **Fractal > 0.6**: heavily fragmented; correlates with defect risk
  per Bird et al. 2011.

**Implementation**: `crates/codelore-lib/src/analyses/ownership.rs`
([source](../crates/codelore-lib/src/analyses/ownership.rs))

**Why it matters**: Distinct from raw `n_authors` — a file with 10
authors where one wrote 90% of the lines is concentrated; a file with
3 authors all at 33% is fragmented. Fractal value captures the
distribution shape, not just the count.

---

### `main-dev` family — Top contributor per file

**Citation**: Foundational: Mockus & Herbsleb (2002), as for
`ownership`. Three-variant decomposition by metric:
- `main-dev`: ranking by lines added (D'Ambros, M., Lanza, M., &
  Robbes, R. (2010). "Evaluating defect prediction approaches: a
  benchmark and an extensive comparison." *Empirical Software
  Engineering*, **17**(4), pages 531–577.)
- `main-dev-by-revs`: ranking by revision count
- `main-dev-by-deletions` (a.k.a. `refactoring-main-dev`): ranking by
  lines deleted — Tornhill's heuristic for surfacing refactoring leads.

**What the signal means**: For each file, the most-contributing author
by the chosen metric, together with their ownership percentage. The
"ownership column" tells you whether the main-dev's claim is solid
(80%+) or contested (40%).

**What "good values" look like**:
- **Ownership > 80%**: ask this person about the file.
- **Ownership 40–80%**: shared knowledge; ask either of the top 2.
- **Ownership < 40%**: ownership is genuinely diffuse; consider
  documenting before knowledge fragments further.

**Implementation**: `crates/codelore-lib/src/analyses/main_dev.rs`
([source](../crates/codelore-lib/src/analyses/main_dev.rs))

**Why it matters**: Direct input to triage workflows ("who do I ask
about this?"). The deletion-ranked variant is particularly useful for
identifying *refactoring* expertise vs *authoring* expertise — they're
not always the same person.

---

### `entity-effort` / `entity-ownership` — Per-author detail rows

**Citation**: D'Ambros, M., Gall, H. C., Lanza, M., & Pinzger, M.
(2008). "Analysing Software Repositories to Understand Software
Evolution." Chapter in *Software Evolution* (Springer), pages 37–67.

**What the signal means**: Where the aggregate `main-dev` collapses to
a single row per file, these analyses emit one row per `(file,
author)` pair so downstream tooling can build sankey diagrams,
expertise heat maps, or refactoring task assignment.

**Implementation**: `crates/codelore-lib/src/analyses/entity_effort.rs`,
`crates/codelore-lib/src/analyses/entity_ownership.rs`

**Why it matters**: These are pipeline inputs more than human-facing
analyses. The aggregations downstream tooling needs differ enough that
shipping the un-aggregated form is the simplest way to support them
all without an API proliferation.

---

### `messages` — Commit-message regex matcher

**Citation**: Mockus, A., & Votta, L. G. (2000). "Identifying Reasons
for Software Changes Using Historic Databases." *ICSM '00*
(International Conference on Software Maintenance), pages 120–130.

Subsequent commit-message classification work: Hindle, A., Ernst, N.
A., Godfrey, M. W., & Mylopoulos, J. (2011). "Automated topic naming to
support cross-project analysis of software maintenance activities."
*MSR '11* (Mining Software Repositories), pages 163–172.

**What the signal means**: Match commit messages against a regex,
count one row per `(file, matching-commit)`. The classic use case:
identify "bug-fix" commits via regex and surface which files attract
the most fixes.

**What "good values" look like**:
- Pattern-dependent. `--expression-to-match "^fix\\b"` gives you
  bug-touchpoints; `--expression-to-match "^refactor"` gives you
  refactoring hotspots.

**Implementation**: `crates/codelore-lib/src/analyses/messages.rs`
([source](../crates/codelore-lib/src/analyses/messages.rs))

**Why it matters**: Commit-message mining is approximate (developers
lie / forget / abbreviate) but it's the only signal that captures
*intent* — what the change was *trying* to do, not just what it
touched.

---

### `top-committers` — Per-author commit leaderboard

**Citation**: No single foundational citation — this is a
contributor-recognition / release-notes / velocity analysis. The
closest academic anchor is the contribution-tracking work in
Mockus, A., Fielding, R. T., & Herbsleb, J. (2000). "A Case Study of
Open Source Software Development: The Apache Server." *ICSE '00*, pages
263–272.

**What the signal means**: For each author (post-mailmap), total
commits, total lines added/deleted, and first/last commit dates. The
modern context for "who has the most commits" — useful for release
notes, contributor recognition, velocity dashboards, and onboarding.

**Implementation**: `crates/codelore-lib/src/analyses/top_committers.rs`
([source](../crates/codelore-lib/src/analyses/top_committers.rs))

**Why it matters**: In code-maat this was approximated via
`author-churn` + sort. CodeLore exposes it as a first-class analysis so
the operator picks the question, not the workaround.

---

### `summary` — Repository-level counts

No single citation — a diagnostic / repo-overview analysis. Useful for
sanity-checking the ingest, comparing two repos at a glance, or
including in CI dashboards as an "is the data healthy?" panel.

**Implementation**: `crates/codelore-lib/src/analyses/summary.rs`
([source](../crates/codelore-lib/src/analyses/summary.rs))

---

## Modern additions ★ (no code-maat equivalent)

### `hotspots` ★ — Revisions × complexity

**Citation**: Tornhill, A. (2018). *Software Design X-Rays.* Chapter
on "Hotspots." Builds on:
- McCabe, T. J. (1976). "A Complexity Measure." *IEEE Transactions on
  Software Engineering*, **SE-2**(4), pages 308–320 (cyclomatic
  complexity).
- Cognitive complexity formalised by SonarSource (G. Ann Campbell,
  2018, *Cognitive Complexity: A New Way of Measuring Understandability*
  whitepaper).

**What the signal means**: The product of how *often* a file changes
(revisions) and how *complex* it is (combined cyclomatic + cognitive)
produces the strongest single prioritisation signal in CodeLore. High
hotspots are where defect risk and refactoring ROI both concentrate.

**What "good values" look like**:
- The metric is repo-relative — the **top 10 entries** are your
  refactoring backlog.
- Hotspots with **high revisions but low complexity** are well-managed
  hot paths; investigate the inverse first.

**Implementation**: `crates/codelore-lib/src/analyses/hotspots.rs`
([source](../crates/codelore-lib/src/analyses/hotspots.rs))

**Why it matters**: The signature CodeLore output. CodeLore's
SARIF-format hotspots integrate directly with GitHub code scanning,
turning the analysis into an in-PR comment without any glue code.

---

### `code-health` ★ — Composite health score

**Citation**: Composite score combining hotspots, ownership, and
churn ratios. Methodology developed in CodeLore (no single external
citation — see `crates/codelore-lib/src/analyses/code_health.rs`
docstring).

**What the signal means**: A single 0–100 score per file summarising
multiple signals. Useful when "list of hotspots" produces too many
results to triage.

**Implementation**: `crates/codelore-lib/src/analyses/code_health.rs`
([source](../crates/codelore-lib/src/analyses/code_health.rs))

---

### `clones` ★ — Type-1 / Type-2 structural clone detection

**Citation**:
- Koschke, R., Falke, R., & Frenzel, P. (2006). "Clone Detection Using
  Abstract Syntax Suffix Trees." *WCRE '06* (Working Conference on
  Reverse Engineering), pages 253–262.
- Sajnani, H., Saini, V., Svajlenko, J., Roy, C. K., & Lopes, C. V.
  (2016). "SourcererCC: Scaling Code Clone Detection to Big-Code."
  *ICSE '16*, pages 1157–1168.

**What the signal means**: AST-structural fingerprinting via
tree-sitter identifies functions that share the same shape (Type-1:
exact; Type-2: parameterised — renamed identifiers / literal swaps).

**What "good values" look like**:
- **0 clones** is unrealistic and not actually the goal — boilerplate
  clones are usually fine.
- Focus on **clones with high `combined_score`** in `clone-coupling`
  (next entry) — those are the actionable ones.

**Implementation**: `crates/codelore-lib/src/analyses/clones.rs`
([source](../crates/codelore-lib/src/analyses/clones.rs))

---

### `clone-coupling` ★ — Live clones × Fisher-significant co-change

**Citation**: Tornhill, A. (2018). *Software Design X-Rays.* Chapter
on "X-Ray Analysis." CodeScene's productisation of the underlying idea
— flag clones that *also* tend to change together as the actionable
subset.

**What the signal means**: A clone pair that changes together at
Fisher-significant rates indicates a violated DRY assumption — the
files are coupled both syntactically (clone) and behaviourally
(co-change). Refactoring removes coupled risk from two places at once.

**What "good values" look like**:
- The metric is repo-relative; treat the **top 10 by
  `combined_score`** as your DRY-violation backlog.

**Implementation**: `crates/codelore-lib/src/analyses/clone_coupling.rs`
([source](../crates/codelore-lib/src/analyses/clone_coupling.rs))

**Why it matters**: This is CodeLore's strategic differentiator vs
every other clone detector. Pure clone tools surface mountains of
boilerplate noise; clone-coupling filters to the pairs that are
actually expensive to maintain.

---

## How CodeLore extends each signal

Where the original research used 1990s/2000s data shapes, CodeLore
exploits the modern stack to surface richer signals from the same
underlying citations:

| Original signal | CodeLore extension |
|---|---|
| `n_authors` (Bird et al. 2011) | `n_humans` / `n_bots` / `n_ai_authors` via identity layers |
| `coupling` (Gall et al. 1998) | Fisher exact significance gate (`p < 0.05` default) filters spurious sweep noise |
| Author email | `.mailmap` canonicalisation + bot / AI-author classification |
| Per-file age (months) | Per-file age (months + days + last-modified date) — second-precision back-test via TIMESTAMP schema |
| CSV-only output | CSV + JSON + SARIF + Markdown + Parquet + SQLite; SARIF integrates with GitHub code scanning natively |
| Code-maat default tie-break: arbitrary | Deterministic secondary sort on canonical author name; cross-run reproducibility |

The detailed migration map between code-maat output and CodeLore
output is in `README.md`'s "Migrating from code-maat" section. The
philosophy (modernise, don't migrate) is in
[`feedback_modernize_dont_migrate`](../.devt/memory/feedback_modernize_dont_migrate.md).

---

## Further reading

A small curated list of papers and books worth reading whole:

- Tornhill, A. (2015). *Your Code as a Crime Scene.* Pragmatic
  Bookshelf. Where most of code-maat's research lineage is collected.
- Tornhill, A. (2018). *Software Design X-Rays.* Pragmatic Bookshelf.
  The follow-up; introduces hotspots and clone-coupling productisation.
- Bird, C., Nagappan, N., et al. (2011). *Don't Touch My Code!* —
  the load-bearing empirical paper behind the `authors` signal.
- Nagappan, N., & Ball, T. (2005). *Use of Relative Code Churn Measures
  to Predict System Defect Density.* — foundational churn paper.
- Conway, M. E. (1968). *How Do Committees Invent?* — original
  Conway's-law essay.
