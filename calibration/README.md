# Corpus calibration

This directory holds the **world-corpus manifest** (`corpus.toml`) that the
`codelore calibrate` command ingests to produce the embedded calibration
artifact powering the corpus-relative percentile lens ("your cyclomatic
complexity sits at P74 versus a reference corpus").

## What the manifest is

`corpus.toml` lists permissive-license, active, real-world open-source projects,
stratified across the Tier-1 languages (rust, python, java, javascript,
typescript). Each `[[repos]]` entry pins a `source` (clone URL or local path), a
`sha` (the default-branch HEAD at manifest-authoring time), and the advisory
`languages` the curator expects it to contribute. The pin makes every build
reproducible against a fixed tree.

The build ingests each repo at its pinned SHA and pools its metrics two ways:
per-function raw metrics (`cyclomatic`, `cognitive`, `sloc`, `nargs`,
`max_nesting`) **per language by file extension** (the advisory `languages`
field is not used to filter — the pool reflects whatever the ingest actually
finds), each pool reduced to a quantile-breakpoint vector; and repo-level
architecture metrics (`propagation_cost`, `cycle_file_share`) from the
resolved import graph, attached as sorted raw-value vectors in the artifact's
`repo_metrics` section — **one observation per repo** with a non-empty import
graph (a repo with no resolvable Tier-1 imports contributes nothing there).
The resulting artifact contains **only aggregated numeric distributions** —
no source code and no user data.

## The embedded artifact

`crates/codelore-lib/src/calibration/world.calib.json` is compiled into the
binary via `include_bytes!` and surfaced by
`codelore_lib::calibration::embedded_world()`. Its `corpus_vintage` gates
activation:

- a vintage beginning with `placeholder-` resolves to `None` — the corpus lens
  stays absent-but-wired (no calibration applied) until a maintainer runs the
  real build;
- any other vintage (e.g. `world-2026-07-14`) resolves to `Some`, activating the
  lens for every `code-health` run that does not pass an explicit
  `--calibration` file — and, through the `repo_metrics` section, the
  corpus-percentile rows on `architecture-metrics`.

A per-language table is only trusted once its pooled `sample_functions` clears
the `MIN_LANG_SAMPLE` floor (currently 500); a thinner language is treated as
absent at lookup time.

## Regenerating / embedding the world artifact

Run the build from the repo root, writing straight into the embedded path:

```sh
cargo build --release -p codelore-cli

./target/release/codelore calibrate \
  --repos calibration/corpus.toml \
  --vintage world-2026-07-14 \
  --output crates/codelore-lib/src/calibration/world.calib.json
```

Then rebuild so the new bytes are compiled in, run the calibration and
code-health test suites, and commit the regenerated `world.calib.json`.

### Vintage naming

Use `world-YYYY-MM` for a full world-corpus build (the month the manifest SHAs
were pinned / the build was run), appending `-DD` when a rebuild lands within
the same month (e.g. `world-2026-07-14`) so the two vintages stay
distinguishable. The `placeholder-` prefix is reserved for the
not-yet-built stand-in. Organization-specific corpora built from a private
manifest should use a distinct label (e.g. `acme-2026-07`) so provenance stamps
stay unambiguous.

### Refreshing the SHA pins

Each pin is a default-branch HEAD captured at manifest-authoring time. To bump a
repo to its current HEAD:

```sh
gh api repos/OWNER/REPO/commits/HEAD --jq .sha
```

Update the `sha` in `corpus.toml`, then regenerate the artifact as above.

## Disk and time expectations

The build materializes each pinned SHA into a throwaway tempdir (auto-removed
once that repo's metrics are pooled) via a **depth-1 fetch** of exactly the
pinned SHA where the server allows fetching arbitrary SHAs
(`uploadpack.allowAnySHA1InWant` — GitHub does); when the server refuses, it
falls back to a full clone with a warning. Local-path sources use a detached
`git worktree` instead. The per-repo progress line reports which path was
taken (`shallow` / `full` / `worktree`).

The ingest is **HEAD-only**: only the pinned tree's per-function complexity
facts and HEAD-time import edges are extracted — no commit history is walked,
and the history tables in the per-repo cache (under the cache root;
`--cache-dir` overrides the default XDG cache) stay empty. That is both why
shallow checkouts are ingestible (there is no history to traverse) and why
the cache entries stay small. Expect:

- **Time:** dominated by sequential network fetches; a depth-1 fetch moves a
  single tree instead of the whole history, so most repos take seconds and
  the full-clone fallback is the slow path.
- **Disk:** the per-repo cache accumulates in the cache root, holding only
  complexity and import facts per repo. Each repo's cache is only read during its own
  ingest, so a constrained environment can point `--cache-dir` at scratch
  storage and clear it between builds. Transient checkout tempdirs are the
  peak: a shallow checkout needs roughly one working tree's worth of space,
  while a full-clone fallback of a very large repo can briefly need more
  before its tempdir is dropped.

A repo that fails to fetch, check out, or ingest is skipped with a logged
reason; `repos_included` / `repos_attempted` in the artifact record the tally.
