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
        // Map BcaError to its spec §6.6 exit code if present in the chain.
        // Falls back to 1 for non-BcaError errors (e.g. clap parse errors).
        let code = e
            .chain()
            .find_map(|cause| cause.downcast_ref::<bca_lib::BcaError>())
            .map_or(1, bca_lib::BcaError::exit_code);
        std::process::exit(code);
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
    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("parsing --analysis {:?}", args.analysis))?;
    if args.format != "csv" {
        anyhow::bail!(
            "Plan 3 walking skeleton only supports --format csv. \
             JSON, SARIF, Markdown, Parquet, SQLite land in Plan 5."
        );
    }

    let complexity_sample = match args.complexity_sample.as_str() {
        "head" => bca_lib::options::ComplexitySample::Head,
        "adaptive" | "full" => anyhow::bail!(
            "Plan 4 walking skeleton only supports --complexity-sample head. \
             adaptive and full land in Plan 5."
        ),
        other => anyhow::bail!("unknown complexity-sample value: {other:?}"),
    };

    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: args.min_revs,
        rows_limit: args.rows,
        complexity_sample,
        ..Options::default()
    };

    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db = FactsDb::new_in_memory().context("open fact store")?;
    db.ingest(&repo, &opts).context("ingest commits")?;

    let mut out: Box<dyn Write> = match args.output {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };

    match analysis {
        AnalysisName::Revisions => {
            let rows = run_revisions(&db, &opts).context("run revisions analysis")?;
            write_revisions_csv(&rows, &mut out).context("write csv")?;
        }
        AnalysisName::Hotspots => {
            let rows = bca_lib::analyses::hotspots::run_hotspots(&db, &opts)
                .context("run hotspots analysis")?;
            bca_lib::output::csv::write_hotspots_csv(&rows, &mut out).context("write csv")?;
        }
        AnalysisName::CodeHealth => {
            let rows = bca_lib::analyses::code_health::run_code_health(&db, &opts)
                .context("run code-health analysis")?;
            bca_lib::output::csv::write_code_health_csv(&rows, &mut out).context("write csv")?;
        }
        _ => anyhow::bail!(
            "Plan 3 supports --analysis revisions | hotspots | code-health. \
             Other analyses land in Plan 4."
        ),
    }
    Ok(())
}
