//! Clap argument definitions. CLI surface from spec §5.2.
//! Subcommands: `analyze`, `diff`, `query`, `facts`,
//! `explain`, `config`, `doctor`, `init`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use codelore_lib::cli_api::constants::{
    DEFAULT_FISHER_SIGNIFICANCE, DEFAULT_MAX_CHANGESET_SIZE, DEFAULT_MAX_COUPLING_PCT,
    DEFAULT_MIN_COUPLING_PCT, DEFAULT_MIN_REVS, DEFAULT_MIN_SHARED_REVS,
};

/// Output format for `codelore diff`. Strongly typed so a typo
/// (`--format mardkown`) is caught at parse time rather than silently
/// dispatching to a default.
#[derive(ValueEnum, Clone, Debug)]
#[clap(rename_all = "lowercase")]
pub enum DiffFormat {
    Text,
    Json,
    Sarif,
    Markdown,
}

impl DiffFormat {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Sarif => "sarif",
            Self::Markdown => "markdown",
        }
    }
}

/// Which analyses to diff. `All` runs hotspots + coupling-absences + clones.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum DiffAnalysisKind {
    Hotspots,
    Coupling,
    Clones,
    All,
}

impl DiffAnalysisKind {
    #[must_use]
    pub fn wants_hotspots(self) -> bool {
        matches!(self, Self::Hotspots | Self::All)
    }
    #[must_use]
    pub fn wants_coupling(self) -> bool {
        matches!(self, Self::Coupling | Self::All)
    }
    #[must_use]
    pub fn wants_clones(self) -> bool {
        matches!(self, Self::Clones | Self::All)
    }
}

/// Quality-gate trigger for `codelore diff --fail-on`.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub enum DiffFailOn {
    /// Never exit non-zero (advisory mode).
    None,
    /// Exit non-zero when a file newly enters the top-N hotspots.
    RankEntrant,
    /// Exit non-zero when an existing hotspot's score increases ≥ threshold.
    ScoreIncrease,
    /// Exit non-zero on ANY finding (rank entrant + score increase + new
    /// clone family + coupling absence).
    Any,
}

#[derive(Parser, Debug)]
#[command(name = "codelore", version, about = "CodeLore — Behavioral Code Analyzer", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Verbose logging
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress the pre-flight banner (the box printed to stderr at the start
    /// of every analyze run showing version, repo, branch, analysis name, and
    /// pre-flight status). The banner is also auto-suppressed when stderr is
    /// not a TTY (e.g. when piping into `tee` or redirecting to a file). When
    /// a pre-flight CHECK fails the failure banner still prints — error
    /// feedback is too important to swallow.
    #[arg(long = "no-banner", global = true, default_value_t = false)]
    pub no_banner: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run an analysis and emit results.
    Analyze(AnalyzeArgs),
    /// Run analyses at two revisions and emit the delta.
    Diff(DiffArgs),
    /// Emit shell-completion script to stdout. Supported shells:
    /// bash | zsh | fish | powershell | elvish. Pipe to your shell's
    /// completion directory.
    Completions(CompletionsArgs),
    /// Print the formula + citation + SQL for any metric or analysis
    /// `CodeLore` exposes. Makes the auditable-formulas brand
    /// promise tactile on the CLI side.
    Explain(ExplainArgs),
    /// Emit JSON Schema 2020-12 for any `CodeLore` output row type.
    /// Integrates with downstream tools (Stoplight, Spectral,
    /// Postman, `OpenAPI` registries).
    Schema(SchemaArgs),
    /// Print operational telemetry — cache size, schema version, and
    /// per-analysis SQL preview. The "what's `CodeLore` doing under
    /// the hood?" subcommand.
    Profile,
    /// Emit a markdown documentation dump covering every supported
    /// analysis, formula, and citation. The seed of the planned full
    /// static-HTML doc site.
    Docs,
    /// Run quality-gate validation against `.codelore-thresholds.toml`
    /// at the repo root. Exit 0 on pass, non-zero on any violation.
    /// Writes `result=pass|fail` to `$GITHUB_OUTPUT` when the env var
    /// is set, for direct GitHub Actions step-output integration.
    Check(CheckArgs),
    /// Start a Model Context Protocol (MCP) server over stdio. Exposes
    /// `CodeLore` analyses as MCP tools for use by AI assistants and
    /// agent frameworks. Read-only — no network, no account, no
    /// telemetry. Warm-cache calls are cheap; first call on a cold
    /// cache pays the ingest cost.
    Mcp(McpArgs),
}

/// MCP server arguments.
#[derive(clap::Args, Debug)]
pub struct McpArgs {
    /// Path to the git repo to analyse (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: std::path::PathBuf,
}

/// Quality-gate check.
#[derive(clap::Args, Debug)]
pub struct CheckArgs {
    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,
    /// Optional explicit thresholds file. When omitted,
    /// `.codelore-thresholds.toml` at the repo root is auto-
    /// discovered. Empty/missing file → empty rule set → check
    /// passes vacuously.
    #[arg(long)]
    pub thresholds_file: Option<PathBuf>,
    /// Print the last 20 gate-run records from the local ledger, grouped
    /// by HEAD SHA. Does not run any gate evaluations.
    #[arg(long)]
    pub history: bool,
    /// Run the Betterer-style quality ratchet against `.codelore-ratchet.toml`
    /// at the repo root. First run writes the snapshot; subsequent runs fail on
    /// any regression and tighten the file on improvement.
    #[arg(long)]
    pub ratchet: bool,
    /// Suppress diagnostic noise (vacuous-pass messages, per-violation detail
    /// lines, inline degraded warnings) on stderr. The final verdict line
    /// (PASS / FAIL / WARNING) and exit code are never suppressed — they are
    /// the machine contract used by hooks and CI scripts.
    #[arg(long)]
    pub quiet: bool,
}

/// Shell-completion script generation.
#[derive(clap::Args, Debug)]
pub struct CompletionsArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

/// Metric / analysis explanation.
#[derive(clap::Args, Debug)]
pub struct ExplainArgs {
    /// Name of the metric or analysis to explain. Pass without an
    /// argument to list every supported topic.
    pub topic: Option<String>,
}

/// JSON Schema export.
#[derive(clap::Args, Debug)]
pub struct SchemaArgs {
    /// Name of the row type (e.g. `hotspots`, `god-classes`,
    /// `bus-factor`). Pass without an argument to list every type.
    pub row_type: Option<String>,
}

// AnalyzeArgs accumulates 4+ independent boolean flags (--no-cache, --verbose,
// --code-maat-compat, --strict-grouping, --include-merges) — each one toggles
// a semantically distinct, user-visible behavior. Clippy's heuristic that >3
// bools = "should be an enum" doesn't apply here; the flags are not mutually
// exclusive states of a single mode.
#[allow(clippy::struct_excessive_bools)]
#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Analysis name.
    #[arg(short, long, default_value = "revisions")]
    pub analysis: String,

    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Output format: csv | json | ndjson | sarif | markdown | gha | html | parquet | sqlite | spa.
    /// Most analyses emit csv/json/markdown. ndjson: hotspots, code-health, coupling, lead-time.
    /// sarif: hotspots, clones, clone-coupling. gha: hotspots. html: hotspots, code-health,
    /// knowledge-islands, clone-coupling, summary, revisions, authors, top-committers.
    /// parquet: hotspots, revisions, summary; requires --output. sqlite: full fact-store dump;
    /// requires --output. spa: interactive dashboard; --output optional (defaults to .codelore/spa.html).
    #[arg(short, long, default_value = "csv")]
    pub format: String,

    /// Write output to file instead of stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Minimum revisions per entity (code-maat parity).
    #[arg(long, default_value_t = DEFAULT_MIN_REVS)]
    pub min_revs: u32,

    /// Limit output to N rows.
    #[arg(long)]
    pub rows: Option<u32>,

    /// Complexity sampling strategy: head (default) | adaptive | full.
    #[arg(long, default_value = "head")]
    pub complexity_sample: String,

    /// Architectural grouping file (one `<lhs> => <group>` mapping per line, code-maat
    /// parity). Plain-text and regex rules are both accepted; `fancy-regex` powers the
    /// engine so lookaround is supported. Paths are rewritten at ingest before
    /// coupling/hotspot/code-health aggregation, so groups appear as first-class
    /// entities throughout the analysis pipeline.
    #[arg(short = 'g', long)]
    pub group_file: Option<PathBuf>,

    /// Optional CSV `author,team` mapping that aliases author identities
    /// to logical teams in every author-bearing analysis (`authors`,
    /// `author-churn`, `entity-ownership`, `main-dev`, `communication`,
    /// etc.). Mirrors code-maat's `-p / --team-map-file` flag; applied
    /// after mailmap normalization and bot filtering, so the resolved
    /// canonical identity is what gets aliased. If this flag is NOT
    /// passed, `CodeLore` auto-discovers `<repo>/.codelore-teams` and
    /// loads it transparently. Unmatched authors pass through unchanged.
    #[arg(long = "team-map-file", short = 'p')]
    pub team_map_file: Option<PathBuf>,

    /// Path patterns to exclude from analyses (repeatable).
    /// `.gitignore`, `.git/info/exclude`, and `.codeloreignore` in the
    /// repo root are auto-respected by default — vendored deps
    /// (`node_modules`, `target`, `dist`), lockfiles, locales, etc.
    /// don't show up in hotspots unless `--include-ignored` is passed.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Analyse files normally excluded by `.gitignore` /
    /// `.codeloreignore`. Default behaviour is to respect them so
    /// vendored deps, build outputs, and lockfiles don't dominate
    /// hotspots. Use this flag when analysing a vendored fork or
    /// when the lockfile IS the engineering signal you care about.
    #[arg(long = "include-ignored", default_value_t = false)]
    pub include_ignored: bool,

    /// Skip the persistent fact-store cache and always run a fresh in-memory
    /// ingest. Useful when you suspect a stale cache or want reproducible timing.
    #[arg(long, default_value_t = false)]
    pub no_cache: bool,

    /// Override the XDG cache root for the persistent fact-store.
    /// Defaults to `$XDG_CACHE_HOME/codelore` (or the OS equivalent).
    /// Useful in CI environments that want per-job caches on a shared runner.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Print the `DuckDB` optimizer plan for the analysis's underlying
    /// SQL to stderr before running the query. Useful for debugging
    /// performance ("which join was the dominator?") or for verifying
    /// that an index is being used. Wired for `hotspots` today;
    /// other analyses gain support in subsequent point releases.
    #[arg(long, default_value_t = false)]
    pub explain: bool,

    /// Disable rename-aware aggregation. By default, a file's pre-rename
    /// history is merged onto its current canonical path so renamed
    /// files don't show split revision counts. Set this flag to fall
    /// back to code-maat's literal-path behaviour. Implied by
    /// `--code-maat-compat`.
    #[arg(long, default_value_t = false)]
    pub no_canonical_lineage: bool,

    // ------------------------------------------------------------------
    // code-maat parity CLI flags (PAR-6). All target Options fields that
    // existed but weren't surfaced on the CLI.
    // ------------------------------------------------------------------
    /// Minimum shared revisions for a coupling pair (code-maat parity).
    /// Pairs below this floor are dropped before the Fisher gate runs.
    #[arg(long, default_value_t = DEFAULT_MIN_SHARED_REVS)]
    pub min_shared_revs: u32,

    /// Minimum coupling degree (0-100%) for a pair to surface in `coupling`.
    /// Code-maat parity; complements --min-shared-revs.
    #[arg(long = "min-coupling", default_value_t = DEFAULT_MIN_COUPLING_PCT)]
    pub min_coupling_pct: u8,

    /// Maximum coupling degree (0-100%) ceiling. Useful for narrowing the
    /// report to non-perfectly-coupled pairs (degree=100 means every
    /// commit modifies both files, often a sign of file split rather than
    /// a real signal).
    #[arg(long = "max-coupling", default_value_t = DEFAULT_MAX_COUPLING_PCT)]
    pub max_coupling_pct: u8,

    /// Drop commits touching more than N files from coupling/soc analyses.
    /// Filters refactor sweeps that create spurious coupling noise.
    /// Code-maat default is 30.
    #[arg(long, default_value_t = DEFAULT_MAX_CHANGESET_SIZE)]
    pub max_changeset_size: u32,

    /// Reference "now" for code-age analysis. Format: YYYY-MM-DD. Default:
    /// today UTC. Useful for reproducing a historic report or holding
    /// CI output stable across days.
    #[arg(long = "age-time-now", value_parser = parse_date)]
    pub age_time_now: Option<time::Date>,

    /// Only include commits authored on or after this date. Format: YYYY-MM-DD.
    /// Applied at repo-walk time so the filter survives across every analysis.
    /// Mirrors `git log --after`.
    #[arg(long, value_parser = parse_date)]
    pub after: Option<time::Date>,

    /// Only include commits authored on or before this date. Format: YYYY-MM-DD.
    /// Applied at repo-walk time. Mirrors `git log --before`.
    #[arg(long, value_parser = parse_date)]
    pub before: Option<time::Date>,

    /// Include merge commits in coupling / churn / ownership analyses. Off by
    /// default (matches code-maat semantics: merges duplicate authorship and
    /// inflate co-change pairs). Set to opt back in.
    #[arg(long)]
    pub include_merges: bool,

    /// Commit-message regex for the `messages` analysis. Required when
    /// `--analysis messages` is run; ignored otherwise. RE2-flavor (per
    /// `DuckDB` `regexp_matches`); use `(?i)` for case-insensitive matching.
    #[arg(short = 'e', long = "expression-to-match")]
    pub message_regex: Option<String>,

    /// Minimum `SoC` value for `soc` to surface a row. Modern replacement
    /// for code-maat's overloaded use of `--min-revs` in that one analysis.
    /// Default: drop solo commits (1).
    #[arg(long)]
    pub min_soc: Option<u32>,

    /// Migration helper: flip internal defaults back to legacy code-maat
    /// behavior. When set:
    ///   - `main-dev-by-revs` emits the lying `added`/`total-added` CSV
    ///     headers (instead of the honest `revisions`/`total-revisions`)
    ///   - `soc` falls back to `--min-revs` for its threshold (code-maat's
    ///     semantically-overloaded use) instead of the dedicated `--min-soc`
    ///   - `--strict-grouping` defaults to true (code-maat's always-strict
    ///     behavior) instead of `CodeLore`'s safer non-strict default
    ///
    /// Use only when dashboards downstream parse code-maat CSV verbatim.
    /// The modern surface is the recommendation; this flag is the safety
    /// net for users in migration who can't update their parsers yet.
    #[arg(long = "code-maat-compat", default_value_t = false)]
    pub code_maat_compat: bool,

    /// Strict-grouping mode: paths that don't match any rule in the
    /// `--group-file` are DROPPED from analysis output (code-maat's
    /// behavior). Default: false — unmapped paths keep their raw names
    /// (`CodeLore` safety divergence: silent data drop is a 2013 mistake).
    /// `--code-maat-compat` implies `--strict-grouping`.
    #[arg(long = "strict-grouping", default_value_t = false)]
    pub strict_grouping: bool,

    /// Time-bucket aggregation for coupling-family analyses (`coupling`,
    /// `clone-coupling`, `soc`). When set, commits within the same
    /// `date_trunc(<unit>, commit.date)` bucket count as a single
    /// "logical commit" for pair-counting purposes. Useful when teams
    /// land related changes across multiple small commits in the same
    /// day/week instead of one rollup commit.
    ///
    /// Modern replacement for code-maat's sliding-window `--temporal-period`
    /// hack. Backed by `DuckDB`'s `date_trunc()` — clean non-overlapping
    /// buckets, no commit-duplication artifact.
    ///
    /// Default: no bucketing (raw commit grain).
    #[arg(long = "time-bucket", value_enum)]
    pub time_bucket: Option<TimeBucketArg>,

    /// T8: An author is considered "departed" if their most recent
    /// commit anywhere in the repo is older than this many days at the
    /// anchor moment (used by `knowledge-islands` analysis).
    ///
    /// Default: 90 days (industry retention/sabbatical empirical
    /// threshold; engineers who leave permanently usually stop
    /// committing within 60 days, 90 avoids flagging extended-leave
    /// contributors).
    ///
    /// Lower for fast-moving startups (30-45); raise for academia /
    /// OSS maintainer codebases (180+).
    #[arg(
        long = "departed-threshold-days",
        default_value_t = codelore_lib::cli_api::constants::DEFAULT_DEPARTED_THRESHOLD_DAYS
    )]
    pub departed_threshold_days: u32,

    /// Trailing window in days for activity-scoped analyses (anchored to
    /// the repo's last commit). Valid range: 1–3650. Default: 90.
    #[arg(
        long = "window-days",
        default_value_t = codelore_lib::cli_api::constants::DEFAULT_WINDOW_DAYS
    )]
    pub window_days: u32,

    /// Knowledge model for `bus-factor`. `commits` (default): Filatov 2010
    /// greedy coverage of ≥80% of commits per module. `doe`: Cury & Avelino
    /// SBES'24 truck-factor procedure — greedy removal of the author with the
    /// most expert files until >50% of files lack an expert.
    #[arg(long = "knowledge-model", default_value = "commits", value_parser = ["commits", "doe"])]
    pub knowledge_model: String,

    /// Hunk-overlap window for rework detection in `delivery-metrics`.
    /// Hunk pairs on the same path where the second commit's author-date
    /// falls within this many days of the first are counted as rework.
    /// Valid range: 1–365. Default: 21.
    #[arg(
        long = "rework-window-days",
        default_value_t = codelore_lib::cli_api::constants::DEFAULT_REWORK_WINDOW_DAYS
    )]
    pub rework_window_days: u32,

    /// Glob pattern for filtering release tags in `release-cadence`.
    /// Only tags whose short name matches this glob are included.
    /// Must be non-empty. Default: `v*`.
    #[arg(
        long = "release-tag-glob",
        default_value = codelore_lib::cli_api::constants::DEFAULT_RELEASE_TAG_GLOB
    )]
    pub release_tag_glob: String,

    /// Target file path (repo-relative) for analyses that operate on a single
    /// file. Required by `function-xray` and `function-coupling`; ignored by
    /// all other analyses.
    #[arg(long)]
    pub target: Option<String>,
}

/// `TimeBucket` mirror on the CLI surface (clap-friendly value enum).
/// Maps 1:1 to `codelore_lib::cli_api::options::TimeBucket`.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[clap(rename_all = "lowercase")]
pub enum TimeBucketArg {
    Day,
    Week,
    Month,
}

impl From<TimeBucketArg> for codelore_lib::cli_api::options::TimeBucket {
    fn from(t: TimeBucketArg) -> Self {
        match t {
            TimeBucketArg::Day => Self::Day,
            TimeBucketArg::Week => Self::Week,
            TimeBucketArg::Month => Self::Month,
        }
    }
}

/// Parse a YYYY-MM-DD date for the date-valued flags (`--age-time-now`,
/// `--after`, `--before`).
fn parse_date(s: &str) -> std::result::Result<time::Date, String> {
    use time::format_description::well_known::Iso8601;
    time::Date::parse(s, &Iso8601::DEFAULT)
        .map_err(|e| format!("invalid date {s:?} (expected YYYY-MM-DD): {e}"))
}

/// PR-mode delta analysis: run analyses at `<base>` and `<head>`, emit the diff.
///
/// Rev range accepts two forms:
///   - `<base>..<head>` (two-dot): straight comparison
///   - `<base>...<head>` (three-dot): anchored to the merge-base — preferred
///     for PR mode because it scopes to PR-only commits even when the base
///     branch has moved since branch creation
#[derive(clap::Args, Debug)]
pub struct DiffArgs {
    /// Rev range: `<base>..<head>` or `<base>...<head>`. Three-dot uses
    /// the merge-base of `<base>` and `<head>` as the actual base SHA.
    pub range: String,

    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Analysis to diff. `all` runs hotspots + coupling-absences + clones.
    #[arg(short, long, value_enum, default_value_t = DiffAnalysisKind::Hotspots)]
    pub analysis: DiffAnalysisKind,

    /// Hotspot rank threshold. A file is a "rank-entrant" if it appears in
    /// the head's top-N hotspots but not the base's.
    #[arg(long, default_value_t = 10)]
    pub top_n: u32,

    /// Minimum hotspot-score delta (head - base) to report a
    /// "score-increased" finding.
    #[arg(long, default_value_t = 0.05)]
    pub score_threshold: f64,

    /// Path to a JSON file caching the BASE-rev analysis. If the file
    /// exists, the base analysis is loaded from it instead of recomputed.
    /// If absent, the freshly-computed base analysis is written there so
    /// the next PR run on the same base SHA hits the cache.
    #[arg(long)]
    pub base_cache: Option<PathBuf>,

    /// Output format. `text` is human-friendly terminal output;
    /// `markdown` is designed for `$GITHUB_STEP_SUMMARY`.
    #[arg(short, long, value_enum, default_value_t = DiffFormat::Text)]
    pub format: DiffFormat,

    /// Write output to file instead of stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Exit non-zero when condition met. Values: `none`, `rank-entrant`,
    /// `score-increase`, `any`.
    #[arg(long, value_enum, default_value_t = DiffFailOn::None)]
    pub fail_on: DiffFailOn,

    /// Minimum revisions per entity for the underlying hotspot analyses.
    #[arg(long, default_value_t = DEFAULT_MIN_REVS)]
    pub min_revs: u32,

    /// Path patterns to exclude (repeatable). Same semantics as
    /// `analyze --exclude`.
    #[arg(long)]
    pub exclude: Vec<String>,

    /// Minimum historical shared revisions for a coupling pair to count as
    /// a candidate for an absent-change warning. Pairs below this threshold
    /// have too-noisy a historical signal to act on. Default 5 (research
    /// brief mitigation 3); raise to 10+ for very large repos where weak
    /// pairs accumulate.
    #[arg(long, default_value_t = DEFAULT_MIN_SHARED_REVS)]
    pub absence_min_shared: u32,

    /// Fisher exact p-value gate for coupling absences. Pairs with
    /// p ≥ this value are not statistically significant; we don't warn
    /// about their absences. Default 0.05 (conventional significance
    /// threshold); 0.01 for stricter signal.
    #[arg(long, default_value_t = DEFAULT_FISHER_SIGNIFICANCE)]
    pub absence_fisher_p: f64,

    /// Optional path to a thresholds file (default name
    /// `.codelore-thresholds.toml`). When set, the file's `[diff]`
    /// section is evaluated against this run's deltas:
    ///
    /// - `delta_code_health_min` — minimum allowed
    ///   `head_median − base_median` code-health delta. Negative
    ///   values mean "drop tolerated up to this magnitude".
    /// - `new_hotspot_max` — maximum allowed count of rank-entrant
    ///   hotspots (files that enter top-N at head but were not
    ///   in top-N at base).
    ///
    /// Violations are surfaced on the output and force a non-zero
    /// exit (overriding `--fail-on=none`).
    #[arg(long)]
    pub thresholds_file: Option<PathBuf>,
}
