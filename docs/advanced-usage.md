# CodeLore — Advanced Usage Guide

This guide is the developer-facing reference for CodeLore. The [README](../README.md) is the 5-minute pitch; this is the 30-minute manual.

## Table of contents

1. [The 14 analyses (what they tell you)](#1-the-14-analyses-what-they-tell-you)
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

## 1. The 14 analyses (what they tell you)

| Analysis | What you ask it | Formula / source | When to reach for it |
|---|---|---|---|
| `revisions` | "Which files change most often?" | `COUNT(DISTINCT rev)` per file | First-look for any unfamiliar repo |
| `hotspots` | "Which files are both complex AND change a lot?" | `percentile_rank(revs) × percentile_rank(cognitive) × (10 − code_health) / 10` ([see design spec](superpowers/specs/2026-06-06-codelore-design.md)) | The headline ranking signal — refactor priorities |
| `code-health` | "How healthy is each file's structure?" | 4-input composite: cognitive 0.40 + churn 0.25 + fragmentation 0.15 + coupling 0.20 | Combined with `hotspots`; tracks degradation |
| `code-age` | "Which files are stale vs. recently churned?" | Months since last commit per file | Find dead code + recently-volatile areas |
| `abs-churn` | "How fast does the team add/delete code?" | Lines added/deleted/commits grouped by date | Trend dashboards |
| `author-churn` | "Who contributes how much?" | Same as `abs-churn` grouped by canonical author (post-mailmap) | Effort distribution |
| `entity-churn` | "Which files churn the most?" | Same grouped by file | Pair with `hotspots` |
| `communication` | "Who works on the same code as whom?" (Conway's Law) | Author pairs by shared-work intensity | Team topology insight |
| `code-ownership` | "Is each file mainly owned by one person, or fragmented?" | Fractal Value = 1 − Herfindahl-Hirschman Index + main-developer | Bus-factor; knowledge-loss risk |
| `change-coupling` | "Which files always change together?" | Fisher exact-filtered logical (temporal) coupling at `p < 0.05` | Hidden architectural debt |
| `summary` | "Give me the one-page snapshot" | Commits + changes + entities + authors counts | First slide of any review |
| `authors` | "List all contributors and commit counts" | Canonical authors sorted desc | Onboarding; recognition |
| `clones` | "Where is code copy-pasted?" | Type 1 + Type 2 via AST structural hashing on tree-sitter | Refactoring candidates |
| `clone-coupling` | "Which copy-pasted blocks ALSO change together?" (the differentiator) | Clones JOIN coupling, Fisher-significant only | Live debt that hurts you on every change |

All analyses are pure SQL views over the DuckDB fact store + thin Rust orchestrators. You can run any analysis at any output format.

## 2. Output formats deep-dive

```bash
codelore analyze --analysis <NAME> --format <FORMAT>
```

| Format | Use case | Notes |
|---|---|---|
| `csv` (default) | Code-maat compatibility; pipe into other tools | Headers match code-maat exactly |
| `json` | Programmatic consumption | Pretty-printed; serde-derived |
| `markdown` | `$GITHUB_STEP_SUMMARY` in CI | GFM tables; one analysis per `# CodeLore <name>` header |
| `sarif` | GitHub Code Scanning / GitLab security / Defectdojo | SARIF 2.1.0; supported for `hotspots`, `clones`, `clone-coupling` today |
| `parquet` | DuckDB / Polars / pandas / Spark | `--output PATH` required; binary format |
| `sqlite` | Ad-hoc SQL exploration of the full fact store | `--output PATH` required; dumps all 8 tables |

Every file output (except SQLite, where it lives inside the DB) emits a `{output}.provenance.json` sidecar with the bca/gix/duckdb versions, every threshold knob, mailmap state, and UTC timestamp. This is your reproducibility receipt.

### SARIF rules CodeLore ships

| Rule ID | Tags | When it fires |
|---|---|---|
| `CODELORE-HOTSPOT` | `behavioral`, `hotspot` | One result per hotspot row, `security-severity = (100 − code_health) / 10` |
| `CODELORE-CLONE` | `behavioral`, `clone`, `type-1`, `type-2` | One result per clone family; `security-severity = 3 + family_size`, capped at 6 |
| `CODELORE-LIVE-CLONE` | `behavioral`, `clone`, `live-clone`, `co-change`, `x-ray` | One result per `(clone_group_id, file_a, file_b)`; `security-severity = combined_score × 10` |

All three use versioned `partialFingerprints` so cross-run identity stays stable.

## 3. Every CLI flag explained

### `codelore analyze`

```
codelore analyze [OPTIONS]
  -a, --analysis NAME       Which analysis [default: revisions]
                            (any of the 14 above)
  -r, --repo PATH           Git repo path [default: .]
  -f, --format FORMAT       Output format [default: csv]
                            csv | json | sarif | markdown | parquet | sqlite
  -o, --output PATH         Write to file instead of stdout
      --min-revs N          Min revisions per entity [default: 5]
      --rows N              Cap output to N rows
      --complexity-sample STRATEGY
                            head (default) | adaptive | full
                            (only `head` is wired up today; the other two parse but warn)
  -g, --group-file PATH     Architectural grouping definition file. Both the
                            flag and the file are parsed today, but the
                            resulting groups aren't yet wired into the
                            analyses themselves
      --exclude PATTERN     Path glob to exclude (repeatable)
      --no-cache            Skip the persistent cache; always fresh ingest
      --cache-dir PATH      Override XDG cache root
  -v, --verbose             Verbose logging (info,codelore=debug)
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
      --min-revs N          Same as analyze [default: 5]
      --exclude PATTERN     Same as analyze (repeatable)
```

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

## 12. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: ingest commits: repository error: find_parent_commit ... could not be found` | Shallow clone (`--depth=N`) is missing parent ancestry for analyses that walk back | Use a full clone or run only HEAD-only analyses (`clones` works on shallow clones — it short-circuits the ingest) |
| Hotspot scores are all `0.0` | Repo has only one commit, OR `fetch-depth: 0` not set in CI | Set `fetch-depth: 0` in `actions/checkout` |
| `codelore analyze --analysis bogus` errors with help-text | Typo on analysis name | The error message lists all 13 supported analyses |
| Same file appears twice in `revisions` output (e.g. `crates/bca-lib/foo.rs` AND `crates/codelore-lib/foo.rs`) | Git rename split — CodeLore doesn't follow renames yet | Known limitation; see the open-item backlog in [`codebase_analysis_report.md`](codebase_analysis_report.md) |
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
│   │   │   ├── analyses/                 # the 14 analyses (one file each)
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
│   ├── codebase_analysis_report.md       # validated improvement backlog
│   ├── perf-evidence-v1.md               # release-blocker performance numbers
│   ├── roadmap-v1.x-and-beyond.md        # near-term and long-term backlog
│   └── superpowers/
│       ├── specs/                        # full design specification
│       └── plans/                        # every implementation plan, executed task-by-task
├── scripts/pgo.sh                        # PGO scaffolding (queued post first stable tag)
├── .github/workflows/
│   ├── ci.yml                            # cargo test + clippy + fmt + deny
│   ├── bench.yml                         # weekly perf regression gate
│   ├── release.yml                       # cargo-dist + SLSA L3 (on tag push)
│   └── container.yml                     # distroless image (on tag push)
└── .codeloreignore                       # optional, user-supplied
```
