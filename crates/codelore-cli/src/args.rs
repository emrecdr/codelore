//! Clap argument definitions. CLI surface from spec §5.2.
//! Plan 1 ships only the minimum: `analyze`. `diff`, `query`, `facts`,
//! `explain`, `config`, `doctor`, `init` land in later plans.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "codelore", version, about = "CodeLore — Behavioral Code Analyzer", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Verbose logging
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run an analysis and emit results.
    Analyze(AnalyzeArgs),
    /// Run analyses at two revisions and emit the delta. Plan 8 §7.
    Diff(DiffArgs),
}

#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Analysis name (Plan 1 supports: revisions).
    #[arg(short, long, default_value = "revisions")]
    pub analysis: String,

    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Output format: csv | json | sarif | markdown | parquet | sqlite.
    /// sarif: hotspots only. parquet: hotspots, revisions, summary; requires --output.
    /// sqlite: full fact-store dump; requires --output.
    #[arg(short, long, default_value = "csv")]
    pub format: String,

    /// Write output to file instead of stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Minimum revisions per entity (code-maat parity).
    #[arg(long, default_value_t = 5)]
    pub min_revs: u32,

    /// Limit output to N rows.
    #[arg(long)]
    pub rows: Option<u32>,

    /// Complexity sampling strategy: head (default) | adaptive | full.
    /// Plan 4 ships head only; adaptive and full land in Plan 5.
    #[arg(long, default_value = "head")]
    pub complexity_sample: String,

    /// Architectural grouping file (one `glob => group` mapping per line, code-maat parity).
    /// Plan 8 §2 Task 7: flag is parsed and forwarded into Options; the actual
    /// aggregation logic (rewrite entity paths to group names) lands in Plan 9.
    /// Today the flag is accepted but produces a warning.
    #[arg(short = 'g', long)]
    pub group_file: Option<PathBuf>,

    /// Path patterns to exclude from analyses (repeatable). Plan 8 §2 Task 8.
    /// Honored by `clones` today; other analyses gain support in Plan 9.
    /// A `.codeleignore` file in the repo root is also honored when present.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Skip the persistent fact-store cache and always run a fresh in-memory
    /// ingest. Useful when you suspect a stale cache or want reproducible timing.
    /// Plan 8 §3 Task 14.
    #[arg(long, default_value_t = false)]
    pub no_cache: bool,

    /// Override the XDG cache root for the persistent fact-store.
    /// Defaults to `$XDG_CACHE_HOME/codelore` (or the OS equivalent).
    /// Useful in CI environments that want per-job caches on a shared runner.
    /// Plan 8 §3 Task 14.
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
}

/// PR-mode delta analysis: run analyses at `<base>` and `<head>`, emit the diff.
/// Plan 8 §7.
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
    /// Valid values: hotspots, coupling, clones, all.
    #[arg(short, long, default_value = "hotspots")]
    pub analysis: String,

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
    #[arg(short, long, default_value = "text")]
    pub format: String,

    /// Write output to file instead of stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Exit non-zero when condition met. Values: `none`, `rank-entrant`,
    /// `score-increase`, `any`.
    #[arg(long, default_value = "none")]
    pub fail_on: String,

    /// Minimum revisions per entity for the underlying hotspot analyses.
    #[arg(long, default_value_t = 5)]
    pub min_revs: u32,

    /// Path patterns to exclude (repeatable). Same semantics as
    /// `analyze --exclude`.
    #[arg(long)]
    pub exclude: Vec<String>,
}
