//! Clap argument definitions. CLI surface from spec §5.2.
//! Subcommands: `analyze`, `diff`, `completions`, `explain`, `schema`,
//! `profile`, `docs`, `check`, `gate`, `mcp`, `ingest-sarif`, `calibrate`,
//! `calibrate-defects`.

use std::path::PathBuf;
use std::str::FromStr;

use clap::builder::{PossibleValue, PossibleValuesParser, TypedValueParser};
use clap::{Parser, Subcommand, ValueEnum};
use codelore_lib::cli_api::analysis::{AnalysisName, UnknownAnalysisError};
use codelore_lib::cli_api::constants::{
    DEFAULT_FISHER_SIGNIFICANCE, DEFAULT_MAX_CHANGESET_SIZE, DEFAULT_MAX_COUPLING_PCT,
    DEFAULT_MIN_COUPLING_PCT, DEFAULT_MIN_REVS, DEFAULT_MIN_SHARED_REVS,
};

/// The canonical `codelore analyze` output-format catalogue: `(name, description)`.
///
/// The single source of truth rendered by three surfaces — the `--format`
/// possible-values (and thus its parse-time validation and did-you-mean), the
/// `codelore profile` telemetry line, and the `codelore docs` catalogue. Adding a
/// format here surfaces it in all three at once, so the three lists can no longer
/// drift apart.
pub const ANALYZE_FORMATS: &[(&str, &str)] = &[
    ("csv", "code-maat-compatible flat tables"),
    ("json", "stable JSON shape per row type"),
    (
        "ndjson",
        "newline-delimited JSON — one row per line for stream consumers (LSP, `jq -c`, CI pipelines)",
    ),
    ("sarif", "SARIF 2.1.0 — surfaces in GitHub Code Scanning"),
    ("markdown", "GFM tables for `$GITHUB_STEP_SUMMARY`"),
    (
        "gha",
        "GitHub Actions workflow commands — `::error::` / `::warning::` / `::notice::` on stdout, surfaced as inline PR annotations",
    ),
    ("html", "self-contained per-analysis HTML report"),
    ("parquet", "columnar bulk export for analytical pipelines"),
    ("sqlite", "full DuckDB fact-store dump"),
    (
        "spa",
        "single-file interactive dashboard (opt-in via `spa` feature)",
    ),
    (
        "step-summary",
        "GFM summary for `$GITHUB_STEP_SUMMARY`; streams to stdout",
    ),
];

/// The format names alone, in catalogue order. Drives `--format` value parsing
/// and the compact `codelore profile` render.
#[must_use]
pub fn analyze_format_names() -> Vec<&'static str> {
    ANALYZE_FORMATS.iter().map(|(name, _)| *name).collect()
}

/// Clap value parser for `--analysis`. Owns the parse so an unknown value is a
/// parse-time error (exit 2) carrying clap's native possible-values list and
/// did-you-mean suggestion — the same contract `--format` and the other typed
/// flags already give. Resolution itself is delegated to [`AnalysisName::from_str`],
/// the one place that knows the canonical names AND the code-maat compatibility
/// aliases (`fragmentation`, `code-ownership`, `refactoring-main-dev`) and the
/// `identity` migration redirect, so this parser adds no second copy of that
/// knowledge.
#[derive(Clone)]
pub struct AnalysisNameParser;

impl TypedValueParser for AnalysisNameParser {
    type Value = AnalysisName;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        // The canonical possible-values set, shared by the delegated error path
        // below and by `possible_values()` (help + shell completions).
        let possible = || PossibleValuesParser::new(AnalysisName::all().iter().map(|a| a.as_str()));

        let Some(raw) = value.to_str() else {
            // Non-UTF-8: let the possible-values parser raise the canonical error.
            return Err(possible()
                .parse_ref(cmd, arg, value)
                .err()
                .unwrap_or_else(|| {
                    clap::Error::raw(clap::error::ErrorKind::InvalidUtf8, "invalid UTF-8\n")
                }));
        };

        match AnalysisName::from_str(raw) {
            Ok(name) => Ok(name),
            // Preserve the code-maat `identity` migration redirect, but as a
            // parser-level rejection so it shares the exit-2 arg-error contract.
            Err(e @ UnknownAnalysisError::IdentityRedirect) => {
                Err(clap::Error::raw(clap::error::ErrorKind::ValueValidation, e))
            }
            // Genuine unknown value: delegate to the possible-values parser for
            // clap's native "invalid value … [possible values: …]" plus its
            // did-you-mean tip. It rejects every value `from_str` rejected, so
            // this branch only ever yields an `Err`.
            Err(UnknownAnalysisError::Unknown(_)) => Err(possible()
                .parse_ref(cmd, arg, value)
                .err()
                .unwrap_or_else(|| {
                    clap::Error::raw(
                        clap::error::ErrorKind::InvalidValue,
                        format!("invalid value '{raw}'\n"),
                    )
                })),
        }
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(
            AnalysisName::all()
                .iter()
                .map(|a| PossibleValue::new(a.as_str())),
        ))
    }
}

/// Output format for `codelore check`. Strongly typed so a typo
/// (`--format sariff`) is caught at parse time rather than silently
/// falling back to text.
#[derive(ValueEnum, Clone, Debug)]
#[clap(rename_all = "lowercase")]
pub enum CheckFormat {
    Text,
    Sarif,
}

/// Output format for `codelore gate`. Strongly typed so a typo
/// (`--format josn`) is caught at parse time rather than silently
/// falling back to text.
#[derive(ValueEnum, Clone, Debug)]
#[clap(rename_all = "lowercase")]
pub enum GateFormat {
    Text,
    Json,
}

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
    /// Run an analysis and emit results. Boxed because `AnalyzeArgs` carries the
    /// widest flag surface of any subcommand — inlining it would bloat every
    /// `Command` value to its size.
    Analyze(Box<AnalyzeArgs>),
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
    /// Gate the current working tree against `.codelore-thresholds.toml`
    /// before committing: projects what the uncommitted edits do to
    /// code health and the import graph vs HEAD, and evaluates the
    /// working-tree `[diff]` gates against the projection. Exit 0 on
    /// pass, 1 on any violation — the same exit contract as `check`.
    Gate(GateArgs),
    /// Start a Model Context Protocol (MCP) server over stdio. Exposes
    /// `CodeLore` analyses as MCP tools for use by AI assistants and
    /// agent frameworks. Read-only — no network, no account, no
    /// telemetry. Warm-cache calls are cheap; first call on a cold
    /// cache pays the ingest cost.
    Mcp(McpArgs),
    /// Ingest one or more SARIF 2.1.0 files produced by external scanners
    /// (Semgrep, Clippy, `CodeQL`, etc.) into the per-repo external-findings
    /// sidecar store. Re-ingesting the same file is idempotent — findings
    /// are replaced per engine so the stored count is always the current
    /// scanner run, never an accumulation of duplicates.
    IngestSarif(IngestSarifArgs),
    /// Build a corpus-calibration artifact from a manifest of pinned repos.
    /// Each repo is ingested at its pinned SHA, its per-function raw metrics
    /// pooled per language, and the pooled distributions reduced to quantile
    /// breakpoints. The artifact powers the corpus-relative percentile lens
    /// (`--calibration`). Use `--merge` to fold the build into an existing
    /// artifact (e.g. your org's repos into the world corpus).
    Calibrate(CalibrateArgs),
    /// Mine a repository's own fix-commit history (AG-SZZ), validate whether
    /// `code-health` predicted where the mined defects landed, and — when the
    /// evidence clears an honesty floor — tune the eight smell weights to
    /// this repository. Writes a `defects.calib.json` artifact consumed by
    /// `--defect-calibration` on `analyze`/`check`.
    CalibrateDefects(CalibrateDefectsArgs),
}

/// MCP server arguments.
#[derive(clap::Args, Debug)]
pub struct McpArgs {
    /// Path to the git repo to analyse (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: std::path::PathBuf,

    /// Own-repo defect-calibration artifact (build one with `codelore
    /// calibrate-defects`). Adds a `defect-evidence` section to `explain_file`
    /// fact sheets. Hard error at server startup if the artifact was mined
    /// from a different repository — see --allow-foreign-calibration.
    #[arg(long)]
    pub defect_calibration: Option<PathBuf>,

    /// Apply a defect-calibration artifact mined from a different repository
    /// (forks, moved checkouts): skips the repo-identity guard.
    #[arg(long)]
    pub allow_foreign_calibration: bool,
}

/// Quality-gate check.
// CheckArgs accumulates 4 independent boolean flags (--history, --ratchet,
// --quiet, --allow-foreign-calibration) — each one toggles a semantically
// distinct, user-visible behavior. Clippy's heuristic that >3 bools =
// "should be an enum" doesn't apply here; the flags are not mutually
// exclusive states of a single mode.
#[allow(clippy::struct_excessive_bools)]
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
    /// Output format for violations: `text` (default, human-readable stderr
    /// lines) or `sarif` (SARIF 2.1.0 with evidence chains on stdout).
    /// Exit codes and verdict lines are unchanged regardless of format.
    #[arg(long, value_enum, default_value_t = CheckFormat::Text)]
    pub format: CheckFormat,
    /// Override the XDG cache root for the persistent fact-store and gate-run
    /// ledger. Defaults to `$XDG_CACHE_HOME/codelore` (or the OS equivalent).
    /// Useful in CI environments that want per-job caches on a shared runner.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Override the `DuckDB` spill directory used once a query's memory
    /// usage exceeds the internal ceiling (see docs/advanced-usage.md).
    /// Must already exist and be writable. Defaults to a subdirectory of
    /// the cache root.
    #[arg(long = "temp-dir")]
    pub temp_dir: Option<PathBuf>,

    /// Corpus-calibration artifact for the `code-health` corpus-percentile lens.
    /// Overrides the embedded world corpus; when omitted the embedded artifact
    /// is used if present.
    #[arg(long)]
    pub calibration: Option<PathBuf>,

    /// Own-repo defect-calibration artifact (build one with `codelore
    /// calibrate-defects`). Its smell weights replace the built-in code-health
    /// defaults for this run. Hard error if the artifact was mined from a
    /// different repository — see --allow-foreign-calibration.
    #[arg(long)]
    pub defect_calibration: Option<PathBuf>,

    /// Apply a defect-calibration artifact mined from a different repository
    /// (forks, moved checkouts): skips the repo-identity guard.
    #[arg(long)]
    pub allow_foreign_calibration: bool,
}

/// Working-tree quality gate.
#[derive(clap::Args, Debug)]
pub struct GateArgs {
    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,
    /// Optional explicit thresholds file. When omitted,
    /// `.codelore-thresholds.toml` at the repo root is auto-
    /// discovered. Empty/missing file → empty rule set → gate
    /// passes vacuously.
    #[arg(long)]
    pub thresholds_file: Option<PathBuf>,
    /// Suppress diagnostic noise (vacuous-pass messages, per-violation detail
    /// lines, advisory findings, the delta table, skip notices). The final
    /// verdict line (PASS / FAIL) and exit code are never suppressed — they
    /// are the machine contract used by hooks and CI scripts.
    #[arg(long)]
    pub quiet: bool,
    /// Output format: `text` (default, human-readable) or `json` (the full
    /// change-set report plus the evaluated violations as one document on
    /// stdout). Exit codes and verdict lines are unchanged regardless of
    /// format.
    #[arg(long, value_enum, default_value_t = GateFormat::Text)]
    pub format: GateFormat,
    /// Override the XDG cache root for the persistent fact-store, gate-run
    /// ledger, and change-set report sidecar. Defaults to
    /// `$XDG_CACHE_HOME/codelore` (or the OS equivalent).
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Override the `DuckDB` spill directory used once a query's memory
    /// usage exceeds the internal ceiling (see docs/advanced-usage.md).
    /// Must already exist and be writable. Defaults to a subdirectory of
    /// the cache root.
    #[arg(long = "temp-dir")]
    pub temp_dir: Option<PathBuf>,

    /// Own-repo defect-calibration artifact (build one with `codelore
    /// calibrate-defects`). Its smell weights replace the built-in code-health
    /// defaults for both scoped projection runs. Hard error if the artifact
    /// was mined from a different repository — see --allow-foreign-calibration.
    #[arg(long)]
    pub defect_calibration: Option<PathBuf>,

    /// Apply a defect-calibration artifact mined from a different repository
    /// (forks, moved checkouts): skips the repo-identity guard.
    #[arg(long)]
    pub allow_foreign_calibration: bool,
}

/// Shell-completion script generation.
#[derive(clap::Args, Debug)]
pub struct CompletionsArgs {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

/// Metric / analysis explanation, or a per-file evidence dossier.
#[derive(clap::Args, Debug)]
pub struct ExplainArgs {
    /// Name of the metric or analysis to explain, or a path to a tracked
    /// source file. A known topic prints its formula and citations; an
    /// existing file path (resolved under `--repo`) prints that file's
    /// deterministic evidence dossier. Pass without an argument to list
    /// every supported topic.
    pub topic: Option<String>,

    /// Path to the git repo used to resolve a file-path argument and load its
    /// facts (default: cwd). Ignored when the argument names a known topic.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,

    /// Append an advisory, LLM-generated narrative to a file dossier, grounded
    /// in the fact sheet and stamped with a citation-check verdict. Requires an
    /// LLM endpoint configured through the `CODELORE_LLM_*` environment
    /// (local-first by default). No effect when the argument names a topic.
    #[arg(long)]
    pub llm: bool,

    /// Regenerate the LLM narrative even when a cached one exists, replacing the
    /// sidecar cache entry. Only meaningful together with `--llm`.
    #[arg(long)]
    pub llm_refresh: bool,

    /// Override the XDG cache root for the persistent fact-store and the
    /// advisory narrative sidecar. Defaults to `$XDG_CACHE_HOME/codelore` (or
    /// the OS equivalent).
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Own-repo defect-calibration artifact (build one with `codelore
    /// calibrate-defects`). Adds a `defect-evidence` section to the file
    /// dossier. Hard error if the artifact was mined from a different
    /// repository — see --allow-foreign-calibration. Ignored when the
    /// argument names a known topic.
    #[arg(long)]
    pub defect_calibration: Option<PathBuf>,

    /// Apply a defect-calibration artifact mined from a different repository
    /// (forks, moved checkouts): skips the repo-identity guard.
    #[arg(long)]
    pub allow_foreign_calibration: bool,
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
    /// Analysis to run. An unknown value is rejected at parse time with the
    /// supported list and a did-you-mean suggestion.
    #[arg(
        short,
        long,
        value_parser = AnalysisNameParser,
        default_value_t = AnalysisName::Revisions
    )]
    pub analysis: AnalysisName,

    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Output format (see the possible-values list below for the full set).
    /// Most analyses emit csv/json/markdown. ndjson: hotspots, code-health,
    /// coupling, lead-time. sarif: hotspots, clones, clone-coupling. gha:
    /// hotspots. html: hotspots, code-health, knowledge-islands, clone-coupling,
    /// summary, revisions, authors, top-committers. parquet: hotspots, revisions,
    /// summary; requires --output. sqlite: full fact-store dump; requires
    /// --output. spa: interactive dashboard; --output optional (defaults to
    /// .codelore/spa.html). step-summary: GFM summary for
    /// `$GITHUB_STEP_SUMMARY`; streams to stdout.
    #[arg(
        short,
        long,
        default_value = "csv",
        value_parser = PossibleValuesParser::new(analyze_format_names())
    )]
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

    /// Complexity sampling strategy. Only `head` (metrics computed at HEAD) is
    /// currently implemented; it is the default and the sole accepted value.
    #[arg(long, default_value = "head", value_parser = ["head"])]
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

    /// Override the `DuckDB` spill directory used once a query's memory
    /// usage exceeds the internal ceiling (see docs/advanced-usage.md).
    /// Must already exist and be writable. Defaults to a subdirectory of
    /// the cache root, or the system temp directory when there is no
    /// cache root in play (e.g. `--no-cache`).
    #[arg(long = "temp-dir")]
    pub temp_dir: Option<PathBuf>,

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
    // code-maat parity CLI flags. All target Options fields that
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

    /// Apply a Benjamini-Hochberg false-discovery-rate correction to the
    /// Fisher co-change gate instead of the per-pair significance test.
    /// Controls the family-wise false-discovery rate across all tested
    /// coupling pairs; stricter than the per-pair gate. Off by default.
    #[arg(long = "fdr-correction", default_value_t = false)]
    pub fdr_correction: bool,

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

    /// Corpus-calibration artifact for the `code-health` corpus-percentile lens.
    /// Overrides the embedded world corpus with a hand-built or org-specific
    /// artifact (build one with `codelore calibrate`). When omitted, the
    /// embedded artifact is used if present; otherwise the corpus lens is absent
    /// and a one-time notice is printed.
    #[arg(long)]
    pub calibration: Option<PathBuf>,

    /// Own-repo defect-calibration artifact (build one with `codelore
    /// calibrate-defects`). Its smell weights replace the built-in code-health
    /// defaults for this run. Hard error if the artifact was mined from a
    /// different repository — see --allow-foreign-calibration.
    #[arg(long)]
    pub defect_calibration: Option<PathBuf>,

    /// Apply a defect-calibration artifact mined from a different repository
    /// (forks, moved checkouts): skips the repo-identity guard.
    #[arg(long)]
    pub allow_foreign_calibration: bool,
}

/// Warnings for analysis-scoped `analyze` flags that were explicitly set but are
/// ignored by the selected analysis. Not errors — deliberately sharing one flag
/// set across several analyses in a script is legitimate — but a silent no-op is
/// a UX trap, so `analyze` prints one advisory line per offending flag to stderr.
///
/// This is the single table pairing each flag with the analyses that honor it.
/// "Explicitly set" is detected structurally, with no argument-provenance
/// plumbing: `Option` flags by `Some`, defaulted flags by a value differing from
/// their default (so passing the default value explicitly is harmlessly not
/// warned about). Multi-owner flags — the coupling-family thresholds and
/// `--window-days`, each honored by a broad set of analyses — are intentionally
/// excluded; a false "ignored" warning would be worse than silence.
#[must_use]
pub fn ignored_flag_warnings(args: &AnalyzeArgs, analysis: AnalysisName) -> Vec<String> {
    use codelore_lib::cli_api::constants::{
        DEFAULT_DEPARTED_THRESHOLD_DAYS, DEFAULT_RELEASE_TAG_GLOB, DEFAULT_REWORK_WINDOW_DAYS,
    };
    let selected = analysis.as_str();
    let mut warnings = Vec::new();
    let mut consider = |flag: &str, honored_by: &[&str], is_set: bool| {
        if is_set && !honored_by.contains(&selected) {
            warnings.push(format!(
                "warning: --{flag} was set but is ignored by analysis `{selected}`; \
                 it is honored only by: {}",
                honored_by.join(", ")
            ));
        }
    };
    consider(
        "target",
        &["function-xray", "function-coupling"],
        args.target.is_some(),
    );
    consider(
        "expression-to-match",
        &["messages"],
        args.message_regex.is_some(),
    );
    consider("min-soc", &["soc"], args.min_soc.is_some());
    consider(
        "knowledge-model",
        &["bus-factor"],
        args.knowledge_model != "commits",
    );
    consider(
        "departed-threshold-days",
        &["knowledge-islands"],
        args.departed_threshold_days != DEFAULT_DEPARTED_THRESHOLD_DAYS,
    );
    consider(
        "rework-window-days",
        &["delivery-metrics"],
        args.rework_window_days != DEFAULT_REWORK_WINDOW_DAYS,
    );
    consider(
        "release-tag-glob",
        &["release-cadence"],
        args.release_tag_glob != DEFAULT_RELEASE_TAG_GLOB,
    );
    warnings
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

/// Ingest SARIF files into the external-findings sidecar.
#[derive(clap::Args, Debug)]
pub struct IngestSarifArgs {
    /// Path to the git repo (default: cwd). Determines which per-repo
    /// sidecar store receives the findings.
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// One or more SARIF 2.1.0 files to ingest. Each file may contain
    /// multiple runs and engines; findings are grouped by engine and
    /// replace the previous batch for that engine atomically.
    #[arg(required = true)]
    pub file: Vec<PathBuf>,

    /// Override the XDG cache root. Defaults to the same root used by
    /// `analyze` and `check`.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
}

/// Build a corpus-calibration artifact from a manifest of pinned repos.
#[derive(clap::Args, Debug)]
pub struct CalibrateArgs {
    /// Path to the corpus manifest (TOML). Each `[[repos]]` entry names a
    /// `source` (clone URL or local path), a pinned `sha`, and the advisory
    /// `languages` it contributes.
    #[arg(long, required = true)]
    pub repos: PathBuf,

    /// Where to write the built artifact (compact JSON).
    #[arg(long, required = true)]
    pub output: PathBuf,

    /// Optional existing artifact to fold this build into via sample-count-
    /// weighted quantile blending (an approximation — see the artifact docs).
    /// Repos and sample counts from both are summed.
    #[arg(long)]
    pub merge: Option<PathBuf>,

    /// Corpus vintage label stamped into the artifact. Defaults to a
    /// date-derived `corpus-YYYY-MM`.
    #[arg(long)]
    pub vintage: Option<String>,

    /// Override the XDG cache root used by the per-repo ingest. Defaults to
    /// the same root used by `analyze` and `check`.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
}

/// Mine a repository's own fix-commit history and build a `defects.calib.json`
/// artifact (own-repo defect calibration).
#[derive(clap::Args, Debug)]
pub struct CalibrateDefectsArgs {
    /// Path to the git repo to mine (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Where to write the built artifact (compact JSON).
    #[arg(long, required = true)]
    pub output: PathBuf,

    /// Artifact vintage label. Defaults to `defects-YYYY-MM-DD` (today's date).
    #[arg(long)]
    pub vintage: Option<String>,

    /// Restrict mining to fix commits within this many trailing days of the
    /// repo's last commit. Defects those fixes are traced back to may predate
    /// the window — only which FIXES are mined is narrowed. Omit to mine the
    /// full history.
    #[arg(long = "window-days")]
    pub window_days: Option<u32>,

    /// Override the `DuckDB` spill directory used once a query's memory
    /// usage exceeds the internal ceiling (see docs/advanced-usage.md). Must
    /// already exist and be writable. Defaults to the system temp directory
    /// — mining runs entirely in memory and has no persistent cache root to
    /// nest a default spill dir under.
    #[arg(long = "temp-dir")]
    pub temp_dir: Option<PathBuf>,

    /// Mine even though the working tree has uncommitted changes. Mining
    /// reads only committed state (git history + object-database blobs at
    /// HEAD), so uncommitted edits are invisible to it — the artifact
    /// describes the commit stamped as `head_at_mining`, not the tree on
    /// disk. Default: refuse loudly so that mismatch is a deliberate choice.
    #[arg(long, default_value_t = false)]
    pub allow_dirty: bool,
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

    /// Append an advisory, LLM-generated PR narrative to the diff, grounded in a
    /// deterministic fact sheet of the run's deltas and stamped with a
    /// citation-check verdict. Advisory only: the narrative never changes the
    /// deterministic findings, the gate verdict, or the exit code, and any
    /// failure to produce it is a stderr warning, not an error. Rendered for
    /// `text` and `markdown` output only; ignored for `json`/`sarif`. Requires
    /// an LLM endpoint configured through the `CODELORE_LLM_*` environment
    /// (local-first by default).
    #[arg(long)]
    pub llm: bool,

    /// Regenerate the LLM narrative even when a cached one exists, replacing the
    /// sidecar cache entry. Only meaningful together with `--llm`.
    #[arg(long)]
    pub llm_refresh: bool,
}
