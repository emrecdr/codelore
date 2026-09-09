# Contributing

## Getting a build

The toolchain is pinned in `rust-toolchain.toml`; rustup will install it for
you. The task runner is [`just`](https://github.com/casey/just).

```bash
just build        # cargo build --workspace
just test         # the suite CI runs, minus the browser tests
just lint         # the exact clippy invocation CI runs
just ci           # fmt-check + lint + zizmor + deny + test
```

Run `just` on its own to list every recipe.

Two things about the test suite are worth knowing before your first run. The
`test-support` feature gates the fixture builder and every fixture-writing
test module, so a bare `cargo test -p codelore-lib` compiles and runs only a
subset. And the single-page dashboard lives behind the `spa` feature, whose
build script fetches its vendored JavaScript at build time — `just test`
enables both.

`just lint` and `just ci` run the same commands as CI, deliberately. Narrower
local runs have missed lints that CI then caught.

## Before you change anything

`CLAUDE.md` at the repository root is the implementor's orientation: the three
crates, the ingest threading model, the cache key, and the areas where a
change has a non-obvious second half. The conventions the project holds itself
to live in `.devt/rules/`.

Three of those are worth repeating here, because they are the ones a first
patch runs into:

- **The `Repo` trait has two implementations, and they change together.**
  `GixRepo` is the production walker; `GitCliRepo` shells out to `git` and
  exists only as a differential oracle. `tests/differential_repo_test.rs`
  asserts they produce identical event streams, so a change to one that is not
  mirrored in the other fails the gate by design.
- **No version numbers, ticket IDs or static counts in code or docs.** Code
  comments and documentation describe the current contract. Version history
  lives in `CHANGELOG.md`, and nowhere else.
- **`unsafe` is forbidden workspace-wide.** There are no `unsafe` blocks and
  CI rejects additions.

## Making the change

Small and traceable. Every changed line should trace to the task; if a
reviewer cannot see why a line was touched, it should not have been.

If you find an unrelated latent bug while you are in there, record it in
`docs/reports/deep_analysis_report.md` as a new finding rather than fixing it
in the same commit. That keeps `git bisect` honest and keeps the review about
one thing. Orphans your own change created — a now-unused import, a helper
with no callers left — are yours to clean up.

Prefer the simplest thing that solves the problem. Three similar lines beat a
premature abstraction, and a fifty-line direct solution beats a
two-hundred-line general one.

## Tests

A fix comes with a test that fails without it. That second half is the part
worth checking: run the new test against the unfixed code and watch it fail,
because a test that passes either way documents nothing. Where a test asserts
on a count or a loop, give it a floor — several tests in this repository have
passed over zero rows.

For a change claimed to preserve behaviour, prove it against a baseline rather
than arguing it: run the affected analysis before and after and diff the
output. Two traps are worth knowing. Pick the analysis by probing that it can
see the change at all, since most analyses are blind to most changes. And pass
`--no-cache` on both runs when the change happens at ingest time, or the
second run serves the first run's facts back to you.

## Commits and pull requests

[Conventional Commits](https://www.conventionalcommits.org): `fix(cache): …`,
`feat(analyses): …`, `docs: …`. Write the body for someone who will read it in
a year with no memory of the discussion — what was wrong, why it mattered, and
what you decided not to do.

User-visible changes get a `CHANGELOG.md` entry under `[Unreleased]`, in the
prose style of the entries already there.

Releases are cut by `scripts/cut-release.sh` and nothing else. Never bump a
version by hand and never push a `v*` tag.

## Licence

The workspace is GPL-3.0-only. `crates/codelore-rca` is a vendored fork of
Mozilla's rust-code-analysis and carries its own licensing — see
`crates/codelore-rca/UPSTREAM.md` before changing anything in it, and keep the
upstream diff small.
