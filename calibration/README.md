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

The build ingests each repo at its pinned SHA, pools per-function raw metrics
(`cyclomatic`, `cognitive`, `sloc`, `nargs`, `max_nesting`) **per language by
file extension** (the advisory `languages` field is not used to filter — the
pool reflects whatever the ingest actually finds), and reduces each pool to a
quantile-breakpoint vector. The resulting artifact contains **only aggregated
numeric distributions** — no source code and no user data.

## The embedded artifact

`crates/codelore-lib/src/calibration/world.calib.json` is compiled into the
binary via `include_bytes!` and surfaced by
`codelore_lib::calibration::embedded_world()`. Its `corpus_vintage` gates
activation:

- a vintage beginning with `placeholder-` resolves to `None` — the corpus lens
  stays absent-but-wired (no calibration applied) until a maintainer runs the
  real build;
- any other vintage (e.g. `world-2026-07-13`) resolves to `Some`, activating the
  lens for every `code-health` run that does not pass an explicit
  `--calibration` file.

A per-language table is only trusted once its pooled `sample_functions` clears
the `MIN_LANG_SAMPLE` floor (currently 500); a thinner language is treated as
absent at lookup time.

## Regenerating / embedding the world artifact

Run the build from the repo root, writing straight into the embedded path:

```sh
cargo build --release -p codelore-cli

./target/release/codelore calibrate \
  --repos calibration/corpus.toml \
  --vintage world-2026-07-13 \
  --output crates/codelore-lib/src/calibration/world.calib.json
```

Then rebuild so the new bytes are compiled in, run the calibration and
code-health test suites, and commit the regenerated `world.calib.json`.

### Vintage naming

Use `world-YYYY-MM` for a full world-corpus build (the month the manifest SHAs
were pinned / the build was run), appending `-DD` when a rebuild lands within
the same month (e.g. `world-2026-07-13`) so the two vintages stay
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

The build clones each repo in full into a throwaway tempdir (auto-removed once
that repo's metrics are pooled) and ingests it into the cache root
(`--cache-dir` overrides the default XDG cache). Expect:

- **Time:** up to a few hours for the full ~100-repo manifest, dominated by
  sequential `git clone`s over the network.
- **Disk:** the per-repo cache accumulates in the cache root (order ~1–2 GB for
  the full manifest). Each repo's cache is only read during its own ingest, so a
  constrained environment can point `--cache-dir` at scratch storage and clear
  it between builds. Transient clone tempdirs are the peak; a very large repo can
  briefly need hundreds of MB before its tempdir is dropped.

A repo that fails to clone, check out, or ingest is skipped with a logged
reason; `repos_included` / `repos_attempted` in the artifact record the tally.
