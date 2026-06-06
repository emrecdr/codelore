//! bca — Behavioral Code Analyzer CLI.

mod args;

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use bca_lib::analyses::revisions::run_revisions;
use bca_lib::facts::FactsDb;
use bca_lib::output::csv::write_revisions_csv;
use bca_lib::repo::GixRepo;
use bca_lib::{AnalysisName, Options};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::args::{AnalyzeArgs, Cli, Command};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Analyze(args) => analyze(args),
    }
}

fn init_logging(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("info,bca=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn analyze(args: AnalyzeArgs) -> Result<()> {
    // Validate analysis name early — produces a clean error message
    // even though Plan 1 only runs `revisions`.
    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("parsing --analysis {:?}", args.analysis))?;
    if analysis != AnalysisName::Revisions {
        anyhow::bail!(
            "Plan 1 walking skeleton only supports --analysis revisions. \
             Full analysis set lands in Plan 4."
        );
    }
    if args.format != "csv" {
        anyhow::bail!(
            "Plan 1 walking skeleton only supports --format csv. \
             JSON, SARIF, Markdown, Parquet, SQLite land in Plan 5."
        );
    }

    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: args.min_revs,
        rows_limit: args.rows,
        ..Options::default()
    };

    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db = FactsDb::new_in_memory().context("open fact store")?;
    db.ingest(&repo, &opts).context("ingest commits")?;
    let rows = run_revisions(&db, &opts).context("run revisions analysis")?;

    let mut out: Box<dyn Write> = match args.output {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    write_revisions_csv(&rows, &mut out).context("write csv")?;
    Ok(())
}
