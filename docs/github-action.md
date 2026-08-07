# CodeLore GitHub Action

The reusable GitHub Action `emrecdr/codelore@v1` brings codelore's behavioural code analysis to any GitHub Actions workflow. Composite action — no Docker pull, no Node bootstrap, ~3 seconds startup.

## Quick start — hotspots SARIF on every PR

```yaml
name: codelore
on:
  pull_request:
  push:
    branches: [main]

jobs:
  hotspots:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write  # required to upload SARIF
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }  # codelore needs the full history
      - uses: emrecdr/codelore@v1
        id: codelore
        with:
          analysis: hotspots
          format: sarif
          output: codelore-hotspots.sarif
      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: ${{ steps.codelore.outputs.result-path }}
          category: codelore-hotspots
```

The findings appear in the PR's **Security** tab and the **Files changed** view, with severity derived from each row's `(100 − cognitive_health) / 10` band.

## Inputs

| Input | Default | Description |
|---|---|---|
| `command` | `analyze` | Which subcommand to run: `analyze` \| `check` \| `gate` \| `diff`. `analysis`/`format`/`output` apply to `analyze` only; for `check`/`gate`/`diff` pass command-specific flags (and diff's `<base>..<head>` range) via `args` |
| `analysis` | `hotspots` | Any codelore analysis name (see `codelore analyze --help` or `docs/research-foundations.md`). Applies to `command: analyze` only |
| `format` | `sarif` | `csv \| json \| sarif \| markdown \| parquet \| sqlite \| html`. Applies to `command: analyze` only |
| `output` | `codelore-result` | Output file path relative to `GITHUB_WORKSPACE`. Empty string = stdout. Applies to `command: analyze` only |
| `repo` | `.` | Path to repository to analyse (defaults to the checked-out workspace) |
| `args` | (empty) | Extra CLI flags appended verbatim. For `analyze`: `--rows`, `--min-revs`, `--departed-threshold-days`, etc. For `check`/`gate`/`diff`: the command-specific flags (`--thresholds-file`, `--ratchet`, `--fail-on`, and diff's `<base>..<head>` range) |
| `version` | `latest` | `latest` follows the most recent v* release; `vX.Y.Z` pins to a specific version |

## Outputs

| Output | Description |
|---|---|
| `result-path` | Absolute path to the generated file (empty when streaming to stdout). Always empty for `command: check`/`gate`/`diff` — `check` and `gate` stream their report and have no output file, and `diff` writes its own `--output` path, which the Action does not report back |
| `version-used` | The actual codelore version downloaded (resolved from `latest` or pinned) |

## Common patterns

### Knowledge-loss risk report on a schedule

```yaml
name: knowledge-loss-weekly
on:
  schedule:
    - cron: '0 9 * * 1'  # Mondays 09:00 UTC
  workflow_dispatch:

jobs:
  report:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: emrecdr/codelore@v1
        id: codelore
        with:
          analysis: knowledge-islands
          format: html
          output: knowledge-islands.html
          args: '--departed-threshold-days 90 --rows 50'
      - uses: actions/upload-artifact@v4
        with:
          name: knowledge-loss-report
          path: knowledge-islands.html
```

### Live clones SARIF on PRs (T9 at-risk column auto-prioritised)

```yaml
- uses: emrecdr/codelore@v1
  id: codelore
  with:
    analysis: clone-coupling
    format: sarif
    output: clone-coupling.sarif
- uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: ${{ steps.codelore.outputs.result-path }}
    category: codelore-clone-coupling
```

Clones that intersect with knowledge-island files get a higher severity automatically (T9 `at_risk` field bumps the SARIF `security-severity`). Reviewers see the most actionable debt findings first.

### Multiple analyses in one workflow

```yaml
strategy:
  matrix:
    # SARIF is wired for hotspots, clones and clone-coupling only — an
    # analysis without a SARIF rule exits 2, so it cannot ride this matrix.
    analysis: [hotspots, clones, clone-coupling]
steps:
  - uses: actions/checkout@v4
    with: { fetch-depth: 0 }
  - uses: emrecdr/codelore@v1
    with:
      analysis: ${{ matrix.analysis }}
      format: sarif
      output: codelore-${{ matrix.analysis }}.sarif
  - uses: github/codeql-action/upload-sarif@v3
    with:
      sarif_file: codelore-${{ matrix.analysis }}.sarif
      category: codelore-${{ matrix.analysis }}
```

### Code-maat-compat mode for migrating dashboards

```yaml
- uses: emrecdr/codelore@v1
  with:
    analysis: coupling
    format: csv
    output: coupling.csv
    args: '--code-maat-compat --min-revs 5'
```

CSV columns and row formatting match code-maat's verbose-mode output exactly. Drop-in for existing code-maat-targeted dashboards.

### Health over time, published on every push to main

```yaml
name: health-trend
on:
  push:
    branches: [main]

jobs:
  trend:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: emrecdr/codelore@v1
        with:
          analysis: health-trend
          format: markdown
          output: health-trend.md
      - uses: emrecdr/codelore@v1
        with:
          analysis: architecture-trend
          format: markdown
          output: architecture-trend.md
      - uses: actions/upload-artifact@v4
        with:
          name: health-over-time
          path: '*-trend.md'
```

Both trends wire `csv | json | markdown`; neither has an HTML emitter, so `format: html` fails the step. Use `csv` if a dashboard is going to read the series rather than a person.

`health-trend` emits a per-commit series of `arch-health`, `code-health` and `combined-health` with green/yellow/red bands; `architecture-trend` tracks propagation cost, cycle count and largest cycle over the same history. Neither needs configuration. Pair them with the gate below and you have the full loop: trends show the direction, gates stop the regressions. The README's [Tracking health over time](../README.md#tracking-health-over-time) walks the whole loop end to end.

## Running quality gates in CI

Set `command:` to run codelore's quality-gate subcommands instead of `analyze`. A gate violation exits non-zero, which fails the step and therefore the workflow — the intended CI signal.

These commands take their flags through `args`. The `analysis`/`format`/`output` inputs are analyze-only and are not injected for them: `check`, `gate`, and `diff` each accept their own `--format` over a different value set (and `diff` its own `--analysis` and `--output`), and the `format` default `sarif` is one `gate` rejects. Pass the command's own flags via `args` instead. All three stream their report to stdout and set no `result-path`, including `diff` when you give it an `--output` of its own.

### Fail the build on a threshold violation (`check`)

```yaml
name: codelore-gate
on:
  pull_request:

jobs:
  gate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: emrecdr/codelore@v1
        with:
          command: check
          args: '--thresholds-file .codelore-thresholds.toml'
```

`check` reads `.codelore-thresholds.toml`, prints the gate table, and exits non-zero on any violation — failing the workflow. Add `--ratchet` to fail only on regressions against the recorded baseline, or `--format sarif` for a machine-readable report (`check` accepts `text | sarif`; `json` is a `gate` format, not a `check` one). (`check` also writes `result=pass|fail` to `$GITHUB_OUTPUT`.)

### Gate the current working tree (`gate`)

```yaml
- uses: emrecdr/codelore@v1
  with:
    command: gate
    args: '--thresholds-file .codelore-thresholds.toml'
```

`gate` accepts `--format text|json` only (not `sarif`).

### Gate a PR range (`diff`)

```yaml
- uses: emrecdr/codelore@v1
  with:
    command: diff
    # diff takes a positional <base>..<head> range plus its flags, all via args.
    args: '${{ github.event.pull_request.base.sha }}..${{ github.sha }} --fail-on any'
```

`diff` compares the two revisions and fails the step when the PR crosses the `--fail-on` boundary (`none | rank-entrant | score-increase | any`).

## Supported runners

The action works on every GitHub-hosted runner family + their self-hosted equivalents:

| `runs-on` | Target |
|---|---|
| `ubuntu-latest` / `ubuntu-24.04` / `ubuntu-22.04` | `x86_64-unknown-linux-gnu` |
| `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` |
| `macos-latest` / `macos-15` (Apple silicon) | `aarch64-apple-darwin` |
| `macos-13` / `macos-14` (Intel) | `x86_64-apple-darwin` |
| `windows-latest` / `windows-2022` | `x86_64-pc-windows-msvc` |

## How the action works

1. Resolves `version` (defaults to the latest v* release).
2. Detects the runner OS+arch and computes the target triple.
3. Downloads + extracts the matching binary archive from GitHub Releases.
4. Adds the install dir to `$GITHUB_PATH`.
5. Runs the requested `codelore` subcommand (`analyze` by default; `check`/`gate`/`diff` when `command:` is set) with the requested flags.
6. Exposes `result-path` (and `version-used`) as outputs.

The action's startup cost is essentially the download (≈ 200-300 ms on warm GitHub CDN). No Docker pull, no Node bootstrap.

## Permissions

| Permission | Why |
|---|---|
| `contents: read` | Required by `actions/checkout` (default) |
| `security-events: write` | Required by `github/codeql-action/upload-sarif` |
| `actions: read` | (Optional) Lets the action read previous run metadata for caching |

## Versioning

There are **two independent pins** here, and it is worth being deliberate
about both:

| pin | selects | ref |
|---|---|---|
| `uses: emrecdr/codelore@<ref>` | the **Action** — the logic in `action.yml` | `v1` |
| `version:` input | the **binary** the Action downloads | `vX.Y.Z` |

`v1` tracks the Action's *interface* — its inputs, outputs, and behaviour. It
is not codelore's SemVer major (codelore is still `0.x`); it moves onto each
release the way `actions/checkout@v4` does, and only becomes `v2` if the
Action's interface changes incompatibly.

Recommended for production — the Action follows fixes, the binary is
whatever that release shipped:

```yaml
- uses: emrecdr/codelore@v1
```

For full reproducibility, pin **both**. Pinning only `version:` leaves the
Action's own logic floating, so what runs around your pinned binary can still
change:

```yaml
# The Action is pinned to a commit; `v1` would still float.
- uses: emrecdr/codelore@<full-40-char-commit-sha> # v1
  with:
    # And the binary is pinned to a release tag (see
    # https://github.com/emrecdr/codelore/releases for the current list).
    version: vX.Y.Z
```

## Limitations

- `fetch-depth: 0` is required on `actions/checkout` so codelore sees the full git history. Shallow clones return empty / partial analyses.
- The action downloads the binary on every run; for self-hosted runners with limited bandwidth, consider caching `$RUNNER_TEMP/codelore` between runs.
