# CodeLore — integration examples

Drop-in templates for the most common CodeLore deployment patterns. Copy them into your own repo, change `<owner>`/`<repo>` placeholders, and you're integrated.

## GitHub Actions

### `.github/workflows/codelore-pr.yml` — PR-mode analysis

Runs `codelore diff origin/<base>...HEAD` on every pull request. Produces three surfaces:

1. **Markdown summary in the Actions run tab** — visible to any reader, no PR write permission needed. Hotspot rank changes, new clone families, coupling absent-change-pattern findings all land here.
2. **SARIF upload to GitHub Code Scanning** — findings appear in the Security tab and annotate the PR diff inline for changed lines.
3. **Optional quality gate** (`--fail-on rank-entrant`) — exits non-zero if the PR promotes any file into the top-10 hotspot list. Start with `continue-on-error: true` (advisory mode) and remove it once you trust the signal.

### Critical configuration

- **`fetch-depth: 0`** in `actions/checkout` is mandatory. Without it `git log` truncates to one commit and hotspot scores become meaningless. This is the single most common CodeLore-in-CI failure mode.
- **Three-dot merge-base notation** (`origin/main...HEAD`) anchors to the merge-base, so the analysis scopes to PR-only commits even when `main` has moved.
- **`security-events: write` permission** is required for SARIF upload. The default-permission `contents: read` is enough only for the Markdown summary path.

### Choosing the `--fail-on` setting

| Setting | Meaning | When to use |
|---|---|---|
| `none` (default) | Advisory only — never fails the job | Pilot rollout, gathering data |
| `rank-entrant` | Fails if the PR promotes a file into the top-N hotspots | Teams ready to enforce "no new hotspots" |
| `score-increase` | Fails if any existing hotspot's score worsens ≥ threshold | Teams refactoring an existing hotspot list |
| `any` | Fails on any negative finding | Teams in "stabilise, don't grow" mode |

Start with `none` for a sprint to calibrate the noise floor against your codebase, then raise the bar.

## Future examples

- GitLab CI YAML (analogous structure; uses `codelore diff` the same way)
- Pre-commit hook (`codelore analyze --quick` against staged files)
- Docker Compose snippet (mount-and-run pattern for monorepos with multiple components)
- A scheduled GitHub Action that produces a weekly hotspot trend report

Contributions welcome — open a PR with your integration template.
