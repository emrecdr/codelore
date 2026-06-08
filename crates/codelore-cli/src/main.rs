//! codelore — Behavioral Code Analyzer CLI.

mod args;
mod diff;
mod diff_output;

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::{AnalysisName, Options};
use tracing_subscriber::EnvFilter;

use crate::args::{AnalyzeArgs, Cli, Command, DiffArgs};

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
        Command::Diff(args) => run_diff_cmd(&args),
    }
}

fn run_diff_cmd(args: &DiffArgs) -> Result<()> {
    let output = diff::run_diff(args).context("codelore diff")?;

    let mut out: Box<dyn Write> = match args.output.as_ref() {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    diff_output::emit(&mut out, &output, args.format.as_str())?;
    drop(out);

    if diff::should_fail(args, &output) {
        // Per spec §6.6: analysis-failure exit code is 4.
        std::process::exit(4);
    }
    Ok(())
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
    // SARIF: hotspots (Plan 5), clones (Plan 8 §2 T10), clone-coupling (Plan 8 §6 T21).
    if format == "sarif"
        && !matches!(
            analysis,
            AnalysisName::Hotspots | AnalysisName::Clones | AnalysisName::CloneCoupling
        )
    {
        anyhow::bail!(
            "--format sarif currently supports --analysis hotspots, clones, and clone-coupling (other analyses land in Plan 9)"
        );
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
        group_file: args.group_file.clone(),
        exclude_patterns: args.exclude.clone(),
        // PAR-6: code-maat parity flag wiring
        min_shared_revs: args.min_shared_revs,
        min_coupling_pct: args.min_coupling_pct,
        max_coupling_pct: args.max_coupling_pct,
        max_changeset_size: args.max_changeset_size,
        age_time_now: args.age_time_now,
        // R2/R3: date-range filters on the commit walk.
        after: args.after,
        before: args.before,
        // R4: opt-in merge-commit inclusion (default off matches code-maat
        // semantics and the GitCliRepo backend).
        include_merges: args.include_merges,
        message_regex: args.message_regex.clone(),
        min_soc: args.min_soc,
        // PAR-9: --code-maat-compat. Implies --strict-grouping (code-maat
        // is always-strict). Other compat behaviors are gated at the
        // analysis / emitter layer.
        code_maat_compat: args.code_maat_compat,
        strict_grouping: args.strict_grouping || args.code_maat_compat,
        // PAR-8: --time-bucket. Maps from the CLI's enum to the lib's enum.
        time_bucket: args.time_bucket.map(Into::into),
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

    // Parquet and SQLite formats require a writable DuckDB connection
    // (for INSTALL/LOAD sqlite extension and COPY TO parquet). These formats
    // always use fresh in-memory ingest and bypass the read-only cache.
    // For all other formats (csv/json/markdown/sarif) the persistent cache is used.
    let needs_writable_db = matches!(format, "parquet" | "sqlite");

    let db = if args.no_cache || needs_writable_db {
        // --no-cache (or writable-format requirement): always fresh in-memory.
        let db = FactsDb::new_in_memory().context("open fact store (in-memory)")?;
        db.ingest(&repo, &opts).context("ingest commits")?;
        db
    } else if let Some(cache_dir) = &args.cache_dir {
        // --cache-dir PATH: use a custom XDG root instead of the default.
        FactsDb::open_or_ingest_with_cache_root(&opts, &repo, cache_dir)
            .context("open or ingest (cache-dir)")?
    } else {
        // Default: use the XDG cache (read-only after first ingest).
        FactsDb::open_or_ingest(&opts, &repo).context("open or ingest")?
    };

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
            ("json", AnalysisName::Clones) => {
                let rows =
                    codelore_lib::analyses::clones::run_clones(&opts).context("run clones")?;
                codelore_lib::output::json::write_clones_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Clones) => {
                let rows =
                    codelore_lib::analyses::clones::run_clones(&opts).context("run clones")?;
                codelore_lib::output::markdown::write_clones_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            ("sarif", AnalysisName::Clones) => {
                let rows =
                    codelore_lib::analyses::clones::run_clones(&opts).context("run clones")?;
                let repo_root = args.repo.display().to_string();
                codelore_lib::output::sarif::write_clones_sarif(&rows, &repo_root, &mut out)
                    .context("write sarif")?;
            }
            (fmt, AnalysisName::Clones) => {
                anyhow::bail!("clones analysis supports csv|json|markdown|sarif; got {fmt:?}")
            }
            // --- authors (Plan 8 §2 Task 6) ---
            ("csv", AnalysisName::Authors) => {
                let rows = codelore_lib::analyses::authors::run_authors(&db, &opts)
                    .context("run authors")?;
                codelore_lib::output::csv::write_authors_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Authors) => {
                let rows = codelore_lib::analyses::authors::run_authors(&db, &opts)
                    .context("run authors")?;
                codelore_lib::output::json::write_authors_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Authors) => {
                let rows = codelore_lib::analyses::authors::run_authors(&db, &opts)
                    .context("run authors")?;
                codelore_lib::output::markdown::write_authors_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::Authors) => {
                anyhow::bail!("authors analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- soc (Sum of Coupling) ---
            ("csv", AnalysisName::Soc) => {
                let rows = codelore_lib::analyses::soc::run_soc(&db, &opts).context("run soc")?;
                codelore_lib::output::csv::write_soc_csv(&rows, &mut out).context("write csv")?;
            }
            ("json", AnalysisName::Soc) => {
                let rows = codelore_lib::analyses::soc::run_soc(&db, &opts).context("run soc")?;
                codelore_lib::output::json::write_soc_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Soc) => {
                let rows = codelore_lib::analyses::soc::run_soc(&db, &opts).context("run soc")?;
                codelore_lib::output::markdown::write_soc_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::Soc) => {
                anyhow::bail!("soc analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- messages (commit-message regex matcher) ---
            ("csv", AnalysisName::Messages) => {
                let rows = codelore_lib::analyses::messages::run_messages(&db, &opts)
                    .context("run messages")?;
                codelore_lib::output::csv::write_messages_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::Messages) => {
                let rows = codelore_lib::analyses::messages::run_messages(&db, &opts)
                    .context("run messages")?;
                codelore_lib::output::json::write_messages_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::Messages) => {
                let rows = codelore_lib::analyses::messages::run_messages(&db, &opts)
                    .context("run messages")?;
                codelore_lib::output::markdown::write_messages_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::Messages) => {
                anyhow::bail!("messages analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- main-dev (top author by lines added) ---
            ("csv", AnalysisName::MainDev) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev(&db, &opts)
                    .context("run main-dev")?;
                codelore_lib::output::csv::write_main_dev_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::MainDev) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev(&db, &opts)
                    .context("run main-dev")?;
                codelore_lib::output::json::write_main_dev_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::MainDev) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev(&db, &opts)
                    .context("run main-dev")?;
                codelore_lib::output::markdown::write_main_dev_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::MainDev) => {
                anyhow::bail!("main-dev analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- main-dev-by-revs (top author by revision count) ---
            ("csv", AnalysisName::MainDevByRevs) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev_by_revs(&db, &opts)
                    .context("run main-dev-by-revs")?;
                codelore_lib::output::csv::write_main_dev_by_revs_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
                .context("write csv")?;
            }
            ("json", AnalysisName::MainDevByRevs) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev_by_revs(&db, &opts)
                    .context("run main-dev-by-revs")?;
                codelore_lib::output::json::write_main_dev_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::MainDevByRevs) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev_by_revs(&db, &opts)
                    .context("run main-dev-by-revs")?;
                codelore_lib::output::markdown::write_main_dev_by_revs_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::MainDevByRevs) => {
                anyhow::bail!("main-dev-by-revs analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- main-dev-by-deletions (alias: refactoring-main-dev) ---
            ("csv", AnalysisName::MainDevByDeletions) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev_by_deletions(&db, &opts)
                    .context("run main-dev-by-deletions")?;
                codelore_lib::output::csv::write_main_dev_by_deletions_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::MainDevByDeletions) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev_by_deletions(&db, &opts)
                    .context("run main-dev-by-deletions")?;
                codelore_lib::output::json::write_main_dev_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::MainDevByDeletions) => {
                let rows = codelore_lib::analyses::main_dev::run_main_dev_by_deletions(&db, &opts)
                    .context("run main-dev-by-deletions")?;
                codelore_lib::output::markdown::write_main_dev_by_deletions_markdown(
                    &rows, &mut out,
                )
                .context("write markdown")?;
            }
            (fmt, AnalysisName::MainDevByDeletions) => {
                anyhow::bail!(
                    "main-dev-by-deletions analysis supports csv|json|markdown; got {fmt:?}"
                )
            }
            // --- entity-effort (per-author revs per file) ---
            ("csv", AnalysisName::EntityEffort) => {
                let rows = codelore_lib::analyses::entity_effort::run_entity_effort(&db, &opts)
                    .context("run entity-effort")?;
                codelore_lib::output::csv::write_entity_effort_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::EntityEffort) => {
                let rows = codelore_lib::analyses::entity_effort::run_entity_effort(&db, &opts)
                    .context("run entity-effort")?;
                codelore_lib::output::json::write_entity_effort_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::EntityEffort) => {
                let rows = codelore_lib::analyses::entity_effort::run_entity_effort(&db, &opts)
                    .context("run entity-effort")?;
                codelore_lib::output::markdown::write_entity_effort_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::EntityEffort) => {
                anyhow::bail!("entity-effort analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- entity-ownership (per-author churn per file) ---
            ("csv", AnalysisName::EntityOwnership) => {
                let rows =
                    codelore_lib::analyses::entity_ownership::run_entity_ownership(&db, &opts)
                        .context("run entity-ownership")?;
                codelore_lib::output::csv::write_entity_ownership_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::EntityOwnership) => {
                let rows =
                    codelore_lib::analyses::entity_ownership::run_entity_ownership(&db, &opts)
                        .context("run entity-ownership")?;
                codelore_lib::output::json::write_entity_ownership_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::EntityOwnership) => {
                let rows =
                    codelore_lib::analyses::entity_ownership::run_entity_ownership(&db, &opts)
                        .context("run entity-ownership")?;
                codelore_lib::output::markdown::write_entity_ownership_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::EntityOwnership) => {
                anyhow::bail!("entity-ownership analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- clone-coupling (Plan 8 §6) ---
            ("csv", AnalysisName::CloneCoupling) => {
                let rows = codelore_lib::analyses::clone_coupling::run_clone_coupling(&db, &opts)
                    .context("run clone-coupling")?;
                codelore_lib::output::csv::write_clone_coupling_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::CloneCoupling) => {
                let rows = codelore_lib::analyses::clone_coupling::run_clone_coupling(&db, &opts)
                    .context("run clone-coupling")?;
                codelore_lib::output::json::write_clone_coupling_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::CloneCoupling) => {
                let rows = codelore_lib::analyses::clone_coupling::run_clone_coupling(&db, &opts)
                    .context("run clone-coupling")?;
                codelore_lib::output::markdown::write_clone_coupling_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            ("sarif", AnalysisName::CloneCoupling) => {
                let rows = codelore_lib::analyses::clone_coupling::run_clone_coupling(&db, &opts)
                    .context("run clone-coupling")?;
                let repo_root = args.repo.display().to_string();
                codelore_lib::output::sarif::write_clone_coupling_sarif(
                    &rows, &repo_root, &mut out,
                )
                .context("write sarif")?;
            }
            (fmt, AnalysisName::CloneCoupling) => {
                anyhow::bail!(
                    "clone-coupling analysis supports csv|json|markdown|sarif; got {fmt:?}"
                )
            }
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
