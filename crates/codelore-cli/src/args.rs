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
