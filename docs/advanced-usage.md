# CodeLore — Advanced Usage Guide

This guide is the developer-facing reference for CodeLore. The [README](../README.md) is the 5-minute pitch; this is the 30-minute manual.

## Table of contents

1. [The 32 analyses (what they tell you)](#1-the-31-analyses-what-they-tell-you)
2. [Output formats deep-dive](#2-output-formats-deep-dive)
3. [Every CLI flag explained](#3-every-cli-flag-explained)
4. [PR-mode: `codelore diff`](#4-pr-mode-codelore-diff)
5. [Configuration: `.codeloreignore` + thresholds](#5-configuration-codeloreignore--thresholds)
6. [Identity resolution (mailmap, bot filtering, AI authorship)](#6-identity-resolution-mailmap-bot-filtering-ai-authorship)
7. [Kamei change-feature vector](#7-kamei-change-feature-vector)
8. [Persistent cache mechanics](#8-persistent-cache-mechanics)
9. [Tool stack: why these choices](#9-tool-stack-why-these-choices)
10. [Performance characteristics](#10-performance-characteristics)
11. [CI/CD integration patterns](#11-cicd-integration-patterns)
12. [Troubleshooting](#12-troubleshooting)
13. [Workspace layout](#13-workspace-layout)

---

## 1. The 32 analyses (what they tell you)

The table below is split into the **17 code-maat-parity analyses** (drop-in successors to legacy code-maat), **1 modern signal** (`top-committers` — a first-class per-author leaderboard that code-maat approximated via `-a author-churn` + sort), **4 modern foundations** marked ★ (the SARIF-backed differentiators), **3 graph-analytics analyses** marked ★ (knowledge-islands + centrality + communities), and **6 architecture-analytics analyses** marked ★★ (god-classes + architecture-violations + stale-code + pair-programming + lead-time + bus-factor — see `docs/maximum-feature-plan.md`).

### Code-maat parity (17) + modern signal

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `revisions` | "Which files change most often?" | `COUNT(DISTINCT rev)` per file | First-look for any unfamiliar repo |
| `summary` | "Give me the one-page snapshot" | Commits + changes + entities + authors counts | First slide of any review |
| `authors` | "Which files are touched by many authors (defect-risk indicator)?" | Per-entity distinct author count, broken out by humans / bots / AI | Bird et al. 2011 risk signal — pair with `hotspots` for triage |
| `top-committers` | "Who are the biggest contributors repo-wide?" | Per-author totals: commits, LoC added/deleted, first/last commit, bot flag | Release notes; onboarding; contributor recognition |
| `code-age` | "Which files are stale vs. recently churned?" | Months since last commit per file | Find dead code + recently-volatile areas |
| `abs-churn` | "How fast does the team add/delete code?" | Lines added/deleted/commits grouped by date | Trend dashboards |
| `author-churn` | "Who contributes how much?" | Same as `abs-churn` grouped by canonical author (post-mailmap) | Effort distribution |
| `entity-churn` | "Which files churn the most?" | Same grouped by file | Pair with `hotspots` |
| `entity-effort` | "How much effort has each entity received per author?" | Per-(entity, author) revision counts | Pair with `code-ownership` for bus-factor narratives |
| `entity-ownership` | "Who has added/deleted what in each entity?" | Per-(entity, author) `added` + `deleted` lines | Fine-grained ownership beyond fractal value |
| `communication` | "Who works on the same code as whom?" (Conway's Law) | Author pairs by shared-work intensity | Team topology insight |
| `ownership` (aliases: `code-ownership`, `fragmentation`) | "Is each file mainly owned by one person, or fragmented?" | Fractal Value = 1 − Herfindahl-Hirschman Index + main-developer | Bus-factor; knowledge-loss risk. `fragmentation` is code-maat's name for the fractal-value-only subset |
| `main-dev` | "Who is the main developer of each entity by lines added?" | Per-entity author with max `added` (default metric) | Knowledge-owner discovery |
| `main-dev-by-revs` | "Who is the main developer by revision count?" | Per-entity author with max revision count | Use when added-lines is misleading (e.g., reformatters) |
| `main-dev-by-deletions` (alias: `refactoring-main-dev`) | "Who is the main refactorer of each entity?" | Per-entity author with max `deleted` lines | Spot quiet refactor leaders |
| `change-coupling` | "Which files always change together?" | Fisher exact-filtered logical (temporal) coupling at `p < 0.05` | Hidden architectural debt |
| `soc` | "Sum of Coupling — how central is each file in the change-coupling graph?" | Σ(N−1) over each commit of size N the file appears in | Find systemic-coupling hubs (high `SoC`) |
| `messages` | "Which entities co-occur with commits matching this message regex?" | Server-side `regexp_matches(message, --expression-to-match)` join with `changes` | Bug-fix density, label-driven hotspots |

### Modern additions (4 ★)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `hotspots` ★ | "Which files are both complex AND change a lot?" | `percentile_rank(revs) × percentile_rank(cognitive) × (100 − code_health) / 10` ([see design spec](superpowers/specs/2026-06-06-codelore-design.md)) | The headline ranking signal — refactor priorities |
| `code-health` ★ | "How healthy is each file's structure?" | 4-input composite: cognitive 0.40 + churn 0.25 + fragmentation 0.15 + coupling 0.20 (Fisher-filtered centrality) | Combined with `hotspots`; tracks degradation |
| `clones` ★ | "Where is code copy-pasted?" | Type 1 + Type 2 via AST structural hashing on tree-sitter | Refactoring candidates |
| `clone-coupling` ★ | "Which copy-pasted blocks ALSO change together?" (the strategic differentiator) | Clones JOIN coupling, Fisher-significant only | Live debt that hurts you on every change |

### Graph-analytics tier (3 ★)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `knowledge-islands` ★ | "Which files are at risk because their primary author is gone?" | Bird et al. 2011 + departed-author detection | Bus-factor risk surfaced automatically (no manual ex-developer marking) |
| `centrality` ★ | "Which files are most central in the coupling graph?" | Degree / weighted-degree / PageRank on the Fisher-significant coupling graph | Network-centrality lens (Newman 2010 §7) |
| `communities` ★ | "What are the actual Conway's-law clusters?" | Leiden algorithm (Traag, Waltman, van Eck 2019) on the coupling graph | Auto-detect socio-technical modules |

### Architecture-analytics tier (6 ★★)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `god-classes` ★★ | "Which files are gnarly AND coupled AND depended-upon?" | `(cognitive / 100) × (fan_in + fan_out)` (Brown et al. 1998 AntiPatterns §3.1) | Pick refactor targets that hit every dimension |
| `architecture-violations` ★★ | "Are layer boundaries respected?" | Imports crossing forbidden boundaries per `.codelore-arch-rules.toml` | CI gate for layered architecture |
| `stale-code` ★★ | "Which trivial files are likely abandoned?" | Alive at HEAD AND untouched ≥12 months AND `max(cognitive) ≤ 5` | Delete-candidate surfacing (intersection minimises false positives) |
| `pair-programming` ★★ | "Who pair-programs with whom?" | `Co-Authored-By:` trailer aggregation per author pair | Team-topology / mentoring signal |
| `lead-time` ★★ | "How long does code sit before shipping?" | Per-commit author-date → committer-date delta (DORA Accelerate) | Cycle-time monitoring |
| `bus-factor` ★★ | "What's our per-module bus factor?" | Filatov 2010 — minimum N authors covering ≥80% of a module's commits | Module-level Key Personnel (CodeScene shows file-level; this is the actionable view) |

All analyses are pure SQL views over the DuckDB fact store + thin Rust orchestrators. You can run any analysis at any output format.

### CLI subcommands beyond `analyze` + `diff`

```bash
codelore explain <metric>           # formula + citation + SQL source for any metric
codelore check                      # quality-gate validation against .codelore-thresholds.toml
codelore check --diff base..head    # PR-mode quality gate
codelore profile                    # operational telemetry
codelore docs                       # markdown analysis catalogue
codelore notes <base>..<head>       # release-notes markdown summary
codelore completions <shell>        # bash | zsh | fish | powershell | elvish
codelore schema <row-type>          # JSON Schema 2020-12 emit
```

`codelore check` writes `result=pass|fail` + `violations=N` to `$GITHUB_OUTPUT` when the env var is set — direct GitHub Actions step-output integration.

## 2. Output formats deep-dive

```bash
codelore analyze --analysis <NAME> --format <FORMAT>
```

| Format | Use case | Notes |
|---|---|---|
| `csv` (default) | Code-maat compatibility; pipe into other tools | Headers match code-maat exactly |
| `json` | Programmatic consumption | Pretty-printed; serde-derived |
| `markdown` | `$GITHUB_STEP_SUMMARY` in CI | GFM tables; one analysis per `# CodeLore <name>` header |
| `sarif` | GitHub Code Scanning / GitLab security / Defectdojo | SARIF 2.1.0; supported for `hotspots`, `clones`, `clone-coupling`, and `codelore diff` (CODELORE-MISSING-COCHANGE) today |
| `parquet` | DuckDB / Polars / pandas / Spark | `--output PATH` required; binary format |
| `sqlite` | Ad-hoc SQL exploration of the full fact store | `--output PATH` required; dumps all 9 tables (`commits`, `changes`, `hunks`, `entities`, `complexity_metrics`, `clones`, `imports`, `author_aliases`, `provenance`). Strict superset of code-maat's `--analysis identity` raw-dataset dump. |
| `spa` | Single-HTML interactive dashboard (CodeScene-equivalent surface). Opens in any browser, runs offline, fits in a CI artefact. | `--output PATH` required; ~1.2 MB self-contained HTML. Embeds Apache ECharts + d3-hierarchy SHA-pinned at build time. Composite (multi-analysis) emitter — bypasses `--analysis`. **Opt-in `spa` Cargo feature**: default `cargo install codelore` builds offline-clean without this. Released binaries / Homebrew / ghcr ship with `spa` enabled. |
| `step-summary` | GitHub Actions `$GITHUB_STEP_SUMMARY`. Single GFM Markdown summary with KPI table, top-10 hotspots (MI band emoji), MI band breakdown (unicode bars), behavioral coupling density, knowledge islands `<details>` collapsible. | Streams to stdout by default; redirect with `>> $GITHUB_STEP_SUMMARY` in CI. Typical 2–10 KB; well under GitHub's 1 MB step-summary cap. Same composite-dashboard inputs as `--format spa` so a single ingest run can emit BOTH (run `--format step-summary` first to stdout, then `--format spa` to file). Requires the same `spa` Cargo feature as `--format spa`. |

Every file output (except SQLite, where the provenance table lives inside the DB, and SPA, where it's embedded as a JSON block in the page) emits a `{output}.provenance.json` sidecar with the bca/gix/duckdb versions, every threshold knob, mailmap state, and UTC timestamp. This is your reproducibility receipt.

### `--format spa` widget surface

The dashboard composes eight widgets in one HTML file, plus a click-target file detail drawer:

1. **KPI tiles** — at-a-glance summary: files analyzed, commits, distinct authors, median code health, cognitive p95, knowledge-island count, coupling pair count, coupling-graph density. Each tile has a `?` provenance tooltip linking to the formula in `docs/research-foundations.md`.
2. **Knowledge islands** (CodeLore differentiator) — ranked table of departed-primary-author files with no substantial other owner. Auto-detected from commit history + co-change intensity. CodeScene paywalls this and requires manual ex-developer marking.
3. **Hotspot circle-pack map** — the signature CodeScene view. Files sized by churn, nested by filesystem hierarchy, `d3.pack()` layout fed into an ECharts `custom` series. Four toggleable colour modes: **Complexity** (cognitive heatmap), **Knowledge map** (per-author palette), **AI attribution** (per-file AI-authorship %), **Clones** (per-file clone-group count overlay).
4. **Hotspot table** — sortable, filterable drill-down. Cross-widget filter state (Alpine `$store('filter')` with `$persist`) survives reload. Click row → file detail drawer.
5. **Change-coupling sankey** — top-N file-pair coupling flows by `combined_score`. Node click → drawer.
6. **Monthly trends** — multi-line chart of top-10 hotspot paths' revision counts over time. Theme-aware (re-renders on theme toggle).
7. **Calendar heatmap** — per-day commit volume, GitHub-style 52-week strip.
8. **X-Ray function sunburst** — function-level cognitive complexity drill-down, leaf colour mapped to cognitive complexity (yellow → red ramp via the same `heatmapColor` helper the circle-pack uses).

Stack: Tailwind v4 (utility-first layout) + DaisyUI 5 (themed components; OS `prefers-color-scheme` honoured on first paint via the plugin's `--prefersdark` config) + Alpine.js 3.15 (HTML-attribute reactivity for stores + drawer + filter) + Apache ECharts + d3-hierarchy. All four vendored at build time, SHA-pinned in `build.rs`; bundle stays fully self-contained (~1.5 MB rendered SPA, no CDN at runtime).

The emitter runs every analysis each widget needs (`hotspots`, `summary`, `code_health`, `coupling`, `knowledge_islands`, `entity_ownership`, `xray`, `daily_commits`, `trends`, plus a clone-summary helper) so a single `codelore analyze --format spa` invocation produces a fully populated dashboard. Coupling and knowledge-islands degrade gracefully on tiny fixtures where Fisher significance can't be reached.

### `--format step-summary` GitHub Actions integration

The step-summary emitter writes a GFM Markdown report sized for GitHub's `$GITHUB_STEP_SUMMARY` cap (1 MB; oversize summaries are silently dropped). Typical output is 2–10 KB. It renders into the Actions UI as a native Markdown block with KPI tables, emoji band labels, and `<details>` collapsibles — no JavaScript required (GitHub sanitizes `<script>` tags from step summaries).

Copy-pasteable workflow snippet:

```yaml
name: codelore-pr-summary
on: [pull_request]

jobs:
  analyze:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # full history for behavioural analyses

      - name: Install codelore
        run: |
          curl -fsSL https://github.com/emrecdr/codelore/releases/latest/download/codelore-x86_64-unknown-linux-gnu.tar.gz \
            | tar -xz -C /usr/local/bin codelore

      - name: Run analysis and write step summary
        run: |
          codelore analyze \
            --analysis hotspots \
            --repo . \
            --format step-summary \
            >> "$GITHUB_STEP_SUMMARY"

      # Optional: also upload the full HTML dashboard as an artefact
      - name: Build full dashboard (optional)
        run: |
          codelore analyze \
            --analysis hotspots \
            --repo . \
            --format spa \
            --output codelore-dashboard.html
      - uses: actions/upload-artifact@v4
        with:
          name: codelore-dashboard
          path: codelore-dashboard.html
```

The step summary appears at the bottom of the workflow run page in the GitHub Actions UI. The HTML artefact is a separate downloadable file with the full interactive dashboard.

### SARIF rules CodeLore ships

| Rule ID | Tags | When it fires |
|---|---|---|
| `CODELORE-HOTSPOT` | `behavioral`, `hotspot` | One result per hotspot row; `security-severity = (100 − code_health) / 10`; `level` derived from severity band (≥7 = error, ≥4 = warning, else note) |
| `CODELORE-CLONE` | `behavioral`, `clone`, `type-1`, `type-2` | One result per clone family; `security-severity = 3 + family_size`, capped at 6 |
| `CODELORE-LIVE-CLONE` | `behavioral`, `clone`, `live-clone`, `co-change`, `x-ray` | One result per `(clone_group_id, file_a, file_b)`; `security-severity = combined_score × 10` |
| `CODELORE-MISSING-COCHANGE` | `behavioral`, `coupling`, `diff` | One result per absence: a historically-Fisher-significant coupling pair where this PR touched only one side. Surfaces missing partner-file edits |

All four use versioned `partialFingerprints` so cross-run identity stays stable.

## 3. Every CLI flag explained

### `codelore analyze`

```
codelore analyze [OPTIONS]
  -a, --analysis NAME           Which analysis [default: revisions]
                                (any of the 22 above; passing an unknown
                                name prints the full valid list)
  -r, --repo PATH               Git repo path [default: .]
  -f, --format FORMAT           Output format [default: csv]
                                csv | json | sarif | markdown | parquet | sqlite | spa
  -o, --output PATH             Write to file instead of stdout
      --min-revs N              Min revisions per entity [default: 5]
      --rows N                  Cap output to N rows
      --complexity-sample STRATEGY
                                head (default) | adaptive | full
                                (only `head` is wired up today; the other two parse but warn)

  # ── Coupling-family thresholds (PAR-6) ────────────────────────────
      --min-shared-revs N       Per-pair shared-commit floor [default: 2]
      --min-coupling N          Min coupling degree percentage [default: 30]
      --max-coupling N          Max coupling degree percentage [default: 100]
      --max-changeset-size N    Drop commits touching more than N files
                                (refactor-sweep filter) [default: 30]

  # ── SoC threshold (PAR-1 + PAR-9) ─────────────────────────────────
      --min-soc N               Minimum Sum-of-Coupling per entity for `soc`
                                analysis [default: 1]. Under
                                --code-maat-compat, --min-revs falls back to
                                the legacy "minimum SoC sum" semantic.

  # ── Messages analysis (PAR-2) ─────────────────────────────────────
  -e, --expression-to-match REGEX
                                Required for `--analysis messages`.
                                Server-side `regexp_matches(message, REGEX)`
                                (RE2 flavor) joined with `changes`.

  # ── Time-bucket coupling (PAR-8) ──────────────────────────────────
      --time-bucket UNIT        day | week | month
                                Modern replacement for code-maat's
                                sliding-window --temporal-period. Materializes
                                `changes_bucketed` via DuckDB
                                `date_trunc(<unit>, commit.date)` and routes
                                the coupling-family analyses through it.
                                Non-overlapping buckets — no commit-duplication
                                artifact.

  # ── Code-age cutoff (PAR-6) ───────────────────────────────────────
      --age-time-now YYYY-MM-DD Override "now" for the `code-age` analysis
                                (defaults to system clock UTC; useful for
                                reproducible historical reports).

  # ── Commit walk filters (R2 / R3 / R4) ────────────────────────────
      --after YYYY-MM-DD        Only include commits authored on or after
                                this date. Applied at repo-walk time so
                                the filter survives across every analysis.
                                Mirrors `git log --after`. Honored by both
                                GixRepo (default) and GitCliRepo backends.
      --before YYYY-MM-DD       Only include commits authored on or before
                                this date. Mirrors `git log --before`.
      --include-merges          Include merge commits in coupling / churn /
                                ownership analyses. Off by default (matches
                                code-maat semantics: merges duplicate
                                authorship and inflate co-change pairs).

  # ── Architectural grouping (PAR-7) ────────────────────────────────
  -g, --group-file PATH         Architectural grouping definition file with
                                full lookaround regex support (powered by
                                fancy-regex 0.14). Rewrites file paths at
                                ingest BEFORE coupling/hotspot/code-health
                                aggregation, so groups show up as first-class
                                entities. First-match-wins; plain-text LHS is
                                escaped + prefix-anchored + slash-bound;
                                explicit ^...$ regex on LHS is used as-is.

      --strict-grouping         When set, fail-fast if any change path matches
                                no group rule (default: paths with no rule are
                                kept under their original filename).
                                Auto-implied by --code-maat-compat.

  # ── Code-maat compatibility (PAR-9) ───────────────────────────────
      --code-maat-compat        Migration helper for code-maat scripts. Flips:
                                  • --strict-grouping ON
                                  • `main-dev-by-revs` CSV emits legacy
                                    `added`/`total-added` headers (matches
                                    code-maat output for piped tooling)
                                  • `soc` falls back to --min-revs for its
                                    threshold (the legacy overloaded semantic)

      --exclude PATTERN         Path glob to exclude (repeatable)
      --no-cache                Skip the persistent cache; always fresh ingest
      --cache-dir PATH          Override XDG cache root
  -v, --verbose                 Verbose logging (info,codelore=debug)
```

### `codelore diff` (PR-mode)

```
codelore diff <RANGE> [OPTIONS]
  RANGE                     <base>..<head>     direct compare
                            <base>...<head>    three-dot: resolves via git merge-base
                            (three-dot recommended for PR mode)

  -a, --analysis KIND       hotspots | coupling | clones | all  [default: hotspots]
                            (NB: diff's `coupling` corresponds to analyze's
                            `change-coupling`; the diff subcommand uses the
                            shorter form throughout)
  -r, --repo PATH           Git repo path [default: .]
      --top-n N             Hotspot rank threshold for entrant detection [default: 10]
      --score-threshold F   Min hotspot score delta to report [default: 0.05]
      --base-cache PATH     JSON file cache for the BASE rev analysis
                            (cuts dual-analysis cost in half across PRs)
  -f, --format FORMAT       text | json | sarif | markdown [default: text]
  -o, --output PATH         Write to file instead of stdout
      --fail-on CONDITION   Exit non-zero (4) when condition fires:
                            none (default) | rank-entrant | score-increase | any
      --absence-min-shared N
                            Min historical shared-revs for a coupling-absence
                            finding to be reportable [default: 3]
      --absence-fisher-p F  Max Fisher p-value for coupling-absence finding
                            [default: 0.05]
      --min-revs N          Same as analyze [default: 5]
      --exclude PATTERN     Same as analyze (repeatable)
```

The diff subcommand emits four SARIF rule types: CODELORE-HOTSPOT (newly-entering or score-rising hotspots), CODELORE-CLONE (PR-introduced clone families), CODELORE-LIVE-CLONE (PR-introduced live-clones), and CODELORE-MISSING-COCHANGE (historically-coupled partner files this PR didn't touch).

## 4. PR-mode: `codelore diff`

The form you actually deploy in CI. Three findings per range:

### Hotspot deltas

- **`rank_entrants`** — files newly entering the top-N at head. "This PR promoted `auth/login.rs` into the top-10 hotspots."
- **`score_increased`** — files in both top-N at base AND head, with `head.score − base.score ≥ --score-threshold`. "Worsened existing hotspot."
- **`pr_touched_existing`** — informational: PR-modified files that were already top-N at base. Context for the reviewer.

### Coupling absences (the CodeScene-signature signal)

Fires when a historically-strong pair (`shared >= 5 AND fisher_p < 0.05`) has **exactly one** member in the PR's changed set. "You changed `auth/login.rs` but historically `auth/session.rs` always changes with it. Did you forget?"

### Clone deltas

- **`new_families`** — clone families introduced by the PR (head fingerprints absent from base).
- **`pr_touched_existing`** — PR modified an existing clone-family member (didn't introduce new debt but didn't fix the existing kind either).

### Quality gate

```bash
codelore diff origin/main...HEAD --fail-on rank-entrant   # block PRs that create new hotspots
codelore diff origin/main...HEAD --fail-on score-increase # block PRs that worsen any hotspot
codelore diff origin/main...HEAD --fail-on any            # block on any finding
```

Exit 4 (the analysis-failure code) when the condition fires. Start with `--fail-on none` for a sprint to calibrate the noise floor, then raise the bar.

## 5. Configuration: `.codeloreignore` + thresholds

### `.codeloreignore`

Drop a file at the repo root with one glob per line. `#` comments + blank lines ignored (gitignore convention). Honored by `clones` today; rolling out to the rest of the analyses next.

```
# .codeloreignore — vendored / generated code
vendor/**
**/*_generated.rs
node_modules/**
target/**
```

### Built-in defaults

These thresholds match code-maat unless noted. Override via CLI flags (some) or the `Options` struct (all, if you call from Rust):

| Knob | Default | Source |
|---|---:|---|
| `min_revs` | 5 | code-maat parity |
| `min_shared_revs` | 5 | code-maat parity |
| `min_coupling_pct` | 30 | code-maat parity |
| `max_changeset_size` | 30 | code-maat parity |
| `fisher_significance` | 0.05 | conventional statistical-significance threshold |
| `min_clone_node_count` | 30 | ≈ 5–8 statements |
| `min_clone_shared_revs` | 3 | research brief (Fisher reliability floor) |
| `clone_similarity_floor` | 0.70 | SourcererCC BCB benchmark optimum |
| `clone_skip_same_dir` | true | drops intentional mirroring like `foo_test.rs ↔ foo.rs` |

## 6. Identity resolution (mailmap, bot filtering, AI authorship)

CodeLore's author-based analyses (`code-ownership`, `authors`, `author-churn`, `communication`) depend on resolving the *same person* across the different identities they commit under. Three layers do this work:

### 6.1 Mailmap consolidation

If a developer commits under multiple emails (`alice@oldcorp.com`, `alice@newcorp.com`, `alice.smith@personal.dev`), the repository's `.mailmap` file is the canonical place to declare them as one person. CodeLore reads `.mailmap` at the repo root and applies it before any author-based aggregation. Both name-and-email and email-only lines are supported per git's mailmap format.

Example `.mailmap`:

```
Alice Smith <alice@canonical.dev> <alice@oldcorp.com>
Alice Smith <alice@canonical.dev> <alice@newcorp.com>
Alice Smith <alice@canonical.dev> Alice S. <alice.smith@personal.dev>
```

After resolution, all three of Alice's identities count as one author in every output.

### 6.2 Bot filtering

Automated commits (dependency-bump bots, CI bots, release bots) skew Conway-style metrics — a Dependabot PR that touches 47 files isn't a human collaboration signal. Each commit is checked against a built-in substring-match list (`identity/bots.rs::DEFAULT_BOT_PATTERNS`); a match in either the author email or the author name marks the commit as a bot commit:

- `dependabot[bot]`
- `github-actions[bot]`
- `claude-code[bot]`
- `copilot[bot]`
- `renovate[bot]`
- `pre-commit-ci[bot]`

Match is plain substring containment, so `dependabot[bot]@noreply.github.com` matches `dependabot[bot]`. Bot commits still land in the fact store (so you can still query them in SQL via the SQLite/Parquet export) but they get the `ai-authored` attribution and the author-based analyses treat them as automated agents rather than human contributors.

### 6.3 AI-authorship classification

Each commit is classified into one of three buckets and stamped in the `commits.ai_attribution` column:

| Class | Trigger (in priority order) |
|---|---|
| `ai-authored` | Author or committer matches one of the bot patterns above |
| `ai-assisted` | Commit message contains `Co-Authored-By: Claude`, `Co-Authored-By: Copilot`, or `Co-Authored-By: GitHub Copilot` |
| `human` | Default — no AI signals found |

The bot list and the assisted-trailer list are intentionally narrow; tools that don't publish a standardized trailer (or that you don't want to count as AI-assisted) won't be detected. The classification is informational today — no published analysis filters by it — but every commit carries the column so you can query it directly from the SQLite/Parquet export:

```sql
SELECT ai_attribution, COUNT(*) AS n FROM commits GROUP BY 1 ORDER BY n DESC;
```

## 7. Kamei change-feature vector

Every commit ingested by CodeLore is enriched with the 14-feature change vector from [Kamei et al.'s JIT-SDP work](https://ieeexplore.ieee.org/document/6341763) (Just-In-Time Software Defect Prediction). These features describe the *shape* of each change and are written to the `commits` table, so any analysis can join against them in SQL.

| # | Feature | Description |
|---|---|---|
| 1 | `ns` | Number of modified subsystems (top-level directories) |
| 2 | `nd` | Number of modified directories |
| 3 | `nf` | Number of modified files |
| 4 | `entropy` | Shannon entropy of the per-file change distribution — high entropy = tangled change across many files |
| 5 | `la` | Lines of code added |
| 6 | `ld` | Lines of code deleted |
| 7 | `lt` | Average size of touched files at the pre-change state |
| 8 | `fix` | 1 if the commit message matches bug/fix regex patterns, else 0 |
| 9 | `ndev` | Number of distinct developers who previously modified the touched files |
| 10 | `age` | Average days since the last modification of each touched file |
| 11 | `nuc` | Number of historical commits touching the same files (their "history density") |
| 12 | `exp` | Author's lifetime commit count in the repo as of this commit |
| 13 | `rexp` | Same as `exp` but with recent commits weighted higher (exponential decay) |
| 14 | `sexp` | Author's prior commit count in the **same subsystem** as the touched files |

These features land in `commits` for every commit. The published analyses don't yet expose them directly via CLI flags — they're foundation for future bug-prediction work — but you can query them right now via `--format sqlite` or `--format parquet` and the columns are there:

```sql
SELECT rev, fix, entropy, la, ld, ndev FROM commits WHERE fix = 1 ORDER BY entropy DESC LIMIT 10;
```

This surfaces the 10 highest-entropy bug-fix commits — useful for retrospective "tangled fix" detection.

## 8. Persistent cache mechanics

CodeLore caches the DuckDB fact store at `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`. Second invocation on the same `(repo_path, HEAD sha, options, schema_version, codelore_version)` opens read-only in ≈ 10 ms instead of re-walking history.

```bash
# Skip the cache (always fresh in-memory)
codelore analyze --analysis hotspots --no-cache

# Override the XDG root (useful in CI with per-job caches)
codelore analyze --analysis hotspots --cache-dir /tmp/codelore-cache

# Inspect the cache
ls "$(dirs -c codelore 2>/dev/null || echo $XDG_CACHE_HOME)/codelore/"
```

Eviction: 5 entries per repo + 2 GB global cap (LRU). Pruning runs after every successful miss-and-write.

**Parquet + SQLite formats bypass the cache** by design — they need a writable DuckDB connection to run `INSTALL/LOAD sqlite` and `COPY TO parquet`.

### Dirty-worktree cache hit warning

The cache key includes `head_sha` but NOT the working tree. That's correct for analyses that read only committed history (`revisions`, `coupling`, `ownership`, `churn`, `messages`, ...), but `hotspots`-style HEAD-time metrics computed by `ingest_complexity_at_head` and `populate_clones_at_head` read files from disk at ingest time. If you change files without committing and then re-run codelore, the cache hits on `head_sha` — and you get the previous run's metrics computed from the previous worktree state, not your current edits.

To surface this, codelore emits a `tracing::warn!` whenever a cache hit lands on a working tree with uncommitted modifications or untracked Tier-1 source files:

```
WARN cache hit on a working tree with uncommitted changes; HEAD-time metrics
     (hotspots' complexity, clones) may be stale relative to disk.
     Pass `--no-cache` to recompute against the current working tree.
```

Detection is cheap (gix `Repository::status` for the pure-Rust walker, `git status --porcelain` for the CLI walker). Pass `--no-cache` if the dirty state matters for your analysis. The warning is informational — codelore still serves the cached result by default to preserve the 10–100× speedup on clean repeated runs. Auto-invalidation via worktree-content hashing was considered and rejected: hashing every tracked file on every invocation costs 100ms–1s on large trees, which would erase the cache's perf win for the majority case where the cache is correct.

## 9. Tool stack: why these choices

Every dependency in CodeLore was picked for a specific reason. The short version:

| Layer | Choice | Alternative considered | Why we picked this |
|---|---|---|---|
| Git read | `gix` (gitoxide) | `git2-rs` (libgit2 binding) | Pure Rust → no LGPL question, native `Send + Sync`, no C build deps, gix-blame is more accurate |
| Fact store | DuckDB (bundled) | Polars / SQLite / custom | Columnar analytics, spill-to-disk for kernel scale, SQL surface as a power-user feature, ZERO setup |
| Parsing | tree-sitter via vendored `rust-code-analysis` | per-language hand-rolled parsers | Battle-tested, language-agnostic, AST structural hashing for clones falls out for free |
| Concurrency | Rayon + crossbeam-channel | tokio | Workload is CPU-bound batch; async runtime is overkill and would force `Send` constraints we don't want |
| Statistics | `fishers_exact` | hand-rolled chi-square | Exact test (not approximate), zero-config, methodologically defensible at small N |
| CLI | `clap` 4 (derive macros) | `argh`, `gumdrop` | Industry standard, automatic `--help`, subcommand parsing |
| Output | `serde_json` + hand-rolled CSV + `sha2`/`hex` for SARIF fingerprints | — | Standard, minimal |
| Caching | `dirs` for XDG paths + DuckDB read-only mode | rolling our own | Conform to OS conventions (works on macOS, Linux, Windows) |
| Tests | `criterion` for benches + `assert_cmd`/`predicates` for CLI | — | Standard Rust test surfaces |

### What we deliberately don't use

- **No async runtime** — workload is CPU-bound batch; an async runtime would add binary size and `Send` constraints for no measurable throughput gain.
- **No libgit2** — gix already does everything we need, and pure-Rust matters for our supply chain story.
- **No LLM** — we're transparency-first. CodeScene's ML hotspot ranking is the opposite of what we ship. (LLM-based bug-link induction is a long-horizon research item with a pluggable interface.)
- **No web UI** — explicitly out-of-scope. Power users want SQL access to the fact store and SARIF in their existing CI dashboard; both are first-class outputs.

## 10. Performance characteristics

Per `docs/perf-evidence-v1.md` (warm-cache numbers):

| Repository | Commits | Source files | Wall (warm) | Peak RSS |
|---|---:|---:|---:|---:|
| codescene (this workspace) | ~95 | 131 .rs | 0.24 s | 89 MB |
| gitoxide (shallow 2000) | 9,985 | 2,903 | 1.16 s | 75 MB |
| tokio (shallow 3000) | 4,523 | 854 | 2.09 s | 230 MB |
| Linux kernel | 1.4M | 70k | < 10 min target | < 4 GB target |

The Linux kernel row is the spec's release-blocker target; the weekly CI bench job (`.github/workflows/bench.yml`) publishes the actual measurement once the cached snapshot reaches a stable baseline.

### Why tokio uses more memory than gitoxide despite fewer commits

Tree-sitter parsing + AST traversal dominate RSS for the Tier-1 file complexity extraction pass. tokio has roughly 3.5× the Rust source-line density per commit (deep generics in the runtime internals) compared to gitoxide. The commit-walk work scales with commit count; the complexity-extraction RSS scales with the number of Tier-1 source files at HEAD.

### Parallel vs serial complexity extraction

The complexity-extraction pass uses Rayon by default (one task per source file). On the `medium_repo` fixture (25 Rust files), parallel vs serial measure within bench noise (≈ 56 ms either way) because the bottleneck is the commit walk + change-feature enrichment SQL, not the parse pass. The parallel pass beats serial measurably on codebases with hundreds of Tier-1 files. Set `RAYON_NUM_THREADS=1` in the env before invoking `codelore` to force serial mode for comparison runs.

## 11. CI/CD integration patterns

### GitHub Actions (the canonical pattern)

See [`examples/.github/workflows/codelore-pr.yml`](../examples/.github/workflows/codelore-pr.yml) for the full template. Critical configuration:

- **`fetch-depth: 0`** in `actions/checkout` is mandatory. Without full history, hotspot scores are truncated to one commit and become meaningless. This is the single most common failure mode.
- **Three-dot merge-base notation** (`origin/main...HEAD`) scopes correctly to PR-only commits even when the base branch has moved since branch creation.
- **`security-events: write` permission** is required for SARIF upload to Code Scanning.
- **GHA cache integration** — pass `--cache-dir ${{ runner.temp }}/codelore-cache` and wrap with `actions/cache@v4` to persist across PRs.

### Quality gate rollout

| Phase | `--fail-on` | What it catches | When to advance |
|---|---|---|---|
| Pilot | `none` (default) | Nothing — advisory only | After 2 sprints of green runs |
| Soft enforce | `rank-entrant` | PRs that create new top-N hotspots | After team is comfortable interpreting findings |
| Strict | `score-increase` | PRs that worsen any existing hotspot | Once your codebase has stabilised |
| Maximum | `any` | Anything (including new clones + missing co-changes) | Mature teams in active refactor |

## 11.5. Per-stage timing (`RUST_LOG=codelore::bench=info`)

CodeLore instruments the three load-bearing stages of `analyze` —
opening the repo, looking up the cache (or ingesting from scratch),
and running the analysis + emitting output — with `tracing` spans
under the `codelore::bench` target. Default WARN-level filtering
suppresses them entirely (zero overhead), but opting in produces a
breakdown without any new flag:

```bash
RUST_LOG=codelore::bench=info codelore analyze \
  --repo path/to/repo --analysis hotspots > /dev/null
```

Each stage prints a `close` event with the elapsed time:

```
INFO bench.open_repo: close time.busy=2.4ms time.idle=15µs
INFO bench.cache_or_ingest: close time.busy=187ms time.idle=24µs
INFO bench.analyze_and_emit: close time.busy=43ms time.idle=18µs
```

For finer-grained timing, raise the level:
`RUST_LOG=codelore=debug` also shows the per-analysis spans inside
`codelore-lib` (cache hit/miss, materialize_changes_bucketed,
etc.). The default `--verbose` flag enables `info` for
`codelore` but not `codelore::bench`, so the bench-specific
spans stay out of normal-verbosity output.

## 12. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: ingest commits: repository error: find_parent_commit ... could not be found` | Shallow clone (`--depth=N`) is missing parent ancestry for analyses that walk back | Use a full clone or run only HEAD-only analyses (`clones` works on shallow clones — it short-circuits the ingest) |
| Hotspot scores are all `0.0` | Repo has only one commit, OR `fetch-depth: 0` not set in CI | Set `fetch-depth: 0` in `actions/checkout` |
| `codelore analyze --analysis bogus` errors with help-text | Typo on analysis name | The error message lists all 13 supported analyses |
| Same file appears twice in `revisions` output (e.g. `crates/bca-lib/foo.rs` AND `crates/codelore-lib/foo.rs`) | Git rename split — CodeLore doesn't follow renames yet | Known limitation; tracked in [`roadmap-v1.x-and-beyond.md`](roadmap-v1.x-and-beyond.md) (Tier 3, "Rename tracking") |
| `clone-coupling` returns 0 rows on a small repo | Fisher exact test needs ≥ 3 shared commits AND non-degenerate contingency table | Verify with `--analysis coupling` first; if that's empty too, the repo doesn't have enough history |
| `--format parquet` fails with "requires --output" | Binary format can't stream to stdout | Pass `--output FILE.parquet` |
| `--format sarif` fails with "supported: hotspots, clones, clone-coupling" | Other analyses don't have a SARIF rule yet | Use one of the supported analyses, or `--format json` |
| Disk space warning during `cargo test` | DuckDB bundled build is heavy (~3-4 GB target dir) | `cargo clean -p codelore-lib` to free; the next build is faster than a full clean |
| `cargo bench` errors on parallel/serial benches | rayon `build_global()` can only run once per process | The bench file uses per-iteration `pool.install()` which sidesteps this; only an issue if you write your own bench |

## 13. Workspace layout

```
codescene/
├── Cargo.toml                            # workspace manifest
├── README.md                             # the 5-min pitch
├── CHANGELOG.md                          # all releases
├── Containerfile                         # distroless image
├── examples/
│   └── .github/workflows/                # GHA integration templates
├── crates/
│   ├── codelore-lib/                     # the library
│   │   ├── src/
│   │   │   ├── facts/                    # DuckDB fact store + ingest pipeline
│   │   │   ├── analyses/                 # the 32 analyses (one file each)
│   │   │   ├── output/                   # 6 format emitters
│   │   │   ├── repo/                     # GixRepo + GitCliRepo + Repo trait
│   │   │   ├── complexity/               # tree-sitter dispatch + ComplexityEntity
│   │   │   ├── clones/                   # Type 1+2 fingerprinting
│   │   │   ├── identity/                 # mailmap + bots.toml
│   │   │   ├── kamei/                    # 14-feature change vector
│   │   │   ├── cache.rs                  # persistent fact-store cache
│   │   │   ├── provenance/               # manifest sidecar
│   │   │   └── options.rs                # the 25-field runtime config
│   │   ├── tests/                        # integration tests
│   │   └── benches/end_to_end.rs         # criterion harness
│   ├── codelore-cli/                     # clap CLI
│   │   └── src/
│   │       ├── main.rs                   # analyze dispatch
│   │       ├── args.rs                   # CLI surface
│   │       ├── diff.rs                   # codelore diff implementation
│   │       └── diff_output.rs            # diff output emitters
│   └── codelore-rca/                     # vendored Mozilla rust-code-analysis (MPL-2.0)
├── docs/
│   ├── advanced-usage.md                 # ← you are here
│   ├── codebase_analysis.md              # architecture overview (workspace + data flow)
│   ├── perf-evidence-v1.md               # release-blocker performance numbers
│   ├── roadmap-v1.x-and-beyond.md        # near-term and long-term backlog
│   └── superpowers/
│       ├── specs/                        # full design specification
│       └── plans/                        # every implementation plan, executed task-by-task
├── scripts/pgo.sh                        # PGO scaffolding (queued post first stable tag)
├── .github/workflows/
│   ├── ci.yml                            # cargo test + clippy + fmt + deny
│   ├── bench.yml                         # weekly perf regression gate
│   ├── release.yml                       # cargo-build matrix + SLSA L3 + Homebrew (on tag push)
│   └── container.yml                     # distroless image (on tag push)
└── .codeloreignore                       # optional, user-supplied
```
