# Own-Repo Defect Calibration — Design

Answers the question the code-health initiative was built on: *does the health
score actually predict where defects land in THIS repository?* — and, when the
evidence clears an honesty floor, tunes the eight smell weights to the
repository's own defect history. Everything is mined from git alone, fully
local, and delivered as an opt-in vintage-stamped artifact so scores stay
byte-reproducible.

This is the first of Phase 3's two independent tracks (the second, advisory
LLM enrichment, gets its own spec). Architecture follows the corpus-calibration
precedent throughout: mine once → versioned artifact with provenance vintage →
analyses apply it only when configured.

## Unit A — defect oracle

A dedicated fix-commit classifier, deliberately **separate from** the kamei
`fix` regex (which remains untouched — it is a JIT-SDP feature input, and its
broad `issue|error|patch` alternation is a documented SZZ precision trap).

A commit is a fix iff its message matches either:
- a conventional-commit prefix `fix:` / `fix(scope):` (case-insensitive), or
- a word-boundary defect term: `\b(bug|bugfix|fix(es|ed)?|defect|regression|hotfix)\b`,

and it is neither a merge commit nor a revert (`^Revert "` prefix). Extra
include patterns are configurable in the mining manifest for teams with
tracker-id conventions. The oracle is a pure function over the message —
table-driven unit tests pin its behavior.

## Unit B — AG-SZZ linkage engine

New `codelore-lib` module behind a small trait (the roadmap's "pluggable SZZ"
seam; this implementation is the first rung, Neural-SZZ/SmartCommit can slot
in later without churn).

For each fix commit F (from Unit A):
1. Deleted line ranges of F come from the existing `hunks` table
   (`old_start`/`old_lines` are the pre-image side).
2. `git blame -w <parent-of-F> -- <path>` (subprocess with detached stdio —
   the same child-process pattern `calibrate` uses) attributes each deleted
   line to the commit that last introduced it → candidate defect-introducing
   commits D.
3. **AG filter**: a candidate line is dropped when its content at the blamed
   revision is cosmetic — blank, or a comment-only line for the file's Tier-1
   language (line reconstructed via `Repo::read_blob_at`; comment syntax per
   language is a small static table). Fix hunks that are pure additions
   (no deleted lines) contribute no candidates — recorded in mining stats.
4. Candidates newer than F, or equal to F, are discarded (clock-skew guard).

Output: `(defect_rev, fix_rev, path)` triples plus per-fix mining stats
(files blamed, lines considered, lines dropped by the AG filter, blame
failures). Mining ingests with full history and `include_merges = true`
(`commit_parents` is required to resolve the blame parent; first-parent is
used for merge fixes).

Blame or blob-read failures for a file are skip-with-log — mining never
aborts on a single path; the artifact records the skip counts.

## Unit C — validation report

Labels: a file is *defect-implicated in window W* when at least one
defect-introducing commit touched it within W (default: full history;
`--window-days`-style narrowing available at mining time and recorded in the
artifact).

Health-at-the-time: each defect-introducing commit is matched to the nearest
health-trend sample at-or-before its date (the existing ≤12-sample scan;
granularity is documented in the report and the sample dates are recorded).
Files without complexity data at the sample are excluded and counted.

Metrics (new helpers in `stats.rs`, all with hand-computable unit tests):
- the headline **band table**: share of defect-introducing changes that landed
  in files red / yellow / green at the time;
- **AUC** of `structural_risk` (HEAD) against the file labels;
- **precision@k** for k = 10 and k = |red files|;
- sample sizes on every number.

Presentation follows the project's honesty framing: association, not
causation; explicit n; no vendor-style multipliers.

Surfaced as a new registered analysis `defect-validation` (standard registry
recipe: variant, dispatch, csv/json/markdown emitters, explain entry). The
analysis READS the artifact — it never mines; without an artifact it emits
zero rows with the honest-absence hint on stderr.

## Unit D — constrained weight tuning

Runs inside artifact building, after validation:
- **Temporal split**: defects ordered by fix date; older 60% train, newer 40%
  validate (never a random split — leakage guard).
- **Search**: coordinate/grid search over the eight `SMELL_WEIGHTS`,
  projected to sum-to-1, each weight bounded to ±50% relative deviation from
  its default — the space is small enough to search exhaustively at coarse
  steps, keeping the procedure deterministic and explainable.
- **Objective**: AUC on the training labels; acceptance requires the tuned
  weights to beat the default weights' AUC on the *validation* split by a
  margin (default +0.02).
- **Honesty floor**: tuning is skipped — defaults kept, reason recorded — when
  linked defects < 30, or implicated files < 10, or the acceptance margin is
  not met. The artifact always states which branch was taken and shows both
  AUCs.

## Unit E — artifact + application

`defects.calib.json` (serde model in `calibration`-adjacent module):
`format_version`, repo identity (canonical path hash + HEAD at mining),
`vintage` (default `defects-YYYY-MM-DD`), oracle config used, mining stats,
validation metrics, `weights` (tuned or default) with the tuning decision and
both AUCs, `generated_at`.

Built by a new subcommand `codelore calibrate-defects --repo . --output
defects.calib.json [--vintage …] [--window-days …]` (mining + validation +
tuning in one run; wall-time dominated by blame subprocess calls, acceptable
because it runs once).

Applied opt-in: `--defect-calibration <file>` on `analyze`/`check` (and the
config-file equivalent). When active, code-health substitutes the artifact's
weights for `SMELL_WEIGHTS` and provenance stamps `defect_vintage` alongside
`corpus_vintage`. **Without the flag, behavior is byte-identical to today —
contract-tested** (strip-and-compare, the corpus-lens precedent). The two
calibrations compose: corpus percentiles are additive columns; defect weights
change the composite — both journeys are recorded in provenance.

## Error handling

- No fix commits found → artifact written with empty linkage, validation
  section marked insufficient, defaults kept. Never an error.
- Artifact/schema mismatches on load → typed error naming the version.
- Applying an artifact mined from a different repo → hard error (repo
  identity check), overridable with an explicit `--allow-foreign-calibration`
  escape hatch for forks.

## Testing

- Oracle: table-driven message classification (incl. revert/merge exclusions,
  conventional prefixes, configured extras).
- SZZ engine: constructed fixture with dated commits planting a KNOWN chain —
  introduce a bug in commit A, unrelated churn B, fix in commit C deleting A's
  lines → engine must link C→A and not C→B; a cosmetic-only candidate (comment
  reformat) must be AG-filtered out. Blame-failure path covered by a fixture
  with a path removed at the parent.
- Metrics: AUC/precision@k against hand-computed values (small vectors).
- Tuning: synthetic labels where the optimum is known; honesty-floor branches
  (too-few defects; margin not met) each pinned.
- Artifact: serde roundtrip, determinism (same history → byte-identical
  artifact), foreign-repo rejection.
- Application: contract test — no artifact = today's bytes; with a
  weights-differing artifact, score/band change and provenance carries the
  vintage.
- Real-CLI: `calibrate-defects` on this repository (which has a rich fix
  history) and `defect-validation` over the artifact; plausible band-table
  and n's pasted into the implementation report.

## Out of scope (explicitly)

- Advisory LLM enrichment (separate spec, second Phase-3 track).
- Neural-SZZ / SmartCommit change untangling (pluggable seam left ready).
- Gate integration of validation metrics (revisit after real-world artifacts).
- Tuning anything beyond the eight smell weights (band cutoffs, composite
  churn/ownership weights stay fixed).
- Un-stubbing kamei `lt` (cheap follow-up once blob reconstruction exists —
  noted, not included).
