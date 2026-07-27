//! `codelore analyze` — run a single analysis over a repository and emit it.
//!
//! Pre-flights and opens the repository, ingests (or reuses cached) facts,
//! dispatches the requested `--analysis` to its writer, and renders the result
//! in the requested `--format` (csv, json, ndjson, sarif, markdown, parquet,
//! sqlite, html, spa, step-summary, gha). The `spa` format builds the
//! single-page dashboard from the full analysis suite.

use std::io::Write;

use anyhow::{Context, Result};
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::{GixRepo, Repo as _};
use codelore_lib::cli_api::{AnalysisName, CodeLoreError, Options};
use codelore_lib::cli_api::{analyses, output};

use crate::args::AnalyzeArgs;
use crate::notice_corpus_lens_absent;

#[allow(clippy::too_many_lines)] // long but linear: pre-flight → ingest → format routing → emit; splitting would obscure the top-level orchestration flow
pub(crate) fn analyze(args: &AnalyzeArgs, no_banner: bool) -> Result<()> {
    use codelore_lib::cli_api::output::banner;
    // Bracket the whole run with a wall-clock timer so the footer can report
    // "completed in 4.3s". Started before any work so pre-flight, ingest,
    // analysis, and emit all count toward the displayed duration — matches
    // what `cargo build`'s `Finished in Xs` includes.
    let started_at = std::time::Instant::now();

    // `--analysis` is parsed and validated by clap (see `args::AnalysisNameParser`),
    // so it arrives already resolved — including the code-maat aliases.
    let analysis = args.analysis;

    // Advise when an analysis-scoped flag was explicitly set but the selected
    // analysis will ignore it. Stderr only — never changes results or exit code.
    for warning in crate::args::ignored_flag_warnings(args, analysis) {
        eprintln!("{warning}");
    }

    // `--format` is validated by clap against the canonical catalogue
    // (`args::ANALYZE_FORMATS`), so every value reaching here is supported.
    let format = args.format.as_str();

    // Format constraints. parquet + sqlite are binary fact-store dumps with no
    // sensible default filename, so they still require --output. `spa` defaults
    // to ./.codelore/spa.html under the current directory when --output is
    // omitted (handled in the spa block below).
    if matches!(format, "parquet" | "sqlite") && args.output.is_none() {
        return Err(CodeLoreError::Output(format!(
            "--format {format} requires --output PATH (binary format, cannot stream to stdout)"
        ))
        .into());
    }
    // step-summary can stream to stdout (it's small GFM text), but typically
    // gets redirected to $GITHUB_STEP_SUMMARY by the caller's CI workflow.
    // SARIF: hotspots, clones, clone-coupling.
    if format == "sarif"
        && !matches!(
            analysis,
            AnalysisName::Hotspots | AnalysisName::Clones | AnalysisName::CloneCoupling
        )
    {
        return Err(CodeLoreError::Analysis(
            "--format sarif currently supports --analysis hotspots, clones, and clone-coupling (other analyses land in Plan 9)"
                .to_string(),
        )
        .into());
    }

    // `--complexity-sample` is clap-restricted to `head` — the only implemented
    // sampling strategy — so it maps unconditionally.
    let complexity_sample = codelore_lib::cli_api::options::ComplexitySample::Head;

    let defect_calibration = codelore_lib::cli_api::quality_gates::resolve_defect_calibration(
        args.defect_calibration.clone(),
        &args.repo,
    )
    .context("resolve defect calibration")?;

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
        include_ignored: args.include_ignored,
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
        // --code-maat-compat. Implies --strict-grouping (code-maat
        // is always-strict). Other compat behaviors are gated at the
        // analysis / emitter layer.
        code_maat_compat: args.code_maat_compat,
        fdr_correction: args.fdr_correction,
        strict_grouping: args.strict_grouping || args.code_maat_compat,
        // --time-bucket. Maps from the CLI's enum to the lib's enum.
        time_bucket: args.time_bucket.map(Into::into),
        // T8: knowledge-islands analysis "departed author" threshold.
        departed_threshold_days: args.departed_threshold_days,
        window_days: args.window_days,
        knowledge_model: args.knowledge_model.clone(),
        rework_window_days: args.rework_window_days,
        release_tag_glob: args.release_tag_glob.clone(),
        target: args.target.clone(),
        calibration: args.calibration.clone(),
        defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        temp_dir: args.temp_dir.clone(),
        ..Options::default()
    };

    // Catch pathological flag combinations (e.g. --min-coupling 60
    // --max-coupling 30) at the boundary rather than silently producing
    // empty output downstream.
    opts.validate().context("validate options")?;

    // `--time-bucket` is only semantically valid for
    // four analyses (coupling, soc, hotspots, code-health). The other
    // 18 either crash with a Catalog Error (no `changes_bucketed`
    // table) or silently return empty rows (rev-on-rev JOIN
    // against the date-string-keyed bucketed table fails). Reject
    // at the CLI boundary with a descriptive error rather than letting
    // either failure mode surprise the user downstream.
    if opts.time_bucket.is_some() && !analysis.supports_time_bucket() {
        return Err(CodeLoreError::Analysis(format!(
            "--time-bucket is not supported for analysis {:?}. \
             Bucketing only applies to co-change analyses; supported: \
             coupling, soc, hotspots, code-health. Remove --time-bucket \
             or switch to one of those analyses.",
            analysis.as_str()
        ))
        .into());
    }

    let analysis_name = args.analysis.as_str();

    // clones is a HEAD-only filesystem + tree-sitter walk — no git
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
        let rows =
            codelore_lib::cli_api::analyses::clones::run_clones(&opts).context("run clones")?;
        emit_to_output_or_stdout(args.output.as_deref(), |out| {
            match format {
                "csv" => {
                    codelore_lib::cli_api::output::csv::write_clones_csv(&rows, out)
                        .context("write csv")?;
                }
                "json" => {
                    codelore_lib::cli_api::output::json::write_json(&rows, out)
                        .context("write json")?;
                }
                "markdown" => {
                    codelore_lib::cli_api::output::markdown::write_clones_markdown(&rows, out)
                        .context("write markdown")?;
                }
                "sarif" => {
                    codelore_lib::cli_api::output::sarif::write_clones_sarif(
                        &rows,
                        &sarif_repo_root(&args.repo),
                        out,
                    )
                    .context("write sarif")?;
                }
                _ => unreachable!("format validated by outer matches!()"),
            }
            Ok(())
        })?;
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

    // The persistent cache opens its DuckDB file read-only.
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
            // --no-cache or sqlite output: always fresh in-memory. Still
            // honors `--temp-dir` so a very large repo spills instead of
            // OOM-ing on this bypass-the-cache path.
            let db = FactsDb::new_in_memory_with_temp_dir(opts.temp_dir.as_deref())
                .context("open fact store (in-memory)")?;
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
        codelore_lib::cli_api::output::sqlite::write_full_fact_store_sqlite(&db, &opts, path)
            .context("write sqlite")?;
        // No sidecar — provenance table lives inside the SQLite DB.
        return Ok(());
    }
    if format == "spa" {
        // Mirrors `--format sqlite` shape: bypasses the per-analysis
        // match because the SPA is a multi-analysis composite, not a
        // single row-type emission. Each new widget gets a new field
        // on `SpaDashboard` and a new analysis call inside
        // `build_spa_dashboard`.
        #[cfg(feature = "spa")]
        {
            // --output is optional for spa. When omitted, default to
            // `.codelore/spa.html` under the current working directory,
            // creating the `.codelore` dir if needed.
            let default_path = std::path::PathBuf::from(".codelore/spa.html");
            let path: &std::path::Path = if let Some(p) = args.output.as_deref() {
                p
            } else {
                if let Some(parent) = default_path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                &default_path
            };
            run_spa_dispatch(&db, &opts, &args.repo, path)?;
            return Ok(());
        }
        #[cfg(not(feature = "spa"))]
        {
            anyhow::bail!(
                "--format spa requires CodeLore to be built with the `spa` Cargo feature. \
                 Reinstall with `cargo install codelore --features spa`, build from source \
                 with `cargo build --features spa`, or use a prebuilt binary from \
                 https://github.com/emrecdr/codelore/releases (which ship with `spa` enabled)."
            );
        }
    }
    if format == "step-summary" {
        // Reuses the same multi-analysis dispatch as `--format spa` —
        // a step-summary IS a different rendering of the same
        // SpaDashboard, sized for GitHub's $GITHUB_STEP_SUMMARY 1 MB
        // cap. Streams to stdout by default so callers can
        // `>> $GITHUB_STEP_SUMMARY` directly.
        #[cfg(feature = "spa")]
        {
            run_step_summary_dispatch(&db, &opts, &args.repo, args.output.as_deref())?;
            return Ok(());
        }
        #[cfg(not(feature = "spa"))]
        {
            anyhow::bail!(
                "--format step-summary requires CodeLore to be built with the `spa` Cargo feature \
                 (the step-summary writer consumes the same SpaDashboard as the SPA HTML emitter). \
                 Reinstall with `cargo install codelore --features spa`, build from source \
                 with `cargo build --features spa`, or use a prebuilt binary from \
                 https://github.com/emrecdr/codelore/releases (which ship with `spa` enabled)."
            );
        }
    }

    // csv / json / sarif / markdown / html: stream through Write
    {
        let _span =
            tracing::info_span!(target: "codelore::bench", "bench.analyze_and_emit").entered();

        // SARIF needs the repo root; HTML needs the page title plus a
        // generated-at timestamp. Bundle them once and hand the same context
        // to every per-analysis dispatch fn so each can pick the bits its
        // wired formats require. The timestamp is computed unconditionally —
        // it is read only by the HTML page chrome, so it is inert for every
        // other format.
        let now = time::OffsetDateTime::now_utc();
        let generated_at = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
        );
        let ctx = EmitCtx {
            // Canonicalized so `--repo .` and an absolute path yield the same
            // SARIF fingerprints (see `sarif_repo_root`).
            repo_root: sarif_repo_root(&args.repo),
            title: format!("CodeLore: {}", analysis.as_str()),
            generated_at,
            analysis,
        };

        // Atomic when writing to a file: an interrupted or failing run never
        // truncates a previous good output (it lands in a temp sibling first).
        emit_to_output_or_stdout(args.output.as_deref(), |out| {
            run_streaming_dispatch(
                &db,
                &repo,
                &opts,
                args.cache_dir.as_deref(),
                format,
                &ctx,
                out,
            )
        })?;
    } // the writer is dropped inside the helper, flushing before the rename

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
            // line carries the bulk of the post-run UX value.
            rows: None,
        };
        eprint!("{}", footer.render(banner::should_color()));
    }

    Ok(())
}

/// Side-channel context every per-analysis dispatch needs beyond `db`, `opts`,
/// and `format`: SARIF wants the repo root; HTML wants the page title and a
/// generated-at timestamp; the analysis identity feeds titling, the
/// unsupported-format list, and the not-wired guidance.
struct EmitCtx {
    repo_root: String,
    title: String,
    generated_at: String,
    analysis: AnalysisName,
}

/// Canonicalize `repo` for the SARIF fingerprint's `repo_root|path` key so an
/// analysis invoked as `--repo .` and as `--repo /abs/path` produces identical
/// fingerprints — otherwise every alert re-keys on invocation style and GitHub
/// churns them. Falls back to the raw path when canonicalize fails (e.g. the
/// path was removed mid-run). Mirrors the idiom in `check`/`diff` (canonicalize
/// then `to_string_lossy`), so a file flagged by more than one command coalesces
/// to a single alert.
fn sarif_repo_root(repo: &std::path::Path) -> String {
    repo.canonicalize()
        .unwrap_or_else(|_| repo.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Emit streaming output to `dest`: atomically to the file when `Some`, or to
/// stdout when `None`. On the file path the `emit` closure fills a temp sibling
/// that is renamed over the destination only after it succeeds (via
/// [`atomic_publish`](codelore_lib::cli_api::output::atomic_publish)), so an
/// interrupted or failing run never truncates a previous good output. Stdout
/// writes stream directly — there is nothing to publish atomically.
fn emit_to_output_or_stdout<F>(dest: Option<&std::path::Path>, emit: F) -> Result<()>
where
    F: FnOnce(&mut Box<dyn Write>) -> Result<()>,
{
    if let Some(path) = dest {
        codelore_lib::cli_api::output::atomic_publish(path, |tmp| {
            let mut out: Box<dyn Write> = Box::new(std::fs::File::create(tmp)?);
            emit(&mut out)
        })
    } else {
        let mut out: Box<dyn Write> = Box::new(std::io::stdout().lock());
        emit(&mut out)
    }
}

/// The streaming `--format` values each analysis's dispatch wires to a real
/// emitter, listed in the order they appear in the unsupported-format guidance.
/// This is the single source of truth the dispatch READS: [`unsupported_format`]
/// derives its advertised list from here, and the `registration_surfaces` tests
/// hold every analysis to account against it. Exhaustive over `AnalysisName`, so
/// a new variant will not compile until its wired set is declared. `csv`, `json`
/// and `markdown` are wired for every analysis; `html` is opt-in (the
/// [`HTML_WIRED`] set); a few analyses add `sarif`/`ndjson`/`gha`.
fn supported_formats(name: AnalysisName) -> &'static [&'static str] {
    use AnalysisName::{
        AbsChurn, ArchViolations, ArchitectureMetrics, ArchitectureRoles, ArchitectureTrend,
        AuthorChurn, Authors, BusFactor, Centrality, CloneCoupling, Clones, CodeAge,
        CodeFamiliarity, CodeHealth, Communication, Communities, CoordinationNeeds, Coupling,
        Crossing, CycleHealth, CycleOrigins, DefectValidation, DeliveryFriction, DeliveryMetrics,
        DependencyCycles, EffortExposure, EntityChurn, EntityEffort, EntityOwnership,
        FindingHotspotOverlap, FunctionCoupling, FunctionXray, GodClasses, HealthTrend,
        HotspotVelocity, Hotspots, Instability, KnowledgeIslands, LeadTime, MainDev,
        MainDevByDeletions, MainDevByRevs, MarginalOwnerRisk, Messages, ModularityViolations,
        Ownership, PairProgramming, RefactoringTargets, ReleaseCadence, Revisions, Soc, StaleCode,
        Summary, TeamComposition, TopCommitters, UnstableInterface,
    };
    // Three streaming formats every analysis wires, and the common variant that
    // adds a bespoke HTML emitter.
    const STREAM: &[&str] = &["csv", "json", "markdown"];
    const STREAM_HTML: &[&str] = &["csv", "json", "markdown", "html"];
    match name {
        Hotspots => &["csv", "json", "markdown", "sarif", "ndjson", "gha", "html"],
        CodeHealth | RefactoringTargets => &["csv", "json", "markdown", "ndjson", "html"],
        CloneCoupling => &["csv", "json", "markdown", "sarif", "html"],
        Coupling => &["csv", "json", "markdown", "ndjson"],
        Clones => &["csv", "json", "markdown", "sarif"],
        LeadTime => &["csv", "json", "ndjson", "markdown"],
        KnowledgeIslands | Summary | Revisions | Authors | TopCommitters => STREAM_HTML,
        HotspotVelocity
        | Ownership
        | CodeAge
        | AbsChurn
        | AuthorChurn
        | EntityChurn
        | Communication
        | Soc
        | Messages
        | MainDev
        | MainDevByRevs
        | MainDevByDeletions
        | EntityEffort
        | EntityOwnership
        | Centrality
        | Communities
        | GodClasses
        | ArchViolations
        | DependencyCycles
        | ArchitectureRoles
        | Instability
        | ArchitectureMetrics
        | ArchitectureTrend
        | HealthTrend
        | CycleOrigins
        | ModularityViolations
        | UnstableInterface
        | Crossing
        | StaleCode
        | PairProgramming
        | BusFactor
        | DeliveryFriction
        | DeliveryMetrics
        | EffortExposure
        | CodeFamiliarity
        | TeamComposition
        | CoordinationNeeds
        | MarginalOwnerRisk
        | ReleaseCadence
        | FunctionXray
        | FunctionCoupling
        | FindingHotspotOverlap
        | CycleHealth
        | DefectValidation => STREAM,
    }
}

/// The analyses whose dispatch wires a bespoke row-type HTML emitter. Every
/// other analysis returns [`html_not_wired`] for `--format html`. Drives both
/// that guidance string and the `html_emitter_set_is_documented` test, so a new
/// (or lost) `write_html` is named in exactly one place.
const HTML_WIRED: &[AnalysisName] = &[
    AnalysisName::Hotspots,
    AnalysisName::CodeHealth,
    AnalysisName::KnowledgeIslands,
    AnalysisName::CloneCoupling,
    AnalysisName::Summary,
    AnalysisName::Revisions,
    AnalysisName::Authors,
    AnalysisName::TopCommitters,
    AnalysisName::RefactoringTargets,
];

/// The verbatim error for `--format html` on an analysis whose row type is not
/// yet wired through the generic HTML emitter. The covered list is derived from
/// [`HTML_WIRED`] so it cannot go stale as bespoke emitters are added or removed.
fn html_not_wired(analysis_name: &str) -> anyhow::Error {
    let covered = HTML_WIRED
        .iter()
        .map(|a| a.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    CodeLoreError::Analysis(format!(
        "--format html for analysis `{analysis_name}` not yet wired (covered: {covered} — file an issue if you need another)"
    ))
    .into()
}

/// The verbatim error for an output `--format` a given analysis's dispatch
/// doesn't wire. The advertised list is derived from [`supported_formats`] — the
/// one source of truth for what each analysis emits — so the message and the
/// wiring cannot drift, and the `CodeLoreError::Analysis` shape keeps the
/// analysis-failure exit code uniform.
fn unsupported_format(analysis: AnalysisName, fmt: &str) -> anyhow::Error {
    CodeLoreError::Analysis(format!(
        "{} analysis supports {}; got {fmt:?}",
        analysis.as_str(),
        supported_formats(analysis).join("|"),
    ))
    .into()
}

/// Collapse a per-analysis output dispatch into one shape. `run` is the
/// analysis call, spliced into (and so evaluated only inside) the matched arm,
/// so an unsupported `--format` never runs the analysis. Each writer receives
/// `(&rows, out)`: pass a path (`output::csv::write_x_csv`) for the standard
/// arity, or a closure for writers that take an extra argument. `csv`, `json`
/// and `markdown` are wired for every analysis; add a trailing `html` marker to
/// the block to wire the bespoke `write_html` arm (otherwise `--format html`
/// returns the shared [`html_not_wired`] guidance). Extra streaming formats
/// (`sarif`/`ndjson`/`gha`) are listed like any other writer. The
/// unsupported-format fallback reads [`supported_formats`], the one source of
/// truth for the emitted set.
macro_rules! dispatch {
    // Streaming set plus a bespoke HTML emitter (the trailing `html` marker).
    (
        $ctx:expr, $format:expr, $out:expr, $run:expr,
        { $( $fmt:literal => $w:expr ),+ , html $(,)? }
    ) => {{
        match $format {
            $( $fmt => {
                let rows = $run.with_context(|| format!("run {}", $ctx.analysis.as_str()))?;
                let emit = $w;
                emit(&rows, $out).with_context(|| format!("write {}", $fmt))?;
            } )+
            "html" => {
                let rows = $run.with_context(|| format!("run {}", $ctx.analysis.as_str()))?;
                output::html::write_html(&rows, $out, &$ctx.title, &$ctx.repo_root, &$ctx.generated_at)
                    .context("write html")?;
            }
            other => return Err(unsupported_format($ctx.analysis, other)),
        }
    }};
    // Streaming only: `--format html` returns the shared not-wired guidance.
    (
        $ctx:expr, $format:expr, $out:expr, $run:expr,
        { $( $fmt:literal => $w:expr ),+ $(,)? }
    ) => {{
        match $format {
            $( $fmt => {
                let rows = $run.with_context(|| format!("run {}", $ctx.analysis.as_str()))?;
                let emit = $w;
                emit(&rows, $out).with_context(|| format!("write {}", $fmt))?;
            } )+
            "html" => return Err(html_not_wired($ctx.analysis.as_str())),
            other => return Err(unsupported_format($ctx.analysis, other)),
        }
    }};
}

/// Run the requested analysis and stream it to `out` in `format` (the
/// csv/json/sarif/markdown/ndjson/gha/html surfaces). One arm per
/// [`AnalysisName`], each a [`dispatch!`] over its wired format set; the arms
/// that need a repository, a `--target`, or an external findings store keep that
/// setup explicit before the shared core. The `parquet`/`sqlite`/`spa`/
/// `step-summary` surfaces are composites handled by the caller, not here.
#[allow(clippy::too_many_lines)] // one declarative arm per analysis — a dispatch table, linear by construction
fn run_streaming_dispatch(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    cache_dir: Option<&std::path::Path>,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match ctx.analysis {
        AnalysisName::Hotspots => dispatch!(ctx, format, out,
        analyses::hotspots::run_hotspots(db, opts),
        {
            "csv" => output::csv::write_hotspots_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_hotspots_markdown,
            "sarif" => |r, o| output::sarif::write_hotspots_sarif(r, &ctx.repo_root, o),
            "ndjson" => output::ndjson::write_ndjson,
            "gha" => output::gha::write_hotspots_gha,
            html,
        }),
        AnalysisName::HotspotVelocity => dispatch!(ctx, format, out,
        analyses::hotspot_velocity::run_hotspot_velocity(db, opts),
        {
            "csv" => output::csv::write_hotspot_velocity_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_hotspot_velocity_markdown,
        }),
        AnalysisName::CodeHealth => {
            // Analyze has no --quiet; the notice self-suppresses off a TTY.
            notice_corpus_lens_absent(opts, false);
            dispatch!(ctx, format, out,
            analyses::code_health::run_code_health(db, opts),
            {
                "csv" => output::csv::write_code_health_csv,
                "json" => output::json::write_json,
                "markdown" => output::markdown::write_code_health_markdown,
                "ndjson" => output::ndjson::write_ndjson,
                html,
            });
        }
        AnalysisName::CodeAge => dispatch!(ctx, format, out,
        analyses::code_age::run_code_age(db, opts),
        {
            "csv" => |r, o| output::csv::write_code_age_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_code_age_markdown,
        }),
        AnalysisName::AbsChurn => dispatch!(ctx, format, out,
        analyses::churn::run_abs_churn(db, opts),
        {
            "csv" => output::csv::write_abs_churn_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_abs_churn_markdown,
        }),
        AnalysisName::AuthorChurn => dispatch!(ctx, format, out,
        analyses::churn::run_author_churn(db, opts),
        {
            "csv" => output::csv::write_author_churn_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_author_churn_markdown,
        }),
        AnalysisName::EntityChurn => dispatch!(ctx, format, out,
        analyses::churn::run_entity_churn(db, opts),
        {
            "csv" => output::csv::write_entity_churn_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_entity_churn_markdown,
        }),
        AnalysisName::Communication => dispatch!(ctx, format, out,
        analyses::communication::run_communication(db, opts),
        {
            "csv" => |r, o| output::csv::write_communication_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_communication_markdown,
        }),
        AnalysisName::Ownership => dispatch!(ctx, format, out,
        analyses::ownership::run_ownership(db, opts),
        {
            "csv" => |r, o| output::csv::write_ownership_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_ownership_markdown,
        }),
        AnalysisName::Coupling => dispatch!(ctx, format, out,
        analyses::coupling::run_coupling(db, opts),
        {
            "csv" => |r, o| output::csv::write_coupling_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_coupling_markdown,
            "ndjson" => output::ndjson::write_ndjson,
        }),
        AnalysisName::Summary => dispatch!(ctx, format, out,
        analyses::summary::run_summary(db, opts),
        {
            "csv" => |r, o| output::csv::write_summary_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_summary_markdown,
            html,
        }),
        // clones is short-circuited before the repo opens for its streaming
        // formats (a HEAD-only tree-sitter walk needs no history); the arms
        // remain the single source of truth for the wired-format set.
        AnalysisName::Clones => dispatch!(ctx, format, out,
        analyses::clones::run_clones(opts),
        {
            "csv" => output::csv::write_clones_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_clones_markdown,
            "sarif" => |r, o| output::sarif::write_clones_sarif(r, &ctx.repo_root, o),
        }),
        AnalysisName::Revisions => dispatch!(ctx, format, out,
        analyses::revisions::run_revisions(db, opts),
        {
            "csv" => output::csv::write_revisions_csv,
            "json" => output::json::write_revisions_json,
            "markdown" => output::markdown::write_revisions_markdown,
            html,
        }),
        AnalysisName::Authors => dispatch!(ctx, format, out,
        analyses::authors::run_authors(db, opts),
        {
            "csv" => |r, o| output::csv::write_authors_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_authors_markdown,
            html,
        }),
        AnalysisName::TopCommitters => dispatch!(ctx, format, out,
        analyses::top_committers::run_top_committers(db, opts),
        {
            "csv" => output::csv::write_top_committers_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_top_committers_markdown,
            html,
        }),
        AnalysisName::GodClasses => dispatch!(ctx, format, out,
        analyses::god_classes::run_god_classes(db, opts),
        {
            "csv" => output::csv::write_god_classes_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_god_classes_markdown,
        }),
        AnalysisName::ArchViolations => dispatch!(ctx, format, out,
        analyses::arch_violations::run_arch_violations(db, opts),
        {
            "csv" => output::csv::write_arch_violations_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_arch_violations_markdown,
        }),
        AnalysisName::DependencyCycles => dispatch!(ctx, format, out,
        analyses::dependency_cycles::run_dependency_cycles(db, opts),
        {
            "csv" => output::csv::write_dependency_cycles_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_dependency_cycles_markdown,
        }),
        AnalysisName::ArchitectureRoles => dispatch!(ctx, format, out,
        analyses::architecture_roles::run_architecture_roles(db, opts),
        {
            "csv" => output::csv::write_architecture_roles_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_architecture_roles_markdown,
        }),
        AnalysisName::Instability => dispatch!(ctx, format, out,
        analyses::instability::run_instability(db, opts),
        {
            "csv" => output::csv::write_instability_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_instability_markdown,
        }),
        AnalysisName::CycleHealth => dispatch!(ctx, format, out,
        analyses::cycle_health::run_cycle_health(db, opts),
        {
            "csv" => output::csv::write_cycle_health_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_cycle_health_markdown,
        }),
        AnalysisName::DefectValidation => dispatch!(ctx, format, out,
        analyses::defect_validation::run_defect_validation(opts),
        {
            "csv" => output::csv::write_defect_validation_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_defect_validation_markdown,
        }),
        AnalysisName::ArchitectureMetrics => dispatch!(ctx, format, out,
        analyses::architecture_metrics::run_architecture_metrics(db, opts),
        {
            "csv" => output::csv::write_architecture_metrics_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_architecture_metrics_markdown,
        }),
        // The trend analyses read blobs at past revs, so they take the opened
        // `repo` in addition to the fact store.
        AnalysisName::ArchitectureTrend => dispatch!(ctx, format, out,
        analyses::architecture_trend::run_architecture_trend(db, repo, opts),
        {
            "csv" => output::csv::write_architecture_trend_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_architecture_trend_markdown,
        }),
        AnalysisName::HealthTrend => dispatch!(ctx, format, out,
        analyses::health_trend::run_health_trend(db, repo, opts),
        {
            "csv" => output::csv::write_health_trend_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_health_trend_markdown,
        }),
        AnalysisName::CycleOrigins => dispatch!(ctx, format, out,
        analyses::cycle_origins::run_cycle_origins(db, repo, opts),
        {
            "csv" => output::csv::write_cycle_origins_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_cycle_origins_markdown,
        }),
        AnalysisName::ModularityViolations => dispatch!(ctx, format, out,
        analyses::modularity_violations::run_modularity_violations(db, opts),
        {
            "csv" => output::csv::write_modularity_violations_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_modularity_violations_markdown,
        }),
        AnalysisName::UnstableInterface => dispatch!(ctx, format, out,
        analyses::unstable_interface::run_unstable_interface(db, opts),
        {
            "csv" => output::csv::write_unstable_interface_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_unstable_interface_markdown,
        }),
        AnalysisName::Crossing => dispatch!(ctx, format, out,
        analyses::crossing::run_crossing(db, opts),
        {
            "csv" => output::csv::write_crossing_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_crossing_markdown,
        }),
        AnalysisName::StaleCode => dispatch!(ctx, format, out,
        analyses::stale_code::run_stale_code(db, opts),
        {
            "csv" => output::csv::write_stale_code_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_stale_code_markdown,
        }),
        AnalysisName::PairProgramming => dispatch!(ctx, format, out,
        analyses::pair_programming::run_pair_programming(db, opts),
        {
            "csv" => output::csv::write_pair_programming_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_pair_programming_markdown,
        }),
        AnalysisName::LeadTime => dispatch!(ctx, format, out,
        analyses::lead_time::run_lead_time(db, opts),
        {
            "csv" => output::csv::write_lead_time_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_lead_time_markdown,
            "ndjson" => output::ndjson::write_ndjson,
        }),
        AnalysisName::BusFactor => dispatch!(ctx, format, out,
        analyses::bus_factor::run_bus_factor(db, opts),
        {
            "csv" => output::csv::write_bus_factor_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_bus_factor_markdown,
        }),
        AnalysisName::DeliveryFriction => dispatch!(ctx, format, out,
        analyses::delivery_friction::run_delivery_friction(db, opts),
        {
            "csv" => output::csv::write_delivery_friction_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_delivery_friction_markdown,
        }),
        AnalysisName::RefactoringTargets => dispatch!(ctx, format, out,
        analyses::refactoring_targets::run_refactoring_targets(db, opts),
        {
            "csv" => output::csv::write_refactoring_targets_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_refactoring_targets_markdown,
            "ndjson" => output::ndjson::write_ndjson,
            html,
        }),
        AnalysisName::KnowledgeIslands => dispatch!(ctx, format, out,
        analyses::knowledge_islands::run_knowledge_islands(db, opts),
        {
            "csv" => output::csv::write_knowledge_islands_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_knowledge_islands_markdown,
            html,
        }),
        AnalysisName::Soc => dispatch!(ctx, format, out,
        analyses::soc::run_soc(db, opts),
        {
            "csv" => output::csv::write_soc_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_soc_markdown,
        }),
        AnalysisName::Messages => dispatch!(ctx, format, out,
        analyses::messages::run_messages(db, opts),
        {
            "csv" => output::csv::write_messages_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_messages_markdown,
        }),
        AnalysisName::MainDev => dispatch!(ctx, format, out,
        analyses::main_dev::run_main_dev(db, opts),
        {
            "csv" => output::csv::write_main_dev_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_main_dev_markdown,
        }),
        AnalysisName::MainDevByRevs => dispatch!(ctx, format, out,
        analyses::main_dev::run_main_dev_by_revs(db, opts),
        {
            "csv" => |r, o| output::csv::write_main_dev_by_revs_csv(r, o, opts.code_maat_compat),
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_main_dev_by_revs_markdown,
        }),
        AnalysisName::MainDevByDeletions => dispatch!(ctx, format, out,
        analyses::main_dev::run_main_dev_by_deletions(db, opts),
        {
            "csv" => output::csv::write_main_dev_by_deletions_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_main_dev_by_deletions_markdown,
        }),
        AnalysisName::EntityEffort => dispatch!(ctx, format, out,
        analyses::entity_effort::run_entity_effort(db, opts),
        {
            "csv" => output::csv::write_entity_effort_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_entity_effort_markdown,
        }),
        AnalysisName::EntityOwnership => dispatch!(ctx, format, out,
        analyses::entity_ownership::run_entity_ownership(db, opts),
        {
            "csv" => output::csv::write_entity_ownership_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_entity_ownership_markdown,
        }),
        AnalysisName::CloneCoupling => dispatch!(ctx, format, out,
        analyses::clone_coupling::run_clone_coupling(db, opts),
        {
            "csv" => output::csv::write_clone_coupling_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_clone_coupling_markdown,
            "sarif" => |r, o| output::sarif::write_clone_coupling_sarif(r, &ctx.repo_root, o),
            html,
        }),
        AnalysisName::Centrality => dispatch!(ctx, format, out,
        analyses::centrality::run_centrality(db, opts),
        {
            "csv" => output::csv::write_centrality_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_centrality_markdown,
        }),
        AnalysisName::Communities => dispatch!(ctx, format, out,
        analyses::communities::run_communities(db, opts),
        {
            "csv" => output::csv::write_communities_csv,
            "json" => output::json::write_communities_json,
            "markdown" => output::markdown::write_communities_markdown,
        }),
        // The decomposed scan enriches the red band with the improving vs
        // degrading churn split — the differentiated signal — using the repo
        // handle to parse each red file's window-start source (scoped to red
        // files, never a second full scan).
        AnalysisName::EffortExposure => dispatch!(ctx, format, out,
        analyses::effort_exposure::run_effort_exposure_decomposed_scan(db, repo, opts),
        {
            "csv" => output::csv::write_effort_exposure_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_effort_exposure_markdown,
        }),
        AnalysisName::CodeFamiliarity => dispatch!(ctx, format, out,
        analyses::code_familiarity::run_code_familiarity(db, opts),
        {
            "csv" => output::csv::write_code_familiarity_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_code_familiarity_markdown,
        }),
        AnalysisName::TeamComposition => dispatch!(ctx, format, out,
        analyses::team_composition::run_team_composition(db, opts),
        {
            "csv" => output::csv::write_team_composition_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_team_composition_markdown,
        }),
        AnalysisName::CoordinationNeeds => dispatch!(ctx, format, out,
        analyses::coordination_needs::run_coordination_needs(db, opts),
        {
            "csv" => output::csv::write_coordination_needs_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_coordination_needs_markdown,
        }),
        AnalysisName::MarginalOwnerRisk => dispatch!(ctx, format, out,
        analyses::marginal_owner_risk::run_marginal_owner_risk(db, opts),
        {
            "csv" => output::csv::write_marginal_owner_risk_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_marginal_owner_risk_markdown,
        }),
        // release-cadence derives inter-release gaps from git tags, so it takes
        // the repository rather than the fact store.
        AnalysisName::ReleaseCadence => dispatch!(ctx, format, out,
        analyses::release_cadence::run_release_cadence(repo, opts),
        {
            "csv" => output::csv::write_release_cadence_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_release_cadence_markdown,
        }),
        AnalysisName::DeliveryMetrics => dispatch!(ctx, format, out,
        analyses::delivery_metrics::run_delivery_metrics(db, opts),
        {
            "csv" => output::csv::write_delivery_metrics_csv,
            "json" => output::json::write_json,
            "markdown" => output::markdown::write_delivery_metrics_markdown,
        }),
        AnalysisName::FunctionXray => {
            let target = opts.target.as_deref().ok_or_else(|| {
                CodeLoreError::Analysis("--target <path> is required for function-xray".to_string())
            })?;
            dispatch!(ctx, format, out,
            analyses::function_xray::run_function_xray(db, repo, opts, target),
            {
                "csv" => output::csv::write_function_xray_csv,
                "json" => output::json::write_json,
                "markdown" => |r, o| output::markdown::write_function_xray_markdown(r, target, o),
            });
        }
        AnalysisName::FunctionCoupling => {
            let target = opts.target.as_deref().ok_or_else(|| {
                CodeLoreError::Analysis(
                    "--target <path> is required for function-coupling".to_string(),
                )
            })?;
            dispatch!(ctx, format, out,
            analyses::function_coupling::run_function_coupling(db, repo, opts, target),
            {
                "csv" => output::csv::write_function_coupling_csv,
                "json" => output::json::write_json,
                "markdown" => |r, o| output::markdown::write_function_coupling_markdown(r, target, o),
            });
        }
        AnalysisName::FindingHotspotOverlap => {
            // Requires the external sidecar: open it read-only from the cache
            // dir. open_nonempty returns None when the sidecar is absent OR
            // present-but-empty — both mean "no findings ingested yet", and
            // both surface the same pre-condition error here. We handle the
            // None case at this layer so we never create an empty sidecar as
            // a side-effect of an analysis read.
            let cache_root = cache_dir.map_or_else(
                codelore_lib::cli_api::cache::default_cache_root,
                std::path::Path::to_path_buf,
            );
            let store = codelore_lib::cli_api::external::ExternalStore::open_nonempty(
                &cache_root,
                &opts.repo_path,
            )
            .context("open external findings store")?
            .ok_or_else(|| {
                CodeLoreError::Analysis(
                    "finding-hotspot-overlap requires prior `codelore ingest-sarif` \
                     (no external findings found)"
                        .to_string(),
                )
            })?;
            dispatch!(ctx, format, out,
            analyses::finding_hotspot_overlap::run_finding_hotspot_overlap(db, opts, &store),
            {
                "csv" => output::csv::write_finding_hotspot_overlap_csv,
                "json" => output::json::write_json,
                "markdown" => output::markdown::write_finding_hotspot_overlap_markdown,
            });
        }
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
    use codelore_lib::cli_api::output::banner::{self, Banner, Preflight};
    use codelore_lib::cli_api::provenance::{DUCKDB_VERSION, GIX_VERSION};

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
    let manifest = codelore_lib::cli_api::provenance::Manifest::capture(db, opts, analysis_name)
        .context("capture provenance manifest")?;
    let json = manifest
        .to_json()
        .context("serialize provenance manifest")?;
    let sidecar = std::path::PathBuf::from(format!("{}.provenance.json", output_path.display()));

    // Atomic-rename pattern: write the sidecar payload to a temp file
    // first, fsync it, then atomically rename into place. Closes the
    // crash window where a half-written .provenance.json on disk would
    // be indistinguishable from a complete one. Downstream consumers
    // (SLSA verifiers, CI gates) can rely on "if the file exists,
    // its contents are complete and durable".
    //
    // The pid suffix lets concurrent codelore runs against the same
    // output path stay isolated (the last writer wins, and intermediate
    // writers don't trample each other's tmp files mid-write).
    //
    // Caveat: this only makes the SIDECAR atomic — the main output
    // (parquet, CSV, etc.) was already dropped by the caller before
    // we got here. Power-loss between the BufWriter drop and the
    // rename below can still leave a main output without a sidecar.
    // That's the residual gap; closing it would require restructuring
    // the output emitters to expose a sync_all hook on the main
    // handle, which is out of scope for the sidecar atomicity fix.
    let mut tmp_name = sidecar.as_os_str().to_owned();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp_path = std::path::PathBuf::from(tmp_name);

    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp_path).with_context(|| {
            format!(
                "create provenance sidecar tmp file at {}",
                tmp_path.display()
            )
        })?;
        f.write_all(json.as_bytes()).with_context(|| {
            format!("write provenance sidecar payload to {}", tmp_path.display())
        })?;
        f.sync_all().with_context(|| {
            format!(
                "fsync provenance sidecar tmp file at {}",
                tmp_path.display()
            )
        })?;
    } // File dropped → OS handle closed before rename.

    std::fs::rename(&tmp_path, &sidecar).with_context(|| {
        // On rename failure, clean up the orphan tmp file best-effort.
        // We can't surface a secondary error here because we're already
        // returning the rename's error; the orphan would be left for
        // the next process-id collision to clobber.
        let _ = std::fs::remove_file(&tmp_path);
        format!(
            "atomically rename {} -> {}",
            tmp_path.display(),
            sidecar.display()
        )
    })?;
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
            codelore_lib::cli_api::output::parquet::write_hotspots_parquet(db, opts, path)
                .context("write parquet")
        }
        AnalysisName::Revisions => {
            codelore_lib::cli_api::output::parquet::write_revisions_parquet(db, opts, path)
                .context("write parquet")
        }
        AnalysisName::Summary => {
            codelore_lib::cli_api::output::parquet::write_summary_parquet(db, opts, path)
                .context("write parquet")
        }
        other => anyhow::bail!(
            "--format parquet currently supports hotspots, revisions, summary only \
             (Plan 5 scope); got {other:?}"
        ),
    }
}

/// Run every analysis the dashboard consumes and assemble a
/// `SpaDashboard`. Shared by both `--format spa` (HTML emitter) and
/// `--format step-summary` (GFM markdown emitter) — they render the
/// same data into different output shapes.
#[cfg(feature = "spa")]
#[allow(clippy::too_many_lines)] // one run-each-analysis-then-assemble sequence
fn build_spa_dashboard(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    repo_path: &std::path::Path,
) -> anyhow::Result<codelore_lib::cli_api::output::spa::SpaDashboard> {
    use codelore_lib::cli_api::analyses::dashboard::{
        run_clone_summary, run_daily_commits, run_trends, run_xray,
    };
    use codelore_lib::cli_api::output::spa::SpaDashboard;

    // Entity-ownership embed cap: retain ownership rows only for the top-N
    // hotspot files (the displayable set). See the `entity_ownership` block.
    const SPA_OWNERSHIP_FILE_CAP: usize = 200;
    // Refactoring-targets embed cap: the guided tour brushes the top-10; a
    // generous head of 20 leaves room without bloating the payload.
    const SPA_REFACTORING_TARGET_CAP: usize = 20;

    // Each run_* call is an SQL query over the already-ingested fact
    // store, so the composite cost is bounded by the query mix
    // (typically <1s on mid-size repos). Optional widgets degrade
    // gracefully (warn + empty vec) on tiny fixtures or when the
    // analysis can't run.
    let hotspots = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
        .context("run hotspots for dashboard")?;
    let summary = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
        .context("run summary for dashboard")?;
    let code_health = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
        .context("run code-health for dashboard")?;
    let coupling = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: coupling analysis failed; skipping: {e}");
            Vec::new()
        });
    let knowledge_islands =
        codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: knowledge-islands analysis failed; skipping: {e}");
                Vec::new()
            });
    // Entity-ownership feeds the knowledge-map lens, the off-boarding picker,
    // and the drawer's contributor list — all keyed on hotspot paths (the
    // only files the circle-pack colours and the drawer opens). It is the
    // largest embedded field (O(files × authors)); cap it to the ownership
    // rows for the top-N hotspot paths so the HTML stays bounded on big repos.
    // Rows for non-hotspot files are never displayed, so dropping them loses
    // nothing on screen — a truncation note fires only when a *hotspot* file's
    // ownership is dropped. `with_no_row_limit` so the cap is the
    // deterministic top-N by hotspot rank, independent of the user's `--rows`
    // (the raw analysis would otherwise truncate alphabetically).
    let (entity_ownership, entity_ownership_cap) = {
        let full = codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(
            db,
            &opts.with_no_row_limit(),
        )
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: entity-ownership analysis failed; skipping: {e}");
            Vec::new()
        });
        // Hotspots are sorted by score DESC, so the first N paths are the
        // displayable head; ownership for anything past it can't be shown.
        let hotspot_paths: std::collections::HashSet<&str> =
            hotspots.iter().map(|h| h.path.as_str()).collect();
        let capped_paths: std::collections::HashSet<&str> = hotspots
            .iter()
            .take(SPA_OWNERSHIP_FILE_CAP)
            .map(|h| h.path.as_str())
            .collect();
        let mut dropped_displayable = false;
        let kept: Vec<_> = full
            .into_iter()
            .filter(|r| {
                let keep = capped_paths.contains(r.entity.as_str());
                if !keep && hotspot_paths.contains(r.entity.as_str()) {
                    dropped_displayable = true;
                }
                keep
            })
            .collect();
        let cap =
            dropped_displayable.then(|| u32::try_from(SPA_OWNERSHIP_FILE_CAP).unwrap_or(u32::MAX));
        (kept, cap)
    };
    let xray = run_xray(db, 500).unwrap_or_else(|e| {
        tracing::warn!("dashboard: xray query failed; skipping: {e}");
        Vec::new()
    });
    let daily_commits = run_daily_commits(db).unwrap_or_else(|e| {
        tracing::warn!("dashboard: daily_commits query failed; skipping: {e}");
        Vec::new()
    });
    // Trends — restrict to the top-50 hotspot paths. The SPA's
    // Top-N selector defaults to 10 (the historical view) but lets
    // the user widen up to all 50 without a re-run. Frontend
    // slices `data.trends` reactively when the user changes the
    // selector — no backend round-trip needed.
    let top_paths: Vec<String> = hotspots.iter().take(50).map(|r| r.path.clone()).collect();
    let trends = run_trends(db, opts, &top_paths).unwrap_or_else(|e| {
        tracing::warn!("dashboard: trends query failed; skipping: {e}");
        Vec::new()
    });
    let mi_rollup = Some(codelore_lib::cli_api::analyses::mi::MiRollup::from_hotspots(&hotspots));
    let coupling_density = compute_spa_coupling_density(db, opts, &coupling);
    let clones = run_clone_summary(db).unwrap_or_else(|e| {
        tracing::warn!("dashboard: clone summary query failed; skipping: {e}");
        Vec::new()
    });
    // Kamei JIT-SDP feature vector — pulled for the last-100
    // non-merge commits so the SPA's window selector can show 10,
    // 30, 60, or all-100 without a re-run. Default render is the
    // last 30 (historical view); the frontend slices reactively.
    let kamei_risk = codelore_lib::cli_api::analyses::dashboard::run_kamei_risk(db, 100)
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: kamei risk query failed; skipping: {e}");
            Vec::new()
        });
    // Resolved import edges for the architecture force-graph widget.
    // Empty when the resolver hasn't covered the repo's language mix
    // yet.
    let imports = codelore_lib::cli_api::analyses::dashboard::run_imports_for_arch_graph(db)
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: imports query failed; skipping: {e}");
            Vec::new()
        });
    // Structure×history fusion overlaid on the architecture graph.
    let modularity_violations =
        codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: modularity-violations failed; skipping: {e}");
                Vec::new()
            });
    let unstable_interface =
        codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: unstable-interface failed; skipping: {e}");
                Vec::new()
            });
    // Full per-file roles (no row cap) so the graph colours every module
    // and the propagation-cost caption sums over the whole graph.
    let architecture_roles =
        codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
            db,
            &opts.with_no_row_limit(),
        )
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: architecture-roles failed; skipping: {e}");
            Vec::new()
        });
    // Open the repo ONCE for the three history/HEAD scans below —
    // sample-trends, release-cadence, and function-xray. All three take an
    // immutable handle and read only, so a single open (instead of one per
    // scan) avoids re-parsing the repo config three times. A failed open
    // degrades every dependent widget to empty, exactly as the per-scan opens
    // did before.
    let spa_repo = match codelore_lib::cli_api::repo::GixRepo::open(repo_path) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(
                "dashboard: could not open repo for history/xray scans; skipping those widgets: {e}"
            );
            None
        }
    };
    // Architecture-trend and health-trend both re-read source at the same
    // sampled historical revs and each rebuild the identical per-rev import
    // graph. `run_sample_trends` builds each graph ONCE and derives both views
    // from it, so a dashboard that shows both pays that historical scan once,
    // not twice. Any failure leaves both trend views empty rather than sinking
    // the whole dashboard.
    let (architecture_trend, health_trend, file_health_series, health_transitions) = spa_repo
        .as_ref()
        .and_then(|repo| {
            match codelore_lib::cli_api::analyses::health_trend::run_sample_trends(db, repo, opts) {
                Ok(t) => Some((
                    t.architecture,
                    t.health.trend,
                    t.health.file_series,
                    t.health.transitions,
                )),
                Err(e) => {
                    tracing::warn!("dashboard: trend scan failed; skipping: {e}");
                    None
                }
            }
        })
        .unwrap_or_else(|| (Vec::new(), Vec::new(), Vec::new(), Vec::new()));
    // Effort-exposure: LOC/commit/churn share per band over the trailing
    // window. Runs the same code-health HEAD scan as the factor header (cheap,
    // cached). With the shared repo handle the red band also carries the
    // improving vs degrading churn split (scoped to red files); without it the
    // base SQL-only rows are used. Degrades to empty on analysis failure so no
    // widget is shown.
    let effort_exposure = {
        use codelore_lib::cli_api::analyses::effort_exposure::{
            run_effort_exposure, run_effort_exposure_decomposed_scan,
        };
        let result = match &spa_repo {
            Some(r) => run_effort_exposure_decomposed_scan(db, r, opts),
            None => run_effort_exposure(db, opts),
        };
        result.unwrap_or_else(|e| {
            tracing::warn!("dashboard: effort-exposure analysis failed; skipping: {e}");
            Vec::new()
        })
    };
    // Marginal-owner risk: ownership concentration × code-health fusion.
    // Degrades to empty when no file meets the high/elevated threshold,
    // or when knowledge_shares is unavailable (e.g. tiny fixture repos).
    let marginal_owner_risk =
        codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: marginal-owner-risk analysis failed; skipping: {e}");
                Vec::new()
            });
    // Delivery-metrics percentile distributions.  Requires include_merges;
    // degrades gracefully to empty so the tile and card are omitted.
    let delivery_metrics = {
        let mut dm_opts = opts.clone();
        dm_opts.include_merges = true;
        codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, &dm_opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: delivery-metrics failed; skipping: {e}");
                Vec::new()
            })
    };
    // Release cadence — reuses the shared `spa_repo` handle.
    let release_cadence = spa_repo
        .as_ref()
        .and_then(
            |repo| match codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(
                repo, opts,
            ) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("dashboard: release-cadence failed; skipping: {e}");
                    None
                }
            },
        )
        .unwrap_or_default();
    // Delivery-friction — top 5 rows for the "where is friction" drill line.
    let delivery_friction =
        codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: delivery-friction failed; skipping: {e}");
                Vec::new()
            })
            .into_iter()
            .take(5)
            .collect::<Vec<_>>();
    // Per-file function X-Ray for the top-10 hotspot paths. Each call reads
    // the HEAD blob via the shared `spa_repo` handle (cheap; tree-sitter spans
    // only) and joins against already-ingested hunks. One failure per path
    // degrades gracefully to an empty rows vec for that path.
    let function_xray: Vec<codelore_lib::cli_api::output::spa::FileFunctionXray> = spa_repo
        .as_ref()
        .map(|xray_repo| {
            hotspots
                .iter()
                .take(10)
                .filter_map(|h| {
                    match codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                        db, xray_repo, opts, &h.path,
                    ) {
                        Ok(rows) if !rows.is_empty() => {
                            Some(codelore_lib::cli_api::output::spa::FileFunctionXray {
                                path: h.path.clone(),
                                rows,
                            })
                        }
                        Ok(_) => None,
                        Err(e) => {
                            tracing::debug!("dashboard: function-xray skipped for {}: {e}", h.path);
                            None
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // Refactoring targets — return-on-investment ranking (risk ÷ effort),
    // capped to a small head. The guided tour brushes the top-10 across every
    // widget. `with_no_row_limit` so the ranking is the true top-N by
    // priority, independent of the user's `--rows`. Degrades to empty when the
    // code-health composite is unavailable (the tour then falls back to the
    // hotspot proxy).
    let refactoring_targets =
        codelore_lib::cli_api::analyses::refactoring_targets::run_refactoring_targets(
            db,
            &opts.with_no_row_limit(),
        )
        .map_or_else(
            |e| {
                tracing::warn!("dashboard: refactoring-targets failed; skipping: {e}");
                Vec::new()
            },
            |mut rows| {
                rows.truncate(SPA_REFACTORING_TARGET_CAP);
                rows
            },
        );
    // Four-factor header tiles assembled from already-computed data.
    // Code + Architecture come from the health_trend series (zero extra
    // cost — the series is already in memory). Knowledge uses
    // code_familiarity when available, falling back to knowledge_islands.
    // Delivery uses delivery_metrics + release_cadence (degrades to no tile).
    let mut factors = codelore_lib::cli_api::analyses::factors::health_trend_factors(&health_trend);
    // Corpus-relative annotation on the Architecture tile: when an active
    // calibration artifact carries repo-level corpus pools, the
    // architecture-metrics rows include `corpus_percentile:propagation_cost`
    // and `corpus_n` — surface them on the tile's detail line. No active
    // artifact, no `repo_metrics` section, or no Architecture tile ⇒ the
    // detail line is unchanged.
    if let Some(arch_tile) = factors.iter_mut().find(|t| t.name == "Architecture") {
        let arch_metric_rows =
            codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                db, opts,
            )
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: architecture-metrics for factor tile failed: {e}");
                Vec::new()
            });
        let metric_value = |name: &str| {
            arch_metric_rows
                .iter()
                .find(|r| r.metric == name)
                .map(|r| r.value.as_str())
        };
        if let (Some(p), Some(n)) = (
            metric_value("corpus_percentile:propagation_cost"),
            metric_value("corpus_n"),
        ) && let Ok(p) = p.parse::<f64>()
        {
            use std::fmt::Write as _;
            // The row's percentile is 0..1; the tile shows the conventional
            // 0..100 "P<nn>" reading.
            let _ = write!(arch_tile.detail, ", P{:.0} of {n} corpus repos", p * 100.0);
        }
    }
    // Knowledge card data — computed once, feeding both the factor tile and
    // the SPA payload. Degrades to empty on failure so the card is simply
    // absent when data is unavailable.
    let code_familiarity =
        codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: code-familiarity for spa failed; skipping: {e}");
                Vec::new()
            });
    let knowledge_tile = code_familiarity
        .first()
        .map(|r| {
            codelore_lib::cli_api::analyses::factors::knowledge_factor_from_familiarity(
                r.familiarity_pct,
                r.islands_pct,
            )
        })
        .or_else(|| {
            // Prevalence denominator: all files live at HEAD (no `--min-revs`
            // gate). Degrades to a zero count on query failure, which the
            // factor constructor treats as "no denominator" → tile omitted.
            let total_live_files =
                codelore_lib::cli_api::analyses::knowledge_islands::count_live_files(db)
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            "dashboard: live-file count for knowledge tile failed; skipping: {e}"
                        );
                        0
                    });
            codelore_lib::cli_api::analyses::factors::knowledge_factor_from_islands(
                &knowledge_islands,
                total_live_files,
            )
        });
    if let Some(kt) = knowledge_tile {
        factors.push(kt);
    }
    if let Some(dt) = codelore_lib::cli_api::analyses::factors::delivery_factor_from_metrics(
        &delivery_metrics,
        &release_cadence,
    ) {
        factors.push(dt);
    }
    // Trim the SPA payload to what its consumers read: the delivery card
    // renders three of the five metrics, and only the cadence summary row
    // is consumed (per-tag rows are standalone-CLI output). The factor
    // tile above already read the full row sets.
    let delivery_metrics: Vec<_> = delivery_metrics
        .into_iter()
        .filter(|r| {
            matches!(
                r.metric.as_str(),
                "rework_pct" | "branch_duration_hours" | "lead_proxy_hours"
            )
        })
        .collect();
    let release_cadence: Vec<_> = release_cadence
        .into_iter()
        .filter(|r| r.tag == "__summary__")
        .collect();
    let team_composition =
        codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: team-composition for spa failed; skipping: {e}");
                Vec::new()
            });
    // Coordination-needs: top 10 by tier desc then co-change entropy desc.
    // Tier order: high > medium > low > single (alphabetical inverse = correct
    // only by accident; sort explicitly).
    let coordination_needs = {
        let tier_rank = |t: &str| match t {
            "high" => 3u8,
            "medium" => 2,
            "low" => 1,
            _ => 0, // "single"
        };
        let mut cn =
            codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(db, opts)
                .unwrap_or_else(|e| {
                    tracing::warn!("dashboard: coordination-needs for spa failed; skipping: {e}");
                    Vec::new()
                });
        cn.sort_by(|a, b| {
            tier_rank(&b.tier).cmp(&tier_rank(&a.tier)).then(
                b.cochange_entropy
                    .partial_cmp(&a.cochange_entropy)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        cn.truncate(10);
        cn
    };
    Ok(SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        entity_ownership,
        entity_ownership_cap,
        xray,
        daily_commits,
        trends,
        mi_rollup,
        coupling_density,
        clones,
        kamei_risk,
        imports,
        modularity_violations,
        unstable_interface,
        architecture_roles,
        architecture_trend,
        health_trend,
        file_health_series,
        health_transitions,
        effort_exposure,
        marginal_owner_risk,
        code_familiarity,
        team_composition,
        coordination_needs,
        delivery_metrics,
        release_cadence,
        delivery_friction,
        function_xray,
        refactoring_targets,
        factors,
        options: codelore_lib::cli_api::output::spa::SpaOptionsSnapshot::from_options(opts),
    })
}

#[cfg(feature = "spa")]
fn run_spa_dispatch(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    repo_path: &std::path::Path,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    use codelore_lib::cli_api::output::spa::write_spa;

    let dash = build_spa_dashboard(db, opts, repo_path)?;

    let now = time::OffsetDateTime::now_utc();
    let generated_at = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    );
    let title = "CodeLore Dashboard";
    let repo_display = repo_path.display().to_string();

    // Atomic publish: an interrupted or failing write never truncates a
    // previous good dashboard, and the file is renamed into place (so the
    // stat() below sees the final size) before this returns.
    codelore_lib::cli_api::output::atomic_publish(output, |tmp| {
        let mut out = std::fs::File::create(tmp)
            .with_context(|| format!("create spa output {}", output.display()))?;
        write_spa(&dash, title, &repo_display, &generated_at, &mut out)
            .context("write spa dashboard")
    })?;

    // User feedback: --format spa is silent on success by default,
    // which makes the dashboard look like nothing happened. Print
    // the output path, size, and a clickable file:// URL for
    // terminals that linkify (iTerm2, modern macOS Terminal, most
    // Linux terminals). Single eprintln to stderr so it doesn't
    // pollute stdout when callers pipe.
    let size_bytes = std::fs::metadata(output).map_or(0, |m| m.len());
    #[allow(clippy::cast_precision_loss)]
    // size_bytes is formatted to one decimal place; display-precision loss on u64→f64 is imperceptible and intentional
    let size_human = if size_bytes >= 1_000_000 {
        format!("{:.1} MB", size_bytes as f64 / 1_000_000.0)
    } else if size_bytes >= 1_000 {
        format!("{:.1} kB", size_bytes as f64 / 1_000.0)
    } else {
        format!("{size_bytes} bytes")
    };
    let abs = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    eprintln!(
        "✓ spa dashboard written to {} ({})",
        output.display(),
        size_human
    );
    eprintln!("  open in browser: file://{}", abs.display());
    Ok(())
}

#[cfg(feature = "spa")]
fn run_step_summary_dispatch(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    repo_path: &std::path::Path,
    output: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    use codelore_lib::cli_api::output::step_summary::write_step_summary;

    let dash = build_spa_dashboard(db, opts, repo_path)?;
    let now = time::OffsetDateTime::now_utc();
    let generated_at = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    );
    let title = "CodeLore Analysis";
    let repo_display = repo_path.display().to_string();

    // step-summary streams to stdout by default so CI workflows can
    // `codelore ... --format step-summary >> $GITHUB_STEP_SUMMARY`
    // directly. `--output PATH` opt-in for local use / testing.
    if let Some(path) = output {
        // Atomic publish so an interrupted write never truncates a previous
        // good summary file (the stdout path streams directly, below).
        codelore_lib::cli_api::output::atomic_publish(path, |tmp| {
            let mut out = std::fs::File::create(tmp)
                .with_context(|| format!("create step-summary output {}", path.display()))?;
            write_step_summary(&dash, title, &repo_display, &generated_at, &mut out)
                .context("write step-summary")
        })?;
        eprintln!("✓ step-summary written to {}", path.display());
    } else {
        let mut out = std::io::stdout().lock();
        write_step_summary(&dash, title, &repo_display, &generated_at, &mut out)
            .context("write step-summary")?;
    }
    Ok(())
}

/// Compute the behavioral coupling graph density for the SPA dashboard.
/// `coupling` is the Fisher-significant edge set already returned by
/// `run_coupling`. Skips the node-count query when the edge set is empty
/// (density would be `0` regardless) and converts node-count failures
/// into a warning + `None` so dashboard generation never blocks on a
/// secondary metric.
#[cfg(feature = "spa")]
fn compute_spa_coupling_density(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    coupling: &[codelore_lib::cli_api::analyses::coupling::CouplingRow],
) -> Option<f64> {
    if coupling.is_empty() {
        return None;
    }
    match codelore_lib::cli_api::analyses::coupling::count_coupling_nodes(db, opts) {
        Ok(n) => Some(codelore_lib::cli_api::analyses::coupling::density(
            n,
            coupling.len(),
        )),
        Err(e) => {
            tracing::warn!("spa: coupling-density node count failed; skipping: {e}");
            None
        }
    }
}

/// Registration-surface exhaustiveness.
///
/// An analysis is registered on several independent output surfaces, and only
/// one of them is guarded by the compiler:
///
/// 1. **dispatch** — the `match ctx.analysis` in [`run_streaming_dispatch`]
///    that routes each [`AnalysisName`] to its `dispatch!` arm. This IS
///    compiler-exhaustive: a new variant fails to compile until it has an arm.
/// 2. **csv** / 3. **markdown** — the `match format` inside the `dispatch!`
///    macro decides which `--format` values reach a real emitter, and each
///    arm's wired set is declared in [`supported_formats`], which the dispatch
///    READS (its unsupported-format fallback is derived from it). Declaring the
///    set is still a separate act from wiring the arm, so the tests below hold
///    each analysis to account.
/// 4. **spa** — [`build_spa_dashboard`] hand-picks which analyses contribute
///    rendered data to `--format spa`; it is a composite, not a per-name
///    dispatch, so it too drifts independently.
///
/// This drift is not hypothetical: an analysis wired for dispatch but forgotten
/// on a rendering surface ships silently — no compile error, no test failure.
/// The contract enforced here: **every [`AnalysisName`] must be EITHER wired on
/// a surface OR listed in that surface's documented-absence registry with an
/// honest one-line reason.** A new analysis that accounts for neither fails a
/// test that names it and the surface.
///
/// # Mechanism per surface (why each is chosen)
///
/// [`supported_formats`] is a module-level function the dispatch depends on, so
/// it DRIVES the wired-format set rather than mirroring it: the
/// unsupported-format error is derived from it. [`registration_surfaces::renders_in_spa`]
/// stays test-only — it mirrors the hand-picked `run_*` calls in
/// [`build_spa_dashboard`], which map to no single queryable seam. Both are
/// **exhaustive `match`es over `AnalysisName`**, so a new variant fails to
/// compile until its wiring is declared; the tests then hold each declaration
/// to account against `AnalysisName::all()` and the documented-absence lists.
///
/// A **runtime probe** was rejected: the dispatch runs the analysis inside the
/// matched arm, and several analyses carry preconditions an empty
/// in-memory db can't satisfy (`--target` for function-xray/-coupling, a
/// SARIF sidecar for finding-hotspot-overlap, historical blob reads for the
/// trend analyses), so a probe would be neither uniform nor side-effect-free.
/// A **source-scan** (`include_str!` + grep of the match arms) was rejected as
/// brittle. The seams live in this file, beside the dispatch match and
/// `build_spa_dashboard`, so they stay truthful by colocation; the csv/markdown
/// emitters they name in turn live in `output/{csv,markdown}.rs` and are reached
/// only through these arms (the compiler proves the named `write_*` exists).
#[cfg(test)]
mod registration_surfaces {
    use codelore_lib::cli_api::AnalysisName;

    use super::{HTML_WIRED, supported_formats};

    /// Whether the analysis contributes rendered data to the `--format spa`
    /// dashboard composite. Exhaustive over `AnalysisName`: a new variant will
    /// not compile until it is placed in one of the groups. Mirrors the `run_*`
    /// calls inside `build_spa_dashboard` — a reviewed judgment, not a
    /// mechanical mapping, because several SPA widgets are fed by dashboard-only
    /// composite queries (`run_xray`, `run_trends`, `run_daily_commits`,
    /// `run_kamei_risk`, `run_imports_for_arch_graph`) that map to no
    /// `AnalysisName`, while `architecture-metrics` contributes only the
    /// corpus-percentile annotation on the Architecture factor tile.
    fn renders_in_spa(name: AnalysisName) -> bool {
        use AnalysisName::{
            AbsChurn, ArchViolations, ArchitectureMetrics, ArchitectureRoles, ArchitectureTrend,
            AuthorChurn, Authors, BusFactor, Centrality, CloneCoupling, Clones, CodeAge,
            CodeFamiliarity, CodeHealth, Communication, Communities, CoordinationNeeds, Coupling,
            Crossing, CycleHealth, CycleOrigins, DefectValidation, DeliveryFriction,
            DeliveryMetrics, DependencyCycles, EffortExposure, EntityChurn, EntityEffort,
            EntityOwnership, FindingHotspotOverlap, FunctionCoupling, FunctionXray, GodClasses,
            HealthTrend, HotspotVelocity, Hotspots, Instability, KnowledgeIslands, LeadTime,
            MainDev, MainDevByDeletions, MainDevByRevs, MarginalOwnerRisk, Messages,
            ModularityViolations, Ownership, PairProgramming, RefactoringTargets, ReleaseCadence,
            Revisions, Soc, StaleCode, Summary, TeamComposition, TopCommitters, UnstableInterface,
        };
        match name {
            Hotspots | Coupling | CodeHealth | Summary | Clones | EntityOwnership
            | KnowledgeIslands | ArchitectureRoles | ArchitectureMetrics | ArchitectureTrend
            | HealthTrend | ModularityViolations | UnstableInterface | DeliveryFriction
            | DeliveryMetrics | RefactoringTargets | EffortExposure | CodeFamiliarity
            | TeamComposition | CoordinationNeeds | MarginalOwnerRisk | ReleaseCadence
            | FunctionXray => true,
            HotspotVelocity
            | Ownership
            | CodeAge
            | AbsChurn
            | AuthorChurn
            | EntityChurn
            | Communication
            | Revisions
            | Authors
            | CloneCoupling
            | Soc
            | Messages
            | MainDev
            | MainDevByRevs
            | MainDevByDeletions
            | EntityEffort
            | TopCommitters
            | Centrality
            | Communities
            | GodClasses
            | ArchViolations
            | DependencyCycles
            | Instability
            | CycleOrigins
            | Crossing
            | StaleCode
            | PairProgramming
            | LeadTime
            | BusFactor
            | FunctionCoupling
            | FindingHotspotOverlap
            | CycleHealth
            | DefectValidation => false,
        }
    }

    /// Analyses with no CSV emitter, each with the reason. Empty today — every
    /// analysis streams CSV. A future analysis that opts out must be listed
    /// here (with a reason) or the CSV surface test fails naming it.
    const DOCUMENTED_ABSENT_CSV: &[(AnalysisName, &str)] = &[];

    /// Analyses with no Markdown emitter, each with the reason. Empty today —
    /// every analysis streams Markdown.
    const DOCUMENTED_ABSENT_MARKDOWN: &[(AnalysisName, &str)] = &[];

    /// Analyses that render NO widget/data in the `--format spa` dashboard,
    /// each with an honest one-line reason. The counterpart of the `true`
    /// group in [`renders_in_spa`]: the two must partition `AnalysisName::all()`
    /// exactly, so adding an analysis to one forces removing it from the other.
    const DOCUMENTED_ABSENT_SPA: &[(AnalysisName, &str)] = &[
        (
            AnalysisName::HotspotVelocity,
            "no widget; change-acceleration is a CLI early-warning table",
        ),
        (
            AnalysisName::Ownership,
            "SPA renders ownership via entity-ownership; the repo fractal has no widget",
        ),
        (AnalysisName::CodeAge, "no code-age widget"),
        (
            AnalysisName::AbsChurn,
            "no widget; absolute churn is summarised in the KPI tiles",
        ),
        (
            AnalysisName::AuthorChurn,
            "no widget; per-author churn is a CLI table",
        ),
        (
            AnalysisName::EntityChurn,
            "no widget; per-file churn is subsumed by the hotspots widget",
        ),
        (
            AnalysisName::Communication,
            "no widget; the author-communication graph is CLI-only",
        ),
        (
            AnalysisName::Revisions,
            "no widget; per-file revision counts are subsumed by the hotspots widget",
        ),
        (
            AnalysisName::Authors,
            "no widget; the per-file author-risk table is CLI-only",
        ),
        (
            AnalysisName::CloneCoupling,
            "no widget; the live-clone x co-change intersection is CLI-only",
        ),
        (
            AnalysisName::Soc,
            "no widget; sum-of-coupling is a scalar CLI metric",
        ),
        (
            AnalysisName::Messages,
            "no widget; commit-message matching is CLI-only",
        ),
        (
            AnalysisName::MainDev,
            "no widget; main-developer tables are CLI-only",
        ),
        (
            AnalysisName::MainDevByRevs,
            "no widget; main-developer (by revisions) is CLI-only",
        ),
        (
            AnalysisName::MainDevByDeletions,
            "no widget; main-developer (by deletions) is CLI-only",
        ),
        (
            AnalysisName::EntityEffort,
            "no widget; per-(entity, author) effort rows are CLI-only",
        ),
        (
            AnalysisName::TopCommitters,
            "no widget; the committer leaderboard is CLI-only",
        ),
        (
            AnalysisName::Centrality,
            "no widget; coupling-graph centrality is CLI-only",
        ),
        (
            AnalysisName::Communities,
            "no widget; Leiden community detection is CLI-only",
        ),
        (
            AnalysisName::GodClasses,
            "no widget; god-class detection is CLI-only",
        ),
        (
            AnalysisName::ArchViolations,
            "no widget; layer-rule validation is opt-in CLI-only",
        ),
        (
            AnalysisName::DependencyCycles,
            "no widget; the architecture graph rings cycle members via architecture-roles",
        ),
        (
            AnalysisName::Instability,
            "no widget; Martin instability is CLI-only",
        ),
        (
            AnalysisName::CycleOrigins,
            "no widget; cycle-origin bisection is CLI-only",
        ),
        (
            AnalysisName::Crossing,
            "no widget; the DV8 crossing pattern is CLI-only",
        ),
        (
            AnalysisName::StaleCode,
            "no widget; stale-code surfacing is CLI-only",
        ),
        (
            AnalysisName::PairProgramming,
            "no widget; co-author pairing is CLI-only",
        ),
        (
            AnalysisName::LeadTime,
            "no widget; the delivery card uses delivery-metrics and release-cadence",
        ),
        (
            AnalysisName::BusFactor,
            "no widget; per-module bus factor is CLI-only",
        ),
        (
            AnalysisName::FunctionCoupling,
            "no widget; per-function-pair co-change needs a --target file",
        ),
        (
            AnalysisName::FindingHotspotOverlap,
            "no widget; external-findings fusion needs prior ingest-sarif",
        ),
        (
            AnalysisName::CycleHealth,
            "no widget; per-SCC heat/verdict is CLI-only",
        ),
        (
            AnalysisName::DefectValidation,
            "no widget; defect-calibration evidence is a diagnostic CLI dump",
        ),
    ];

    /// Assert that `handled` (the structural seam) and `absent` (the reviewed
    /// registry) partition `AnalysisName::all()` exactly for one surface:
    /// every analysis is accounted for on exactly one side. Failures name the
    /// offending analysis (and this surface) so a drift points straight at the
    /// missing emitter/widget or the missing documented-absence entry.
    fn assert_surface_partition(
        surface: &str,
        handled: impl Fn(AnalysisName) -> bool,
        absent: &[(AnalysisName, &str)],
    ) {
        let absent_names: std::collections::BTreeSet<&'static str> =
            absent.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(
            absent_names.len(),
            absent.len(),
            "{surface}: the documented-absence list has duplicate entries",
        );
        for (a, reason) in absent {
            assert!(
                !reason.trim().is_empty(),
                "{surface}: documented-absence entry for `{}` has an empty reason",
                a.as_str(),
            );
        }

        let mut unaccounted = Vec::new();
        let mut double_counted = Vec::new();
        for &a in AnalysisName::all() {
            match (handled(a), absent_names.contains(a.as_str())) {
                (false, false) => unaccounted.push(a.as_str()),
                (true, true) => double_counted.push(a.as_str()),
                _ => {}
            }
        }
        assert!(
            unaccounted.is_empty(),
            "{surface}: {} analysis/analyses are neither wired nor documented-absent — \
             wire a `{surface}` emitter/widget, or add each to the `{surface}` \
             documented-absence list with a reason: {}",
            unaccounted.len(),
            unaccounted.join(", "),
        );
        assert!(
            double_counted.is_empty(),
            "{surface}: {} analysis/analyses are BOTH wired and documented-absent — \
             remove the stale documented-absence entry: {}",
            double_counted.len(),
            double_counted.join(", "),
        );
    }

    /// dispatch surface: the `match &analysis` is compiler-exhaustive, so every
    /// analysis is dispatchable. Assert the seam agrees — an analysis that
    /// wired no output format at all would be a dead dispatch arm.
    #[test]
    fn dispatch_surface_reaches_every_analysis() {
        for &a in AnalysisName::all() {
            assert!(
                !supported_formats(a).is_empty(),
                "dispatch: `{}` declares no output format (unreachable analysis)",
                a.as_str(),
            );
        }
    }

    #[test]
    fn csv_surface_accounts_for_every_analysis() {
        assert_surface_partition(
            "csv",
            |a| supported_formats(a).contains(&"csv"),
            DOCUMENTED_ABSENT_CSV,
        );
    }

    #[test]
    fn markdown_surface_accounts_for_every_analysis() {
        assert_surface_partition(
            "markdown",
            |a| supported_formats(a).contains(&"markdown"),
            DOCUMENTED_ABSENT_MARKDOWN,
        );
    }

    #[test]
    fn spa_surface_accounts_for_every_analysis() {
        assert_surface_partition("spa", renders_in_spa, DOCUMENTED_ABSENT_SPA);
    }

    /// Compact guard for the (non-tracked) HTML surface: the analyses whose
    /// `supported_formats` includes `html` must be exactly `HTML_WIRED`.
    #[test]
    fn html_emitter_set_is_documented() {
        let derived: std::collections::BTreeSet<&str> = AnalysisName::all()
            .iter()
            .copied()
            .filter(|&a| supported_formats(a).contains(&"html"))
            .map(AnalysisName::as_str)
            .collect();
        let documented: std::collections::BTreeSet<&str> =
            HTML_WIRED.iter().map(|a| a.as_str()).collect();
        assert_eq!(
            derived, documented,
            "html: the bespoke-HTML-emitter set drifted from HTML_WIRED — reconcile \
             `supported_formats` with the analyses whose dispatch arm wires a real `write_html`",
        );
    }
}
