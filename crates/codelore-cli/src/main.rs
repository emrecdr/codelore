//! codelore — Behavioral Code Analyzer CLI.

mod args;
mod diff;
mod diff_output;

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::{GixRepo, Repo as _};
use codelore_lib::{AnalysisName, Options};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

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
        Command::Analyze(args) => analyze(&args, cli.no_banner),
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
    // Emit a span-close event with elapsed time whenever a span exits.
    // Enables `RUST_LOG=codelore::bench=info codelore analyze …` to print
    // per-stage timing — no `--bench` flag needed. The CLOSE event is
    // suppressed by default at WARN level, so this has zero overhead for
    // normal runs.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_span_events(FmtSpan::CLOSE)
        .init();
}

#[allow(clippy::too_many_lines)]
fn analyze(args: &AnalyzeArgs, no_banner: bool) -> Result<()> {
    use codelore_lib::output::banner;
    // Bracket the whole run with a wall-clock timer so the footer can report
    // "completed in 4.3s". Started before any work so pre-flight, ingest,
    // analysis, and emit all count toward the displayed duration — matches
    // what `cargo build`'s `Finished in Xs` includes.
    let started_at = std::time::Instant::now();

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
        team_map_file: args.team_map_file.clone(),
        explain: args.explain,
        // Canonical lineage defaults to ON. `--no-canonical-lineage`
        // disables it explicitly; `--code-maat-compat` also disables
        // it to preserve bit-for-bit code-maat output (code-maat uses
        // `--no-renames` in its git log so it never canonicalizes).
        use_canonical_lineage: !args.no_canonical_lineage && !args.code_maat_compat,
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
        // T8: knowledge-islands analysis "departed author" threshold.
        departed_threshold_days: args.departed_threshold_days,
        ..Options::default()
    };

    // Catch pathological flag combinations (e.g. --min-coupling 60
    // --max-coupling 30) at the boundary rather than silently producing
    // empty output downstream.
    opts.validate().context("validate options")?;

    // F14 + F15 fix: `--time-bucket` is only semantically valid for
    // four analyses (coupling, soc, hotspots, code-health). The other
    // 18 either crash with a Catalog Error (no `changes_bucketed`
    // table — F14) or silently return empty rows (rev-on-rev JOIN
    // against the date-string-keyed bucketed table fails — F15). Reject
    // at the CLI boundary with a descriptive error rather than letting
    // either failure mode surprise the user downstream.
    if opts.time_bucket.is_some() && !analysis.supports_time_bucket() {
        anyhow::bail!(
            "--time-bucket is not supported for analysis {:?}. \
             Bucketing only applies to co-change analyses; supported: \
             coupling, soc, hotspots, code-health. Remove --time-bucket \
             or switch to one of those analyses.",
            analysis.as_str()
        );
    }

    let analysis_name = args.analysis.as_str();

    // Plan 7 clones is a HEAD-only filesystem + tree-sitter walk — no git
    // history is needed. Short-circuit BEFORE opening the repo so shallow
    // clones, working trees with uncommitted changes, and "not a git repo"
    // directories all work for any of the 4 clone-supporting output formats
    // (csv | json | markdown | sarif). Earlier this short-circuit only
    // covered `csv`; the other formats fell through to the full git-ingest
    // path, which was both ~10–100× slower and broke entirely on non-git
    // directories — a real user-facing bug closed here.
    if matches!(analysis, AnalysisName::Clones)
        && matches!(format, "csv" | "json" | "markdown" | "sarif")
    {
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };
        let rows = codelore_lib::analyses::clones::run_clones(&opts).context("run clones")?;
        match format {
            "csv" => {
                codelore_lib::output::csv::write_clones_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            "json" => {
                codelore_lib::output::json::write_clones_json(&rows, &mut out)
                    .context("write json")?;
            }
            "markdown" => {
                codelore_lib::output::markdown::write_clones_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            "sarif" => {
                let repo_root = args.repo.display().to_string();
                codelore_lib::output::sarif::write_clones_sarif(&rows, &repo_root, &mut out)
                    .context("write sarif")?;
            }
            _ => unreachable!("format validated by outer matches!()"),
        }
        return Ok(());
    }

    // Pre-flight banner: opens the repo, runs cheap validity checks (path
    // exists, is a git repo, has at least one commit, --output parent dir
    // is writable), emits the boxed Style-B banner to stderr, and either
    // returns the opened `GixRepo` (Ready) or bails with a banner-shaped
    // error explaining what's wrong. Failure banners ALWAYS print even when
    // `--no-banner` was passed — error feedback is too important to swallow.
    let repo = {
        let _span = tracing::info_span!(target: "codelore::bench", "bench.open_repo").entered();
        preflight_and_open_repo(args, &opts, analysis_name, no_banner)?
    };

    // F7 (refined): the persistent cache opens its DuckDB file read-only.
    // SQLite output requires `INSTALL sqlite; LOAD sqlite;` which writes
    // to the DuckDB extension registry on the connected database — that
    // cannot run on a read-only connection. Parquet output uses core
    // DuckDB's built-in `COPY ... TO file.parquet (FORMAT PARQUET)`,
    // which DOES work on read-only. So the bypass narrows from
    // `parquet|sqlite` (both) to `sqlite` only — parquet now benefits
    // from the cache speedup like csv/json/markdown/sarif.
    let needs_writable_db = format == "sqlite";

    let db = {
        let _span =
            tracing::info_span!(target: "codelore::bench", "bench.cache_or_ingest").entered();
        if args.no_cache || needs_writable_db {
            // --no-cache or sqlite output: always fresh in-memory.
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
        }
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
        let _span =
            tracing::info_span!(target: "codelore::bench", "bench.analyze_and_emit").entered();
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
                codelore_lib::output::csv::write_code_age_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
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
                codelore_lib::output::csv::write_communication_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
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
                codelore_lib::output::csv::write_ownership_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
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
                codelore_lib::output::csv::write_coupling_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
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
                codelore_lib::output::csv::write_summary_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
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
            // --- authors (per-entity Bird et al. risk indicator; modernised) ---
            ("csv", AnalysisName::Authors) => {
                let rows = codelore_lib::analyses::authors::run_authors(&db, &opts)
                    .context("run authors")?;
                codelore_lib::output::csv::write_authors_csv(
                    &rows,
                    &mut out,
                    opts.code_maat_compat,
                )
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
            // --- top-committers (per-author commit leaderboard) ---
            ("csv", AnalysisName::TopCommitters) => {
                let rows = codelore_lib::analyses::top_committers::run_top_committers(&db, &opts)
                    .context("run top-committers")?;
                codelore_lib::output::csv::write_top_committers_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::TopCommitters) => {
                let rows = codelore_lib::analyses::top_committers::run_top_committers(&db, &opts)
                    .context("run top-committers")?;
                codelore_lib::output::json::write_top_committers_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::TopCommitters) => {
                let rows = codelore_lib::analyses::top_committers::run_top_committers(&db, &opts)
                    .context("run top-committers")?;
                codelore_lib::output::markdown::write_top_committers_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::TopCommitters) => {
                anyhow::bail!("top-committers analysis supports csv|json|markdown; got {fmt:?}")
            }
            // --- knowledge-islands (T8: bus-factor / knowledge-loss risk) ---
            ("csv", AnalysisName::KnowledgeIslands) => {
                let rows =
                    codelore_lib::analyses::knowledge_islands::run_knowledge_islands(&db, &opts)
                        .context("run knowledge-islands")?;
                codelore_lib::output::csv::write_knowledge_islands_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            ("json", AnalysisName::KnowledgeIslands) => {
                let rows =
                    codelore_lib::analyses::knowledge_islands::run_knowledge_islands(&db, &opts)
                        .context("run knowledge-islands")?;
                codelore_lib::output::json::write_knowledge_islands_json(&rows, &mut out)
                    .context("write json")?;
            }
            ("markdown", AnalysisName::KnowledgeIslands) => {
                let rows =
                    codelore_lib::analyses::knowledge_islands::run_knowledge_islands(&db, &opts)
                        .context("run knowledge-islands")?;
                codelore_lib::output::markdown::write_knowledge_islands_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            (fmt, AnalysisName::KnowledgeIslands) => {
                anyhow::bail!("knowledge-islands analysis supports csv|json|markdown; got {fmt:?}")
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

    // Footer: closes the bracket opened by the pre-flight banner. Shows total
    // wall-clock time (humanised: "234ms" / "4.3s" / "2m 34s"). Suppressed
    // under the same conditions as the header — same `should_print` policy
    // ensures piped invocations stay clean.
    if banner::should_print(false, no_banner) {
        let footer = banner::Footer {
            analysis: analysis_name,
            elapsed: started_at.elapsed(),
            // Row counts plumbed through every (format, analysis) match arm
            // is a bigger refactor; deferred. The duration + analysis-name
            // line is the main UX win for v0.1.2.
            rows: None,
        };
        eprint!("{}", footer.render(banner::should_color()));
    }

    Ok(())
}

/// Pre-flight: cheap validations BEFORE the expensive ingest. Builds the
/// Style-B banner from `Options` + `GixRepo` state, prints to stderr (always
/// on failure, conditionally on success per `should_print`), and either
/// returns the opened repo (Ready) or bails with a clear error.
///
/// Checks run in order:
/// 1. `--repo` path exists on the filesystem
/// 2. Path opens as a git repository (gix-recognised)
/// 3. Repository has at least one commit (HEAD resolves)
/// 4. `--output` parent directory exists (catches typos before the 30s ingest)
fn preflight_and_open_repo(
    args: &AnalyzeArgs,
    opts: &Options,
    analysis_name: &str,
    no_banner: bool,
) -> Result<GixRepo> {
    use codelore_lib::CodeLoreError;
    use codelore_lib::output::banner::{self, Banner, Preflight};
    use codelore_lib::provenance::{DUCKDB_VERSION, GIX_VERSION};

    let repo_path_str = args.repo.display().to_string();
    let options_summary = format_options_summary(opts);
    let pkg_version = env!("CARGO_PKG_VERSION");

    // Helper: build a Banner with the given pre-flight state. Shared shell
    // keeps the failure renders consistent regardless of which check tripped.
    let make_banner =
        |branch: Option<String>, head_short: Option<String>, preflight: Preflight| Banner {
            codelore_version: pkg_version,
            gix_version: GIX_VERSION,
            duckdb_version: DUCKDB_VERSION,
            repo_path: repo_path_str.clone(),
            branch,
            head_short,
            analysis: analysis_name,
            options_summary: options_summary.clone(),
            preflight,
        };

    // Pre-flight failures MUST surface as typed `CodeLoreError` variants so
    // the spec §6.6 exit-code mapping in `main()` picks them up (Repo → 3,
    // Output → 5). A bare `anyhow::bail!` here would slip the chain and fall
    // through to the default exit code 1, breaking integration tests like
    // `invalid_repo_exits_with_code_3` and surprising any orchestrator that
    // dispatches on exit codes.

    // Step 1: does the path even exist on disk?
    if !args.repo.exists() {
        let b = make_banner(
            None,
            None,
            Preflight::RepoPathMissing {
                repo_path: repo_path_str.clone(),
            },
        );
        eprint!("{}", b.render(banner::should_color()));
        return Err(
            CodeLoreError::Repo(format!("--repo path does not exist: {repo_path_str}")).into(),
        );
    }

    // Step 2: open as git repo. gix returns an error for non-repo paths;
    // we translate into the `NotARepository` preflight variant for the user.
    // `GixRepo::open` already returns a `CodeLoreError::Repo`, so wrapping
    // with `anyhow::Error::new(e)` preserves the typed variant in the chain.
    let repo = match GixRepo::open(&args.repo) {
        Ok(r) => r,
        Err(e) => {
            let b = make_banner(
                None,
                None,
                Preflight::NotARepository {
                    repo_path: repo_path_str.clone(),
                },
            );
            eprint!("{}", b.render(banner::should_color()));
            return Err(anyhow::Error::new(e).context("open repo"));
        }
    };

    // Step 3: HEAD points to a commit. An empty repo (post `git init`, no
    // commits yet) makes every history-based analysis return nothing useful.
    let Ok(head_sha) = repo.head_sha() else {
        let b = make_banner(repo.head_branch_name(), None, Preflight::EmptyRepository);
        eprint!("{}", b.render(banner::should_color()));
        return Err(CodeLoreError::Repo("repository has no commits".to_string()).into());
    };
    let head_short: String = head_sha.chars().take(7).collect();
    let branch = repo.head_branch_name();

    // Step 4: `--output` parent dir exists. Fail-fast saves the user from
    // waiting 30s through an ingest only to discover the directory was a
    // typo. We don't actually try to open a write handle (that races with
    // legitimate file creation by the emitter); just check the parent dir.
    if let Some(out_path) = &args.output
        && let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        let b = make_banner(
            branch.clone(),
            Some(head_short.clone()),
            Preflight::OutputNotWritable {
                path: out_path.display().to_string(),
                reason: format!("parent directory does not exist: {}", parent.display()),
            },
        );
        eprint!("{}", b.render(banner::should_color()));
        return Err(CodeLoreError::Output(format!(
            "--output parent directory does not exist: {}",
            parent.display()
        ))
        .into());
    }

    // All green — print the Ready banner if conditions allow (TTY + not
    // suppressed), then hand the opened repo back to the caller.
    let b = make_banner(branch, Some(head_short), Preflight::Ready);
    if banner::should_print(false, no_banner) {
        eprint!("{}", b.render(banner::should_color()));
    }
    Ok(repo)
}

/// One-line summary of the tuning knobs that mattered for this run. Goes
/// into the banner's `Analysis:` row alongside the analysis name. We only
/// surface a small curated set — the full canonical Options JSON lives in
/// the provenance sidecar for reproducibility audits.
fn format_options_summary(opts: &Options) -> String {
    let mut parts = vec![format!("min-revs={}", opts.min_revs)];
    if let Some(n) = opts.rows_limit {
        parts.push(format!("rows={n}"));
    }
    if opts.include_merges {
        parts.push("merges=on".to_string());
    }
    if opts.code_maat_compat {
        parts.push("code-maat-compat".to_string());
    }
    if opts.explain {
        parts.push("explain".to_string());
    }
    parts.join(", ")
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
