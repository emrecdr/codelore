//! codelore — Behavioral Code Analyzer CLI.

mod args;

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::{AnalysisName, Options};
use tracing_subscriber::EnvFilter;

use crate::args::{AnalyzeArgs, Cli, Command};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        // Map CodeLoreError to its spec §6.6 exit code if present in the chain.
        // Falls back to 1 for non-CodeLoreError errors (e.g. clap parse errors).
        let code = e
            .chain()
            .find_map(|cause| cause.downcast_ref::<codelore_lib::CodeLoreError>())
            .map_or(1, codelore_lib::CodeLoreError::exit_code);
        std::process::exit(code);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Analyze(args) => analyze(&args),
    }
}

fn init_logging(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("info,codelore=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

#[allow(clippy::too_many_lines)]
fn analyze(args: &AnalyzeArgs) -> Result<()> {
    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("parsing --analysis {:?}", args.analysis))?;

    let format = args.format.as_str();
    match format {
        "csv" | "json" | "sarif" | "markdown" | "parquet" | "sqlite" => {}
        other => anyhow::bail!(
            "unknown --format {other:?}. Supported: csv, json, sarif, markdown, parquet, sqlite"
        ),
    }

    // Format constraints
    if matches!(format, "parquet" | "sqlite") && args.output.is_none() {
        anyhow::bail!(
            "--format {format} requires --output PATH (binary format, cannot stream to stdout)"
        );
    }
    if format == "sarif" && !matches!(analysis, AnalysisName::Hotspots) {
        anyhow::bail!("--format sarif currently supports --analysis hotspots only (Plan 5 scope)");
    }

    let complexity_sample = match args.complexity_sample.as_str() {
        "head" => codelore_lib::options::ComplexitySample::Head,
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

    let analysis_name = args.analysis.as_str();

    // Plan 7 clones is HEAD-only filesystem walk — no git history needed.
    // Short-circuit before opening the repo so shallow clones, untracked
    // working trees, and "not a git repo" cases all work for clones.
    if matches!(analysis, AnalysisName::Clones) && format == "csv" {
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };
        let rows = codelore_lib::analyses::clones::run_clones(&opts).context("run clones")?;
        codelore_lib::output::csv::write_clones_csv(&rows, &mut out).context("write csv")?;
        return Ok(());
    }

    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db = FactsDb::new_in_memory().context("open fact store")?;
    db.ingest(&repo, &opts).context("ingest commits")?;

    // Parquet + SQLite write to file directly through DuckDB, not via Write trait.
    if format == "parquet" {
        let path = args.output.as_ref().expect("validated above");
        write_parquet(&db, &opts, analysis, path)?;
        write_provenance_sidecar(&db, &opts, analysis_name, path)?;
        return Ok(());
    }
    if format == "sqlite" {
        let path = args.output.as_ref().expect("validated above");
        codelore_lib::output::sqlite::write_full_fact_store_sqlite(&db, &opts, path)
            .context("write sqlite")?;
        // No sidecar — provenance table lives inside the SQLite DB.
        return Ok(());
    }

    // csv / json / sarif / markdown: stream through Write
    {
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };

        match (format, &analysis) {
            // --- revisions ---
            ("csv", AnalysisName::Revisions) => {
                let rows = codelore_lib::analyses::revisions::run_revisions(&db, &opts)
                    .context("run revisions")?;
                codelore_lib::output::csv::write_revisions_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Revisions) => {
                let rows = codelore_lib::analyses::revisions::run_revisions(&db, &opts)
                    .context("run revisions")?;
                codelore_lib::output::json::write_revisions_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Revisions) => {
                let rows = codelore_lib::analyses::revisions::run_revisions(&db, &opts)
                    .context("run revisions")?;
                codelore_lib::output::markdown::write_revisions_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- hotspots ---
            ("csv", AnalysisName::Hotspots) => {
                let rows = codelore_lib::analyses::hotspots::run_hotspots(&db, &opts)
                    .context("run hotspots")?;
                codelore_lib::output::csv::write_hotspots_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Hotspots) => {
                let rows = codelore_lib::analyses::hotspots::run_hotspots(&db, &opts)
                    .context("run hotspots")?;
                codelore_lib::output::json::write_hotspots_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Hotspots) => {
                let rows = codelore_lib::analyses::hotspots::run_hotspots(&db, &opts)
                    .context("run hotspots")?;
                codelore_lib::output::markdown::write_hotspots_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            ("sarif", AnalysisName::Hotspots) => {
                let rows = codelore_lib::analyses::hotspots::run_hotspots(&db, &opts)
                    .context("run hotspots")?;
                let repo_root = args.repo.display().to_string();
                codelore_lib::output::sarif::write_hotspots_sarif(&rows, &repo_root, &mut out)
                    .context("write sarif")?;
            }
            // --- code-health ---
            ("csv", AnalysisName::CodeHealth) => {
                let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts)
                    .context("run code-health")?;
                codelore_lib::output::csv::write_code_health_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::CodeHealth) => {
                let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts)
                    .context("run code-health")?;
                codelore_lib::output::json::write_code_health_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::CodeHealth) => {
                let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts)
                    .context("run code-health")?;
                codelore_lib::output::markdown::write_code_health_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- code-age ---
            ("csv", AnalysisName::CodeAge) => {
                let rows = codelore_lib::analyses::code_age::run_code_age(&db, &opts)
                    .context("run code-age")?;
                codelore_lib::output::csv::write_code_age_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::CodeAge) => {
                let rows = codelore_lib::analyses::code_age::run_code_age(&db, &opts)
                    .context("run code-age")?;
                codelore_lib::output::json::write_code_age_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::CodeAge) => {
                let rows = codelore_lib::analyses::code_age::run_code_age(&db, &opts)
                    .context("run code-age")?;
                codelore_lib::output::markdown::write_code_age_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- abs-churn ---
            ("csv", AnalysisName::AbsChurn) => {
                let rows = codelore_lib::analyses::churn::run_abs_churn(&db, &opts)
                    .context("run abs-churn")?;
                codelore_lib::output::csv::write_abs_churn_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::AbsChurn) => {
                let rows = codelore_lib::analyses::churn::run_abs_churn(&db, &opts)
                    .context("run abs-churn")?;
                codelore_lib::output::json::write_abs_churn_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::AbsChurn) => {
                let rows = codelore_lib::analyses::churn::run_abs_churn(&db, &opts)
                    .context("run abs-churn")?;
                codelore_lib::output::markdown::write_abs_churn_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- author-churn ---
            ("csv", AnalysisName::AuthorChurn) => {
                let rows = codelore_lib::analyses::churn::run_author_churn(&db, &opts)
                    .context("run author-churn")?;
                codelore_lib::output::csv::write_author_churn_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::AuthorChurn) => {
                let rows = codelore_lib::analyses::churn::run_author_churn(&db, &opts)
                    .context("run author-churn")?;
                codelore_lib::output::json::write_author_churn_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::AuthorChurn) => {
                let rows = codelore_lib::analyses::churn::run_author_churn(&db, &opts)
                    .context("run author-churn")?;
                codelore_lib::output::markdown::write_author_churn_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- entity-churn ---
            ("csv", AnalysisName::EntityChurn) => {
                let rows = codelore_lib::analyses::churn::run_entity_churn(&db, &opts)
                    .context("run entity-churn")?;
                codelore_lib::output::csv::write_entity_churn_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::EntityChurn) => {
                let rows = codelore_lib::analyses::churn::run_entity_churn(&db, &opts)
                    .context("run entity-churn")?;
                codelore_lib::output::json::write_entity_churn_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::EntityChurn) => {
                let rows = codelore_lib::analyses::churn::run_entity_churn(&db, &opts)
                    .context("run entity-churn")?;
                codelore_lib::output::markdown::write_entity_churn_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- communication ---
            ("csv", AnalysisName::Communication) => {
                let rows = codelore_lib::analyses::communication::run_communication(&db, &opts)
                    .context("run communication")?;
                codelore_lib::output::csv::write_communication_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Communication) => {
                let rows = codelore_lib::analyses::communication::run_communication(&db, &opts)
                    .context("run communication")?;
                codelore_lib::output::json::write_communication_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Communication) => {
                let rows = codelore_lib::analyses::communication::run_communication(&db, &opts)
                    .context("run communication")?;
                codelore_lib::output::markdown::write_communication_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- ownership ---
            ("csv", AnalysisName::Ownership) => {
                let rows = codelore_lib::analyses::ownership::run_ownership(&db, &opts)
                    .context("run ownership")?;
                codelore_lib::output::csv::write_ownership_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Ownership) => {
                let rows = codelore_lib::analyses::ownership::run_ownership(&db, &opts)
                    .context("run ownership")?;
                codelore_lib::output::json::write_ownership_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Ownership) => {
                let rows = codelore_lib::analyses::ownership::run_ownership(&db, &opts)
                    .context("run ownership")?;
                codelore_lib::output::markdown::write_ownership_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- coupling ---
            ("csv", AnalysisName::Coupling) => {
                let rows = codelore_lib::analyses::coupling::run_coupling(&db, &opts)
                    .context("run coupling")?;
                codelore_lib::output::csv::write_coupling_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Coupling) => {
                let rows = codelore_lib::analyses::coupling::run_coupling(&db, &opts)
                    .context("run coupling")?;
                codelore_lib::output::json::write_coupling_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Coupling) => {
                let rows = codelore_lib::analyses::coupling::run_coupling(&db, &opts)
                    .context("run coupling")?;
                codelore_lib::output::markdown::write_coupling_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- summary ---
            ("csv", AnalysisName::Summary) => {
                let rows = codelore_lib::analyses::summary::run_summary(&db, &opts)
                    .context("run summary")?;
                codelore_lib::output::csv::write_summary_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Summary) => {
                let rows = codelore_lib::analyses::summary::run_summary(&db, &opts)
                    .context("run summary")?;
                codelore_lib::output::json::write_summary_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Summary) => {
                let rows = codelore_lib::analyses::summary::run_summary(&db, &opts)
                    .context("run summary")?;
                codelore_lib::output::markdown::write_summary_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            // --- clones (Plan 7) ---
            ("csv", AnalysisName::Clones) => {
                let rows =
                    codelore_lib::analyses::clones::run_clones(&opts).context("run clones")?;
                codelore_lib::output::csv::write_clones_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            (fmt, AnalysisName::Clones) => anyhow::bail!(
                "clones analysis currently supports --format csv only (json/markdown/sarif land in Plan 7.x); got {fmt:?}"
            ),
            // --- authors (reserved) ---
            (_, AnalysisName::Authors) => anyhow::bail!(
                "Plan 4 supports 11 analyses: revisions, hotspots, code-health, \
             code-age, abs-churn, author-churn, entity-churn, communication, \
             code-ownership, change-coupling, summary. \
             The 'authors' analysis lands in Plan 5."
            ),
            _ => unreachable!("format/analysis combination should have been validated above"),
        }
    } // out is dropped here, flushing any buffered writes

    if let Some(path) = args.output.as_ref() {
        write_provenance_sidecar(&db, &opts, analysis_name, path)?;
    }

    Ok(())
}

fn write_provenance_sidecar(
    db: &FactsDb,
    opts: &Options,
    analysis_name: &str,
    output_path: &std::path::Path,
) -> Result<()> {
    let manifest = codelore_lib::provenance::Manifest::capture(db, opts, analysis_name)
        .context("capture provenance manifest")?;
    let json = manifest
        .to_json()
        .context("serialize provenance manifest")?;
    let sidecar = std::path::PathBuf::from(format!("{}.provenance.json", output_path.display()));
    std::fs::write(&sidecar, json)
        .with_context(|| format!("write provenance sidecar to {}", sidecar.display()))?;
    Ok(())
}

fn write_parquet(
    db: &FactsDb,
    opts: &Options,
    analysis: AnalysisName,
    path: &std::path::Path,
) -> Result<()> {
    match analysis {
        AnalysisName::Hotspots => {
            codelore_lib::output::parquet::write_hotspots_parquet(db, opts, path)
                .context("write parquet")
        }
        AnalysisName::Revisions => {
            codelore_lib::output::parquet::write_revisions_parquet(db, opts, path)
                .context("write parquet")
        }
        AnalysisName::Summary => {
            codelore_lib::output::parquet::write_summary_parquet(db, opts, path)
                .context("write parquet")
        }
        other => anyhow::bail!(
            "--format parquet currently supports hotspots, revisions, summary only \
             (Plan 5 scope); got {other:?}"
        ),
    }
}
