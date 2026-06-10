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

The findings appear in the PR's **Security** tab and the **Files changed** view, with severity derived from each row's `(100 − code_health) / 10` band.

## Inputs

| Input | Default | Description |
|---|---|---|
| `analysis` | `hotspots` | Any codelore analysis name (see `codelore analyze --help` or `docs/research-foundations.md`) |
| `format` | `sarif` | `csv \| json \| sarif \| markdown \| parquet \| sqlite \| html` |
| `output` | `codelore-result` | Output file path relative to `GITHUB_WORKSPACE`. Empty string = stdout |
| `repo` | `.` | Path to repository to analyse (defaults to the checked-out workspace) |
| `args` | (empty) | Extra CLI flags appended verbatim (`--rows`, `--min-revs`, `--departed-threshold-days`, etc.) |
| `version` | `latest` | `latest` follows the most recent v* release; `vX.Y.Z` pins to a specific version |

## Outputs

| Output | Description |
|---|---|
| `result-path` | Absolute path to the generated file (empty when streaming to stdout) |
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
    analysis: [hotspots, knowledge-islands, clone-coupling]
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
5. Runs `codelore analyze` with the requested flags.
6. Exposes `result-path` (and `version-used`) as outputs.

The action's startup cost is essentially the download (≈ 200-300 ms on warm GitHub CDN). No Docker pull, no Node bootstrap.

## Permissions

| Permission | Why |
|---|---|
| `contents: read` | Required by `actions/checkout` (default) |
| `security-events: write` | Required by `github/codeql-action/upload-sarif` |
| `actions: read` | (Optional) Lets the action read previous run metadata for caching |

## Versioning

Following SemVer. Major-version pin recommended for production:

```yaml
- uses: emrecdr/codelore@v1
```

Pin to a specific release for reproducibility:

```yaml
- uses: emrecdr/codelore@v1
  with:
    version: v0.3.0
```

## Limitations

- `fetch-depth: 0` is required on `actions/checkout` so codelore sees the full git history. Shallow clones return empty / partial analyses.
- The action downloads the binary on every run; for self-hosted runners with limited bandwidth, consider caching `$RUNNER_TEMP/codelore` between runs.
