//! `codelore analyze` — run a single analysis over a repository and emit it.
//!
//! Pre-flights and opens the repository, ingests (or reuses cached) facts,
//! dispatches the requested `--analysis` to its writer, and renders the result
//! in the requested `--format` (csv, json, ndjson, sarif, markdown, parquet,
//! sqlite, html, spa, step-summary, gha). The `spa` format builds the
//! single-page dashboard from the full analysis suite.

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::{GixRepo, Repo as _};
use codelore_lib::cli_api::{AnalysisName, CodeLoreError, Options};

use crate::args::AnalyzeArgs;
use crate::notice_corpus_lens_absent;

#[allow(clippy::too_many_lines)] // long but linear: pre-flight → format-validate → ingest → 43-analysis dispatch → emit; splitting would obscure the top-level orchestration flow
pub(crate) fn analyze(args: &AnalyzeArgs, no_banner: bool) -> Result<()> {
    use codelore_lib::cli_api::output::banner;
    // Bracket the whole run with a wall-clock timer so the footer can report
    // "completed in 4.3s". Started before any work so pre-flight, ingest,
    // analysis, and emit all count toward the displayed duration — matches
    // what `cargo build`'s `Finished in Xs` includes.
    let started_at = std::time::Instant::now();

    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("parsing --analysis {:?}", args.analysis))?;

    let format = args.format.as_str();
    match format {
        "csv" | "json" | "ndjson" | "sarif" | "markdown" | "parquet" | "sqlite" | "html"
        | "spa" | "step-summary" | "gha" => {}
        other => {
            return Err(CodeLoreError::Analysis(format!(
                "unknown --format {other:?}. Supported: csv, json, ndjson, sarif, markdown, parquet, sqlite, html, spa, step-summary, gha"
            ))
            .into());
        }
    }

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

    let complexity_sample = match args.complexity_sample.as_str() {
        "head" => codelore_lib::cli_api::options::ComplexitySample::Head,
        "adaptive" | "full" => {
            return Err(CodeLoreError::InvalidOptions(
                "--complexity-sample currently supports only 'head'; \
                 'adaptive' and 'full' are not yet available"
                    .to_string(),
            )
            .into());
        }
        other => {
            return Err(CodeLoreError::InvalidOptions(format!(
                "unknown --complexity-sample value {other:?}; expected 'head'"
            ))
            .into());
        }
    };

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
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };
        let rows =
            codelore_lib::cli_api::analyses::clones::run_clones(&opts).context("run clones")?;
        match format {
            "csv" => {
                codelore_lib::cli_api::output::csv::write_clones_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            "json" => {
                codelore_lib::cli_api::output::json::write_json(&rows, &mut out)
                    .context("write json")?;
            }
            "markdown" => {
                codelore_lib::cli_api::output::markdown::write_clones_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            "sarif" => {
                let repo_root = args.repo.display().to_string();
                codelore_lib::cli_api::output::sarif::write_clones_sarif(
                    &rows, &repo_root, &mut out,
                )
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
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };

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
            repo_root: args.repo.display().to_string(),
            title: format!("CodeLore: {}", analysis.as_str()),
            generated_at,
            analysis_name: analysis.as_str(),
        };

        match &analysis {
            AnalysisName::Hotspots => dispatch_hotspots(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::HotspotVelocity => {
                dispatch_hotspot_velocity(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CodeHealth => dispatch_code_health(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::CodeAge => dispatch_code_age(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::AbsChurn => dispatch_abs_churn(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::AuthorChurn => {
                dispatch_author_churn(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::EntityChurn => {
                dispatch_entity_churn(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Communication => {
                dispatch_communication(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Ownership => dispatch_ownership(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Coupling => dispatch_coupling(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Summary => dispatch_summary(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Clones => dispatch_clones(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Revisions => dispatch_revisions(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Authors => dispatch_authors(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::TopCommitters => {
                dispatch_top_committers(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::GodClasses => dispatch_god_classes(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::ArchViolations => {
                dispatch_arch_violations(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::DependencyCycles => {
                dispatch_dependency_cycles(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ArchitectureRoles => {
                dispatch_architecture_roles(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Instability => {
                dispatch_instability(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CycleHealth => {
                dispatch_cycle_health(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::DefectValidation => {
                dispatch_defect_validation(&opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ArchitectureMetrics => {
                dispatch_architecture_metrics(&db, &opts, format, &ctx, &mut out)?;
            }
            // The only analysis that needs the Repo (it reads blobs at
            // historical revs), so it gets the opened `repo` directly
            // rather than the (db, opts) signature the rest share.
            AnalysisName::ArchitectureTrend => {
                dispatch_architecture_trend(&db, &repo, &opts, format, &ctx, &mut out)?;
            }
            // Also reads blobs at past revs — same special-case as ArchitectureTrend.
            AnalysisName::HealthTrend => {
                dispatch_health_trend(&db, &repo, &opts, format, &ctx, &mut out)?;
            }
            // Also needs the Repo (reads blobs at past revs while bisecting).
            AnalysisName::CycleOrigins => {
                dispatch_cycle_origins(&db, &repo, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ModularityViolations => {
                dispatch_modularity_violations(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::UnstableInterface => {
                dispatch_unstable_interface(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Crossing => dispatch_crossing(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::StaleCode => dispatch_stale_code(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::PairProgramming => {
                dispatch_pair_programming(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::LeadTime => dispatch_lead_time(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::BusFactor => dispatch_bus_factor(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::DeliveryFriction => {
                dispatch_delivery_friction(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::RefactoringTargets => {
                dispatch_refactoring_targets(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::KnowledgeIslands => {
                dispatch_knowledge_islands(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Soc => dispatch_soc(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Messages => dispatch_messages(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::MainDev => dispatch_main_dev(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::MainDevByRevs => {
                dispatch_main_dev_by_revs(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::MainDevByDeletions => {
                dispatch_main_dev_by_deletions(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::EntityEffort => {
                dispatch_entity_effort(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::EntityOwnership => {
                dispatch_entity_ownership(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CloneCoupling => {
                dispatch_clone_coupling(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Centrality => dispatch_centrality(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Communities => dispatch_communities(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::EffortExposure => {
                dispatch_effort_exposure(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CodeFamiliarity => {
                dispatch_code_familiarity(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::TeamComposition => {
                dispatch_team_composition(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CoordinationNeeds => {
                dispatch_coordination_needs(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::MarginalOwnerRisk => {
                dispatch_marginal_owner_risk(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ReleaseCadence => {
                dispatch_release_cadence(&repo, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::DeliveryMetrics => {
                dispatch_delivery_metrics(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::FunctionXray => {
                let target = opts.target.as_deref().ok_or_else(|| {
                    CodeLoreError::Analysis(
                        "--target <path> is required for function-xray".to_string(),
                    )
                })?;
                dispatch_function_xray(&db, &repo, &opts, target, format, &ctx, &mut out)?;
            }
            AnalysisName::FunctionCoupling => {
                let target = opts.target.as_deref().ok_or_else(|| {
                    CodeLoreError::Analysis(
                        "--target <path> is required for function-coupling".to_string(),
                    )
                })?;
                dispatch_function_coupling(&db, &repo, &opts, target, format, &ctx, &mut out)?;
            }
            AnalysisName::FindingHotspotOverlap => {
                // Requires the external sidecar: open it read-only from the cache
                // dir. open_nonempty returns None when the sidecar is absent OR
                // present-but-empty — both mean "no findings ingested yet", and
                // both surface the same pre-condition error here. We handle the
                // None case at this layer so we never create an empty sidecar as
                // a side-effect of an analysis read.
                let cache_root = args
                    .cache_dir
                    .clone()
                    .unwrap_or_else(codelore_lib::cli_api::cache::default_cache_root);
                let store = codelore_lib::cli_api::external::ExternalStore::open_nonempty(
                    &cache_root,
                    &args.repo,
                )
                .context("open external findings store")?
                .ok_or_else(|| {
                    codelore_lib::cli_api::CodeLoreError::Analysis(
                        "finding-hotspot-overlap requires prior `codelore ingest-sarif` \
                         (no external findings found)"
                            .to_string(),
                    )
                })?;
                dispatch_finding_hotspot_overlap(&db, &opts, &store, format, &ctx, &mut out)?;
            }
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
            // line carries the bulk of the post-run UX value.
            rows: None,
        };
        eprint!("{}", footer.render(banner::should_color()));
    }

    Ok(())
}

/// Side-channel context every per-analysis dispatch fn needs beyond `db`,
/// `opts`, and `format`: SARIF wants the repo root; HTML wants the page title,
/// a generated-at timestamp, and the analysis name (for the not-wired message).
struct EmitCtx {
    repo_root: String,
    title: String,
    generated_at: String,
    analysis_name: &'static str,
}

/// The verbatim error for `--format html` on an analysis whose row type is not
/// yet wired through the generic HTML emitter. Shared so every non-HTML
/// dispatch fn reports the same coverage list.
fn html_not_wired(analysis_name: &str) -> anyhow::Error {
    CodeLoreError::Analysis(format!(
        "--format html for analysis `{analysis_name}` not yet wired (covered: hotspots, \
         code-health, knowledge-islands, clone-coupling, summary, revisions, \
         authors, top-committers — file an issue if you need another)"
    ))
    .into()
}

/// The verbatim error for an output `--format` a given analysis's dispatch
/// fn doesn't wire. Shared so every per-analysis fallback arm reports the
/// same `CodeLoreError::Analysis` shape — `"<analysis> analysis supports
/// <list>; got <fmt>"` — keeping the analysis-failure exit code uniform.
fn unsupported_format(analysis_name: &str, supported: &str, fmt: &str) -> anyhow::Error {
    CodeLoreError::Analysis(format!(
        "{analysis_name} analysis supports {supported}; got {fmt:?}"
    ))
    .into()
}

fn dispatch_revisions(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::csv::write_revisions_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::json::write_revisions_json(&rows, out)
                .context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::markdown::write_revisions_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "revisions",
                "csv|json|markdown|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_hotspot_velocity(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::hotspot_velocity::run_hotspot_velocity(db, opts)
                    .context("run hotspot-velocity")?;
            codelore_lib::cli_api::output::csv::write_hotspot_velocity_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::hotspot_velocity::run_hotspot_velocity(db, opts)
                    .context("run hotspot-velocity")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::hotspot_velocity::run_hotspot_velocity(db, opts)
                    .context("run hotspot-velocity")?;
            codelore_lib::cli_api::output::markdown::write_hotspot_velocity_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "hotspot-velocity",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_hotspots(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::csv::write_hotspots_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::markdown::write_hotspots_markdown(&rows, out)
                .context("write markdown")?;
        }
        "sarif" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::sarif::write_hotspots_sarif(&rows, &ctx.repo_root, out)
                .context("write sarif")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "gha" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::gha::write_hotspots_gha(&rows, out)
                .context("write gha")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "hotspots",
                "csv|json|markdown|sarif|ndjson|gha|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_code_health(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    // Analyze has no --quiet; the notice self-suppresses off a TTY.
    notice_corpus_lens_absent(opts, false);
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::csv::write_code_health_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::markdown::write_code_health_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "code-health",
                "csv|json|markdown|ndjson|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_code_age(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::code_age::run_code_age(db, opts)
                .context("run code-age")?;
            codelore_lib::cli_api::output::csv::write_code_age_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::code_age::run_code_age(db, opts)
                .context("run code-age")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::code_age::run_code_age(db, opts)
                .context("run code-age")?;
            codelore_lib::cli_api::output::markdown::write_code_age_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("code-age", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_abs_churn(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_abs_churn(db, opts)
                .context("run abs-churn")?;
            codelore_lib::cli_api::output::csv::write_abs_churn_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_abs_churn(db, opts)
                .context("run abs-churn")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_abs_churn(db, opts)
                .context("run abs-churn")?;
            codelore_lib::cli_api::output::markdown::write_abs_churn_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("abs-churn", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_author_churn(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_author_churn(db, opts)
                .context("run author-churn")?;
            codelore_lib::cli_api::output::csv::write_author_churn_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_author_churn(db, opts)
                .context("run author-churn")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_author_churn(db, opts)
                .context("run author-churn")?;
            codelore_lib::cli_api::output::markdown::write_author_churn_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("author-churn", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_entity_churn(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_entity_churn(db, opts)
                .context("run entity-churn")?;
            codelore_lib::cli_api::output::csv::write_entity_churn_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_entity_churn(db, opts)
                .context("run entity-churn")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_entity_churn(db, opts)
                .context("run entity-churn")?;
            codelore_lib::cli_api::output::markdown::write_entity_churn_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("entity-churn", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_communication(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::communication::run_communication(db, opts)
                .context("run communication")?;
            codelore_lib::cli_api::output::csv::write_communication_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::communication::run_communication(db, opts)
                .context("run communication")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::communication::run_communication(db, opts)
                .context("run communication")?;
            codelore_lib::cli_api::output::markdown::write_communication_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "communication",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_ownership(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::ownership::run_ownership(db, opts)
                .context("run ownership")?;
            codelore_lib::cli_api::output::csv::write_ownership_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::ownership::run_ownership(db, opts)
                .context("run ownership")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::ownership::run_ownership(db, opts)
                .context("run ownership")?;
            codelore_lib::cli_api::output::markdown::write_ownership_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("ownership", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_coupling(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::csv::write_coupling_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::markdown::write_coupling_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "coupling",
                "csv|json|markdown|ndjson",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_summary(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::csv::write_summary_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::markdown::write_summary_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => return Err(unsupported_format("summary", "csv|json|markdown|html", fmt)),
    }
    Ok(())
}

fn dispatch_clones(
    _db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    // csv | json | markdown | sarif are short-circuited before the repo opens
    // (a HEAD-only tree-sitter walk needs no history); the arms remain here so
    // the wired-format set stays the single source of truth for clones.
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::csv::write_clones_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::markdown::write_clones_markdown(&rows, out)
                .context("write markdown")?;
        }
        "sarif" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::sarif::write_clones_sarif(&rows, &ctx.repo_root, out)
                .context("write sarif")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("clones", "csv|json|markdown|sarif", fmt)),
    }
    Ok(())
}

fn dispatch_authors(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::csv::write_authors_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::markdown::write_authors_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => return Err(unsupported_format("authors", "csv|json|markdown|html", fmt)),
    }
    Ok(())
}

fn dispatch_top_committers(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::csv::write_top_committers_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::markdown::write_top_committers_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "top-committers",
                "csv|json|markdown|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_god_classes(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::god_classes::run_god_classes(db, opts)
                .context("run god-classes")?;
            codelore_lib::cli_api::output::csv::write_god_classes_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::god_classes::run_god_classes(db, opts)
                .context("run god-classes")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::god_classes::run_god_classes(db, opts)
                .context("run god-classes")?;
            codelore_lib::cli_api::output::markdown::write_god_classes_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("god-classes", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_arch_violations(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::arch_violations::run_arch_violations(db, opts)
                    .context("run architecture-violations")?;
            codelore_lib::cli_api::output::csv::write_arch_violations_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::arch_violations::run_arch_violations(db, opts)
                    .context("run architecture-violations")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::arch_violations::run_arch_violations(db, opts)
                    .context("run architecture-violations")?;
            codelore_lib::cli_api::output::markdown::write_arch_violations_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-violations",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_instability(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::instability::run_instability(db, opts)
                .context("run instability")?;
            codelore_lib::cli_api::output::csv::write_instability_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::instability::run_instability(db, opts)
                .context("run instability")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::instability::run_instability(db, opts)
                .context("run instability")?;
            codelore_lib::cli_api::output::markdown::write_instability_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("instability", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_cycle_origins(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::cycle_origins::run_cycle_origins(db, repo, opts)
                    .context("run cycle-origins")?;
            codelore_lib::cli_api::output::csv::write_cycle_origins_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::cycle_origins::run_cycle_origins(db, repo, opts)
                    .context("run cycle-origins")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::cycle_origins::run_cycle_origins(db, repo, opts)
                    .context("run cycle-origins")?;
            codelore_lib::cli_api::output::markdown::write_cycle_origins_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "cycle-origins",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_architecture_trend(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, repo, opts,
            )
            .context("run architecture-trend")?;
            codelore_lib::cli_api::output::csv::write_architecture_trend_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, repo, opts,
            )
            .context("run architecture-trend")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, repo, opts,
            )
            .context("run architecture-trend")?;
            codelore_lib::cli_api::output::markdown::write_architecture_trend_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-trend",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_health_trend(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)
                    .context("run health-trend")?;
            codelore_lib::cli_api::output::csv::write_health_trend_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)
                    .context("run health-trend")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)
                    .context("run health-trend")?;
            codelore_lib::cli_api::output::markdown::write_health_trend_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format("health-trend", "csv|json|markdown", fmt));
        }
    }
    Ok(())
}

fn dispatch_cycle_health(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::cycle_health::run_cycle_health(db, opts)
                .context("run cycle-health")?;
            codelore_lib::cli_api::output::csv::write_cycle_health_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::cycle_health::run_cycle_health(db, opts)
                .context("run cycle-health")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::cycle_health::run_cycle_health(db, opts)
                .context("run cycle-health")?;
            codelore_lib::cli_api::output::markdown::write_cycle_health_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format("cycle-health", "csv|json|markdown", fmt));
        }
    }
    Ok(())
}

/// Reads a defect-calibration artifact (never the fact store) and emits its
/// evidence rows, so it takes no `FactsDb` — unlike the other dispatchers.
fn dispatch_defect_validation(
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::defect_validation::run_defect_validation(opts)
                    .context("run defect-validation")?;
            codelore_lib::cli_api::output::csv::write_defect_validation_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::defect_validation::run_defect_validation(opts)
                    .context("run defect-validation")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::defect_validation::run_defect_validation(opts)
                    .context("run defect-validation")?;
            codelore_lib::cli_api::output::markdown::write_defect_validation_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "defect-validation",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_architecture_metrics(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                    db, opts,
                )
                .context("run architecture-metrics")?;
            codelore_lib::cli_api::output::csv::write_architecture_metrics_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                    db, opts,
                )
                .context("run architecture-metrics")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                    db, opts,
                )
                .context("run architecture-metrics")?;
            codelore_lib::cli_api::output::markdown::write_architecture_metrics_markdown(
                &rows, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-metrics",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_architecture_roles(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
                db, opts,
            )
            .context("run architecture-roles")?;
            codelore_lib::cli_api::output::csv::write_architecture_roles_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
                db, opts,
            )
            .context("run architecture-roles")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
                db, opts,
            )
            .context("run architecture-roles")?;
            codelore_lib::cli_api::output::markdown::write_architecture_roles_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-roles",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_dependency_cycles(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::dependency_cycles::run_dependency_cycles(db, opts)
                    .context("run dependency-cycles")?;
            codelore_lib::cli_api::output::csv::write_dependency_cycles_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::dependency_cycles::run_dependency_cycles(db, opts)
                    .context("run dependency-cycles")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::dependency_cycles::run_dependency_cycles(db, opts)
                    .context("run dependency-cycles")?;
            codelore_lib::cli_api::output::markdown::write_dependency_cycles_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "dependency-cycles",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_modularity_violations(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(
                    db, opts,
                )
                .context("run modularity-violations")?;
            codelore_lib::cli_api::output::csv::write_modularity_violations_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(
                    db, opts,
                )
                .context("run modularity-violations")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(
                    db, opts,
                )
                .context("run modularity-violations")?;
            codelore_lib::cli_api::output::markdown::write_modularity_violations_markdown(
                &rows, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "modularity-violations",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_crossing(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::crossing::run_crossing(db, opts)
                .context("run crossing")?;
            codelore_lib::cli_api::output::csv::write_crossing_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::crossing::run_crossing(db, opts)
                .context("run crossing")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::crossing::run_crossing(db, opts)
                .context("run crossing")?;
            codelore_lib::cli_api::output::markdown::write_crossing_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("crossing", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_unstable_interface(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(
                db, opts,
            )
            .context("run unstable-interface")?;
            codelore_lib::cli_api::output::csv::write_unstable_interface_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(
                db, opts,
            )
            .context("run unstable-interface")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(
                db, opts,
            )
            .context("run unstable-interface")?;
            codelore_lib::cli_api::output::markdown::write_unstable_interface_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "unstable-interface",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_stale_code(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::stale_code::run_stale_code(db, opts)
                .context("run stale-code")?;
            codelore_lib::cli_api::output::csv::write_stale_code_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::stale_code::run_stale_code(db, opts)
                .context("run stale-code")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::stale_code::run_stale_code(db, opts)
                .context("run stale-code")?;
            codelore_lib::cli_api::output::markdown::write_stale_code_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("stale-code", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_pair_programming(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::pair_programming::run_pair_programming(db, opts)
                    .context("run pair-programming")?;
            codelore_lib::cli_api::output::csv::write_pair_programming_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::pair_programming::run_pair_programming(db, opts)
                    .context("run pair-programming")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::pair_programming::run_pair_programming(db, opts)
                    .context("run pair-programming")?;
            codelore_lib::cli_api::output::markdown::write_pair_programming_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "pair-programming",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_lead_time(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::csv::write_lead_time_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::markdown::write_lead_time_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "lead-time",
                "csv|json|ndjson|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_bus_factor(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::bus_factor::run_bus_factor(db, opts)
                .context("run bus-factor")?;
            codelore_lib::cli_api::output::csv::write_bus_factor_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::bus_factor::run_bus_factor(db, opts)
                .context("run bus-factor")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::bus_factor::run_bus_factor(db, opts)
                .context("run bus-factor")?;
            codelore_lib::cli_api::output::markdown::write_bus_factor_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("bus-factor", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_delivery_friction(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
                    .context("run delivery-friction")?;
            codelore_lib::cli_api::output::csv::write_delivery_friction_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
                    .context("run delivery-friction")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
                    .context("run delivery-friction")?;
            codelore_lib::cli_api::output::markdown::write_delivery_friction_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "delivery-friction",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_effort_exposure(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
                    .context("run effort-exposure")?;
            codelore_lib::cli_api::output::csv::write_effort_exposure_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
                    .context("run effort-exposure")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
                    .context("run effort-exposure")?;
            codelore_lib::cli_api::output::markdown::write_effort_exposure_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "effort-exposure",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_code_familiarity(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                    .context("run code-familiarity")?;
            codelore_lib::cli_api::output::csv::write_code_familiarity_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                    .context("run code-familiarity")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                    .context("run code-familiarity")?;
            codelore_lib::cli_api::output::markdown::write_code_familiarity_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "code-familiarity",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_team_composition(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
                    .context("run team-composition")?;
            codelore_lib::cli_api::output::csv::write_team_composition_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
                    .context("run team-composition")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
                    .context("run team-composition")?;
            codelore_lib::cli_api::output::markdown::write_team_composition_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "team-composition",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_coordination_needs(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(
                db, opts,
            )
            .context("run coordination-needs")?;
            codelore_lib::cli_api::output::csv::write_coordination_needs_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(
                db, opts,
            )
            .context("run coordination-needs")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(
                db, opts,
            )
            .context("run coordination-needs")?;
            codelore_lib::cli_api::output::markdown::write_coordination_needs_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "coordination-needs",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_marginal_owner_risk(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(
                    db, opts,
                )
                .context("run marginal-owner-risk")?;
            codelore_lib::cli_api::output::csv::write_marginal_owner_risk_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(
                    db, opts,
                )
                .context("run marginal-owner-risk")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(
                    db, opts,
                )
                .context("run marginal-owner-risk")?;
            codelore_lib::cli_api::output::markdown::write_marginal_owner_risk_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "marginal-owner-risk",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_finding_hotspot_overlap(
    db: &FactsDb,
    opts: &Options,
    store: &codelore_lib::cli_api::external::ExternalStore,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    // Reject unsupported formats before running the (non-trivial) analysis, so
    // a bad format still surfaces the format error rather than an analysis error.
    match format {
        "csv" | "json" | "markdown" => {}
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "finding-hotspot-overlap",
                "csv|json|markdown",
                fmt,
            ));
        }
    }

    let rows =
        codelore_lib::cli_api::analyses::finding_hotspot_overlap::run_finding_hotspot_overlap(
            db, opts, store,
        )
        .context("run finding-hotspot-overlap")?;
    match format {
        "csv" => codelore_lib::cli_api::output::csv::write_finding_hotspot_overlap_csv(&rows, out)
            .context("write csv"),
        "json" => codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json"),
        // The format was validated above; only the three writer formats remain.
        _ => codelore_lib::cli_api::output::markdown::write_finding_hotspot_overlap_markdown(
            &rows, out,
        )
        .context("write markdown"),
    }
}

fn dispatch_release_cadence(
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(repo, opts)
                    .context("run release-cadence")?;
            codelore_lib::cli_api::output::csv::write_release_cadence_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(repo, opts)
                    .context("run release-cadence")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(repo, opts)
                    .context("run release-cadence")?;
            codelore_lib::cli_api::output::markdown::write_release_cadence_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "release-cadence",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_delivery_metrics(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, opts)
                    .context("run delivery-metrics")?;
            codelore_lib::cli_api::output::csv::write_delivery_metrics_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, opts)
                    .context("run delivery-metrics")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, opts)
                    .context("run delivery-metrics")?;
            codelore_lib::cli_api::output::markdown::write_delivery_metrics_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "delivery-metrics",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_function_xray(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    target: &str,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                db, repo, opts, target,
            )
            .context("run function-xray")?;
            codelore_lib::cli_api::output::csv::write_function_xray_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                db, repo, opts, target,
            )
            .context("run function-xray")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                db, repo, opts, target,
            )
            .context("run function-xray")?;
            codelore_lib::cli_api::output::markdown::write_function_xray_markdown(
                &rows, target, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "function-xray",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_function_coupling(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    target: &str,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::function_coupling::run_function_coupling(
                db, repo, opts, target,
            )
            .context("run function-coupling")?;
            codelore_lib::cli_api::output::csv::write_function_coupling_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::function_coupling::run_function_coupling(
                db, repo, opts, target,
            )
            .context("run function-coupling")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::function_coupling::run_function_coupling(
                db, repo, opts, target,
            )
            .context("run function-coupling")?;
            codelore_lib::cli_api::output::markdown::write_function_coupling_markdown(
                &rows, target, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "function-coupling",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_refactoring_targets(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    let rows =
        codelore_lib::cli_api::analyses::refactoring_targets::run_refactoring_targets(db, opts)
            .context("run refactoring-targets")?;
    match format {
        "csv" => codelore_lib::cli_api::output::csv::write_refactoring_targets_csv(&rows, out)
            .context("write csv")?,
        "json" => {
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            codelore_lib::cli_api::output::markdown::write_refactoring_targets_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => {
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "refactoring-targets",
                "csv|json|markdown|ndjson|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_knowledge_islands(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::csv::write_knowledge_islands_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::markdown::write_knowledge_islands_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "knowledge-islands",
                "csv|json|markdown|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_soc(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::soc::run_soc(db, opts).context("run soc")?;
            codelore_lib::cli_api::output::csv::write_soc_csv(&rows, out).context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::soc::run_soc(db, opts).context("run soc")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::soc::run_soc(db, opts).context("run soc")?;
            codelore_lib::cli_api::output::markdown::write_soc_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("soc", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_messages(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::messages::run_messages(db, opts)
                .context("run messages")?;
            codelore_lib::cli_api::output::csv::write_messages_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::messages::run_messages(db, opts)
                .context("run messages")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::messages::run_messages(db, opts)
                .context("run messages")?;
            codelore_lib::cli_api::output::markdown::write_messages_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("messages", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_main_dev(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev(db, opts)
                .context("run main-dev")?;
            codelore_lib::cli_api::output::csv::write_main_dev_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev(db, opts)
                .context("run main-dev")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev(db, opts)
                .context("run main-dev")?;
            codelore_lib::cli_api::output::markdown::write_main_dev_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("main-dev", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_main_dev_by_revs(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_revs(db, opts)
                .context("run main-dev-by-revs")?;
            codelore_lib::cli_api::output::csv::write_main_dev_by_revs_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_revs(db, opts)
                .context("run main-dev-by-revs")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_revs(db, opts)
                .context("run main-dev-by-revs")?;
            codelore_lib::cli_api::output::markdown::write_main_dev_by_revs_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "main-dev-by-revs",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_main_dev_by_deletions(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_deletions(db, opts)
                    .context("run main-dev-by-deletions")?;
            codelore_lib::cli_api::output::csv::write_main_dev_by_deletions_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_deletions(db, opts)
                    .context("run main-dev-by-deletions")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_deletions(db, opts)
                    .context("run main-dev-by-deletions")?;
            codelore_lib::cli_api::output::markdown::write_main_dev_by_deletions_markdown(
                &rows, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "main-dev-by-deletions",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_entity_effort(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::entity_effort::run_entity_effort(db, opts)
                .context("run entity-effort")?;
            codelore_lib::cli_api::output::csv::write_entity_effort_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::entity_effort::run_entity_effort(db, opts)
                .context("run entity-effort")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::entity_effort::run_entity_effort(db, opts)
                .context("run entity-effort")?;
            codelore_lib::cli_api::output::markdown::write_entity_effort_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "entity-effort",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_entity_ownership(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
                    .context("run entity-ownership")?;
            codelore_lib::cli_api::output::csv::write_entity_ownership_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
                    .context("run entity-ownership")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
                    .context("run entity-ownership")?;
            codelore_lib::cli_api::output::markdown::write_entity_ownership_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "entity-ownership",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_clone_coupling(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::csv::write_clone_coupling_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::markdown::write_clone_coupling_markdown(&rows, out)
                .context("write markdown")?;
        }
        "sarif" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::sarif::write_clone_coupling_sarif(
                &rows,
                &ctx.repo_root,
                out,
            )
            .context("write sarif")?;
        }
        "html" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "clone-coupling",
                "csv|json|markdown|sarif|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_centrality(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::centrality::run_centrality(db, opts)
                .context("run centrality")?;
            codelore_lib::cli_api::output::csv::write_centrality_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::centrality::run_centrality(db, opts)
                .context("run centrality")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::centrality::run_centrality(db, opts)
                .context("run centrality")?;
            codelore_lib::cli_api::output::markdown::write_centrality_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("centrality", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_communities(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let result = codelore_lib::cli_api::analyses::communities::run_communities(db, opts)
                .context("run communities")?;
            codelore_lib::cli_api::output::csv::write_communities_csv(&result, out)
                .context("write csv")?;
        }
        "json" => {
            let result = codelore_lib::cli_api::analyses::communities::run_communities(db, opts)
                .context("run communities")?;
            codelore_lib::cli_api::output::json::write_communities_json(&result, out)
                .context("write json")?;
        }
        "markdown" => {
            let result = codelore_lib::cli_api::analyses::communities::run_communities(db, opts)
                .context("run communities")?;
            codelore_lib::cli_api::output::markdown::write_communities_markdown(&result, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("communities", "csv|json|markdown", fmt)),
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
    let entity_ownership =
        codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: entity-ownership analysis failed; skipping: {e}");
                Vec::new()
            });
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
    let trends = run_trends(db, &top_paths).unwrap_or_else(|e| {
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
    // Architecture-trend is the one dashboard input that needs the repo
    // (it re-reads source at sampled historical revs) and the only one
    // markedly heavier than an SQL query — so it opens its own repo
    // handle and degrades gracefully: any failure leaves the trend chart
    // empty rather than sinking the whole dashboard.
    let architecture_trend = codelore_lib::cli_api::repo::GixRepo::open(repo_path)
        .map_err(anyhow::Error::from)
        .and_then(|repo| {
            codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, &repo, opts,
            )
            .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: architecture-trend failed; skipping: {e}");
            Vec::new()
        });
    // Repo health timeline + per-file series + transitions — like
    // architecture-trend, needs the repo to re-read source at sampled
    // historical revs. Opens its own handle and degrades gracefully.
    let (health_trend, file_health_series, health_transitions) =
        codelore_lib::cli_api::repo::GixRepo::open(repo_path)
            .map_err(anyhow::Error::from)
            .and_then(|repo| {
                codelore_lib::cli_api::analyses::health_trend::run_health_trend_detail(
                    db, &repo, opts,
                )
                .map_err(anyhow::Error::from)
            })
            .map_or_else(
                |e| {
                    tracing::warn!("dashboard: health-trend failed; skipping: {e}");
                    (Vec::new(), Vec::new(), Vec::new())
                },
                |d| (d.trend, d.file_series, d.transitions),
            );
    // Effort-exposure: LOC/commit/churn share per band over the trailing
    // window. Pure SQL over already-ingested facts; runs the same
    // code-health HEAD scan as the factor header (cheap, cached). Degrades
    // to empty on analysis failure so no widget is shown.
    let effort_exposure =
        codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: effort-exposure analysis failed; skipping: {e}");
                Vec::new()
            });
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
    // Release cadence — uses a separate Repo handle (same as architecture-trend).
    let release_cadence = codelore_lib::cli_api::repo::GixRepo::open(repo_path)
        .map_err(anyhow::Error::from)
        .and_then(|repo| {
            codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(&repo, opts)
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: release-cadence failed; skipping: {e}");
            Vec::new()
        });
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
    // Per-file function X-Ray for the top-10 hotspot paths. Each call
    // opens the HEAD blob via the gix repo handle (cheap; tree-sitter
    // spans only) and joins against already-ingested hunks. One failure
    // per path degrades gracefully to an empty rows vec for that path.
    let function_xray: Vec<codelore_lib::cli_api::output::spa::FileFunctionXray> =
        codelore_lib::cli_api::repo::GixRepo::open(repo_path).map_or_else(
            |e| {
                tracing::warn!("dashboard: could not open repo for function-xray; skipping: {e}");
                Vec::new()
            },
            |xray_repo| {
                hotspots
                    .iter()
                    .take(10)
                    .filter_map(|h| {
                        match codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                            db, &xray_repo, opts, &h.path,
                        ) {
                            Ok(rows) if !rows.is_empty() => {
                                Some(codelore_lib::cli_api::output::spa::FileFunctionXray {
                                    path: h.path.clone(),
                                    rows,
                                })
                            }
                            Ok(_) => None,
                            Err(e) => {
                                tracing::debug!(
                                    "dashboard: function-xray skipped for {}: {e}",
                                    h.path
                                );
                                None
                            }
                        }
                    })
                    .collect()
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

    let mut out = std::fs::File::create(output)
        .with_context(|| format!("create spa output {}", output.display()))?;
    write_spa(&dash, title, &repo_display, &generated_at, &mut out)
        .context("write spa dashboard")?;
    // Drop the writer so file size is finalised on disk before the
    // stat() below.
    drop(out);

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
        let mut out = std::fs::File::create(path)
            .with_context(|| format!("create step-summary output {}", path.display()))?;
        write_step_summary(&dash, title, &repo_display, &generated_at, &mut out)
            .context("write step-summary")?;
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
