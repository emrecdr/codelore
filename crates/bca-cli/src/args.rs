//! Clap argument definitions. CLI surface from spec §5.2.
//! Plan 1 ships only the minimum: `analyze`. `diff`, `query`, `facts`,
//! `explain`, `config`, `doctor`, `init` land in later plans.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "bca", version, about = "Behavioral Code Analyzer", long_about = None)]
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
}
