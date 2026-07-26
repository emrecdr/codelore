//! codelore — Behavioral Code Analyzer CLI.

mod analyze;
mod args;
mod calibrate;
mod calibrate_defects;
mod check;
mod diff;
mod diff_output;
mod mcp;

use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::{GixRepo, Repo as _};
use codelore_lib::cli_api::{AnalysisName, CodeLoreError, Options};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::args::{Cli, Command, DiffArgs, GateFormat, IngestSarifArgs, McpArgs};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        // Map CodeLoreError to its spec §6.6 exit code if present in the chain.
        // Falls back to 1 for non-CodeLoreError errors (e.g. clap parse errors).
        let code = e
            .chain()
            .find_map(|cause| cause.downcast_ref::<codelore_lib::cli_api::CodeLoreError>())
            .map_or(1, codelore_lib::cli_api::CodeLoreError::exit_code);
        std::process::exit(code);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Analyze(args) => analyze::analyze(&args, cli.no_banner),
        Command::Diff(args) => run_diff_cmd(&args),
        Command::Completions(args) => {
            run_completions_cmd(&args);
            Ok(())
        }
        Command::Explain(args) => run_explain_cmd(&args),
        Command::Schema(args) => run_schema_cmd(&args),
        Command::Profile => run_profile_cmd(),
        Command::Docs => run_docs_cmd(),
        Command::Check(args) => check::run_check_cmd(&args),
        Command::Gate(args) => run_gate_cmd(&args),
        Command::Mcp(args) => run_mcp_cmd(&args),
        Command::IngestSarif(args) => run_ingest_sarif_cmd(&args),
        Command::Calibrate(args) => calibrate::run_calibrate_cmd(&args),
        Command::CalibrateDefects(args) => calibrate_defects::run_calibrate_defects_cmd(&args),
    }
}

fn run_mcp_cmd(args: &McpArgs) -> Result<()> {
    mcp::run_mcp_server(
        args.repo.clone(),
        args.defect_calibration.clone(),
        args.allow_foreign_calibration,
    )
}

/// Ingest one or more SARIF files into the per-repo external-findings sidecar.
///
/// For each file, parses all SARIF runs, groups findings by engine, and
/// calls `ExternalStore::replace_engine` per engine so re-ingest is
/// idempotent. Prints a summary line to stdout on success.
fn run_ingest_sarif_cmd(args: &IngestSarifArgs) -> Result<()> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::external::{
        ExternalStore, group_findings_by_engine, parse_sarif_with_engines,
    };

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    let store = ExternalStore::open_or_create(&cache_root, &args.repo)
        .context("open external findings store")?;

    // Parse all input files into a flat vec, then group by engine so that
    // two SARIF files from the same engine are combined rather than
    // overwriting each other. Track every engine name present across all
    // inputs — including runs that produced zero results — so a clean re-scan
    // clears the engine's stale rows instead of leaving them behind.
    let mut all_findings = Vec::new();
    let mut all_engines: Vec<String> = Vec::new();
    for path in &args.file {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read SARIF file {}", path.display()))?;
        let (findings, engines) = parse_sarif_with_engines(&raw)
            .with_context(|| format!("parse SARIF file {}", path.display()))?;
        all_findings.extend(findings);
        for engine in engines {
            if !all_engines.contains(&engine) {
                all_engines.push(engine);
            }
        }
    }
    let mut by_engine = group_findings_by_engine(all_findings);
    // Seed empty batches for engines that ran but flagged nothing this pass.
    // `replace_engine` with an empty slice deletes that engine's prior rows,
    // keeping the stored count aligned with the current scanner run.
    for engine in all_engines {
        by_engine.entry(engine).or_default();
    }

    let engine_count = by_engine.len();
    let mut total_ingested: usize = 0;
    for (engine, findings) in &by_engine {
        let n = store
            .replace_engine(engine, findings)
            .with_context(|| format!("ingest findings for engine {engine}"))?;
        total_ingested += n;
    }

    println!(
        "ingested {} finding(s) from {} engine(s) → {}",
        total_ingested,
        engine_count,
        store.path().display(),
    );

    Ok(())
}

/// Print the one-per-run notice that the corpus-percentile lens is inactive:
/// no `--calibration` artifact was passed and no world corpus is embedded, so
/// code-health rows carry no `corpus_percentile`. Called exactly once on each
/// code-health-producing path, so the "deduped per run" contract is structural.
/// Suppressed under `quiet` and when stderr is not a TTY (so redirected /
/// CI output stays clean), mirroring the pre-flight banner's print policy.
pub(crate) fn notice_corpus_lens_absent(opts: &Options, quiet: bool) {
    use std::io::IsTerminal as _;
    if quiet || !std::io::stderr().is_terminal() {
        return;
    }
    let embedded_absent = codelore_lib::cli_api::calibration::embedded_world().is_none();
    if opts.calibration.is_none() && embedded_absent {
        eprintln!(
            "note: corpus-percentile lens inactive — no calibration artifact (pass --calibration <path> or build one with `codelore calibrate`)."
        );
    }
}

/// Working-tree quality gate. Projects what the uncommitted edits do to code
/// health and the import graph vs HEAD (the change-set engine), evaluates the
/// working-tree `[diff]` gates against the projection, and exits 0 (pass) or
/// 1 (fail) — the same exit contract as `codelore check`. Writes
/// `result=pass|fail` to `$GITHUB_OUTPUT` for direct GitHub Actions
/// step-output consumption.
fn run_gate_cmd(args: &args::GateArgs) -> Result<()> {
    use codelore_lib::change_set::build_change_set_report;
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::quality_gates::ledger::{append_gate_runs, now_utc_ts};
    use codelore_lib::cli_api::quality_gates::{Thresholds, evaluate_gate_thresholds};

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    let thresholds = if let Some(path) = &args.thresholds_file {
        Thresholds::from_path(path).context("load thresholds file")?
    } else {
        Thresholds::discover(&args.repo).context("discover thresholds file")?
    };

    if thresholds.is_empty() {
        if !args.quiet {
            eprintln!(
                "codelore gate: no thresholds configured (no `.codelore-thresholds.toml` at repo root); vacuously passing."
            );
        }
        // A JSON consumer still gets one contract document on stdout — the same
        // empty shape a clean tree emits — so an agent hook that always runs
        // `gate --format json` never has to special-case a repo with no
        // thresholds configured.
        if matches!(args.format, GateFormat::Json) {
            println!(
                "{}",
                serde_json::json!({ "changes": [], "findings": [], "violations": [] })
            );
        }
        write_github_output("result", "pass");
        return Ok(());
    }

    // Mirrors `quality_gates::resolve_defect_calibration`, but reuses the
    // `thresholds` value already loaded above instead of re-discovering
    // (and re-parsing) the thresholds file.
    let resolved_defect_calibration = args.defect_calibration.clone().or_else(|| {
        thresholds.calibration.defect_artifact.clone().map(|p| {
            if p.is_absolute() {
                p
            } else {
                args.repo.join(p)
            }
        })
    });

    let opts = Options {
        repo_path: args.repo.clone(),
        defect_calibration: resolved_defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        temp_dir: args.temp_dir.clone(),
        ..Options::default()
    };
    opts.validate().context("validate options")?;
    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db =
        FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root).context("ingest")?;

    let changes = repo
        .worktree_changes()
        .context("enumerate working-tree changes")?;
    if changes.is_empty() {
        report_gate_clean_tree(args);
        return Ok(());
    }

    let report = build_change_set_report(&db, &repo, &opts, &cache_root)
        .context("build change-set report")?;
    let violations = evaluate_gate_thresholds(&thresholds, &report);

    let ts = now_utc_ts();
    append_gate_runs(
        &cache_root,
        &args.repo,
        &gate_ledger_records(&thresholds, &report, &violations, &ts),
    );

    emit_gate_run_notices(args, &thresholds, &report);
    if matches!(args.format, GateFormat::Json) {
        render_gate_json(&report, &violations)?;
    }
    render_gate_verdict(args, &report, &violations)
}

/// Report the empty-change-set pass: a clean tree means there is nothing to
/// gate, which is an explicit PASS (exit 0), not a skipped evaluation. The
/// JSON format still gets one document on stdout — with the contract keys
/// present and empty — so downstream parsers never special-case a clean tree.
fn report_gate_clean_tree(args: &args::GateArgs) {
    let verdict = "✅ codelore gate: PASS (no working-tree changes to gate)";
    if matches!(args.format, GateFormat::Json) {
        println!(
            "{}",
            serde_json::json!({ "changes": [], "findings": [], "violations": [] })
        );
        eprintln!("{verdict}");
    } else {
        println!("{verdict}");
    }
    write_github_output("result", "pass");
    write_github_output("violations", "0");
}

/// Emit the gate run's stderr notices (suppressed under `--quiet`): the
/// merge-in-progress note, and the skip notice when `delta_code_health_min`
/// is configured but a whole-repo code-health median is unavailable on
/// either side (no scoreable files) — skipped, not failed.
fn emit_gate_run_notices(
    args: &args::GateArgs,
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    report: &codelore_lib::change_set::ChangeSetReport,
) {
    if args.quiet {
        return;
    }
    if report.merge_in_progress {
        eprintln!("note: merge/rebase in progress — projection reflects committed HEAD history");
    }
    if thresholds.diff.delta_code_health_min.is_some()
        && (report.health.baseline_median.is_none() || report.health.projected_median.is_none())
    {
        eprintln!(
            "  ⚠ delta_code_health_min: skipped — no whole-repo code-health median to compare"
        );
    }
}

/// Render the gate's JSON document: the full change-set report with the
/// evaluated `violations` array folded in as a sibling key, one document on
/// stdout. Verdict lines stay on stderr so stdout is clean JSON.
fn render_gate_json(
    report: &codelore_lib::change_set::ChangeSetReport,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
) -> Result<()> {
    let mut doc = serde_json::to_value(report).context("serialize change-set report")?;
    doc["violations"] = serde_json::to_value(violations).context("serialize gate violations")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&doc).context("render gate JSON")?
    );
    Ok(())
}

/// Number of delta rows the text render shows; the rest fold into a
/// `(+n more files)` tail. The JSON document always carries every row.
const GATE_DELTA_TABLE_ROWS: usize = 10;

/// Number of advisory-finding rows the text render shows; the rest fold into
/// a `(+n more findings)` tail. Mirrors [`GATE_DELTA_TABLE_ROWS`]'s
/// render-only cap — `report.findings` (the JSON document, and the in-memory
/// `ChangeSetReport`) always carries every finding by design (spec §6); only
/// the rendered text is bounded, so a large coupling cluster or a big batch
/// of added files can never blow the token budget.
const GATE_FINDINGS_ROWS: usize = 10;

/// Print the advisory (non-verdict) text sections to stdout: one line per
/// finding (capped at [`GATE_FINDINGS_ROWS`] with a `(+n more findings)`
/// tail), then the per-file delta table in the engine's order (|delta|
/// descending, unscored rows last). Suppressed under `--quiet`.
fn render_gate_advisories(
    args: &args::GateArgs,
    report: &codelore_lib::change_set::ChangeSetReport,
) {
    if args.quiet {
        return;
    }
    for f in report.findings.iter().take(GATE_FINDINGS_ROWS) {
        println!("[{}] {}: {}", f.kind, f.path, f.detail);
    }
    let hidden_findings = report.findings.len().saturating_sub(GATE_FINDINGS_ROWS);
    if hidden_findings > 0 {
        println!("(+{hidden_findings} more findings)");
    }
    for d in report.health.deltas.iter().take(GATE_DELTA_TABLE_ROWS) {
        match (d.baseline_score, d.projected_score, d.delta) {
            (Some(b), Some(p), Some(delta)) => {
                println!("{}  {b:.1} → {p:.1}  ({delta:+.1})", d.path);
            }
            _ => println!(
                "{}  — {}",
                d.path,
                d.reason.as_deref().unwrap_or("not scored")
            ),
        }
    }
    let hidden = report
        .health
        .deltas
        .len()
        .saturating_sub(GATE_DELTA_TABLE_ROWS);
    if hidden > 0 {
        println!("(+{hidden} more files)");
    }
}

/// Print the verdict, write the GitHub Actions step outputs, and apply
/// check's exit contract: any violation bails (exit 1).
fn render_gate_verdict(
    args: &args::GateArgs,
    report: &codelore_lib::change_set::ChangeSetReport,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
) -> Result<()> {
    if violations.is_empty() {
        if matches!(args.format, GateFormat::Text) {
            println!(
                "✅ codelore gate: PASS ({} changed file(s) evaluated)",
                report.changes.len()
            );
            render_gate_advisories(args, report);
        } else {
            // JSON keeps stdout pure for the report document (already printed),
            // so the verdict line goes to stderr — mirroring the clean-tree and
            // FAIL paths, and honoring the contract that a verdict line is
            // emitted regardless of format.
            eprintln!(
                "✅ codelore gate: PASS ({} changed file(s) evaluated)",
                report.changes.len()
            );
        }
        write_github_output("result", "pass");
        write_github_output("violations", "0");
        return Ok(());
    }
    eprintln!("❌ codelore gate: FAIL — {} violation(s)", violations.len());
    if matches!(args.format, GateFormat::Text) {
        if !args.quiet {
            for v in violations {
                eprintln!(
                    "  - {gate}: {path} — actual {actual} vs threshold {threshold}",
                    gate = v.gate,
                    path = v.path,
                    actual = v.actual,
                    threshold = v.threshold,
                );
            }
        }
        render_gate_advisories(args, report);
    }
    write_github_output("result", "fail");
    write_github_output("violations", &violations.len().to_string());
    // Plain anyhow::bail carries no CodeLoreError, so main()'s chain-walk
    // falls through to the default exit code 1 — check parity by design;
    // typed CodeLoreError variants keep their repo/output exit codes.
    anyhow::bail!("{} gate violation(s) — see above", violations.len());
}

/// Build the ledger records for one gate run: one record per configured
/// working-tree gate, `mode: "gate"`. Counts (offending files, newly cyclic
/// paths) are recorded as the measured value, mirroring the ledger's
/// violation-count convention for gates without a single scalar.
fn gate_ledger_records(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    report: &codelore_lib::change_set::ChangeSetReport,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
    ts: &str,
) -> Vec<codelore_lib::cli_api::quality_gates::ledger::GateRunRecord> {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;
    let rec = |gate: &str, threshold: f64, value: f64, verdict: &str| GateRunRecord {
        ts: ts.to_owned(),
        head_sha: report.head_sha.clone(),
        gate: gate.to_owned(),
        threshold,
        value,
        verdict: verdict.to_owned(),
        mode: "gate".to_owned(),
    };
    let count = |gate: &str| violations.iter().filter(|v| v.gate == gate).count();
    let count_f64 = |gate: &str| f64::from(u32::try_from(count(gate)).unwrap_or(u32::MAX));
    let verdict = |gate: &str| if count(gate) == 0 { "passed" } else { "failed" };

    let mut records = Vec::new();
    let d = &thresholds.diff;
    if let Some(min) = d.delta_code_health_min {
        let record = match (
            report.health.baseline_median,
            report.health.projected_median,
        ) {
            (Some(base), Some(projected)) => rec(
                "delta_code_health_min",
                min,
                projected - base,
                verdict("delta_code_health_min"),
            ),
            _ => rec("delta_code_health_min", min, 0.0, "skipped"),
        };
        records.push(record);
    }
    if let Some(min) = d.delta_code_health_min_per_file {
        records.push(rec(
            "delta_code_health_min_per_file",
            min,
            count_f64("delta_code_health_min_per_file"),
            verdict("delta_code_health_min_per_file"),
        ));
    }
    if let Some(min) = d.new_file_health_min {
        records.push(rec(
            "new_file_health_min",
            min,
            count_f64("new_file_health_min"),
            verdict("new_file_health_min"),
        ));
    }
    if d.no_new_cycles {
        records.push(rec(
            "no_new_cycles",
            0.0,
            count_f64("no_new_cycles"),
            verdict("no_new_cycles"),
        ));
    }
    records
}

/// Write a single `key=value` line to `$GITHUB_OUTPUT` when the env
/// var is set. No-op outside GitHub Actions.
pub(crate) fn write_github_output(key: &str, value: &str) {
    if let Ok(path) = std::env::var("GITHUB_OUTPUT")
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{key}={value}");
    }
}

/// Operational telemetry. Prints what `CodeLore` ships under the
/// hood — schema version, pinned dependency versions, supported
/// analysis count, supported output format count. Useful for triage
/// when behaviour surprises a user.
#[allow(clippy::unnecessary_wraps)] // dispatcher uniformity — every arm returns Result<()>
fn run_profile_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    println!("# CodeLore profile\n");
    println!("**Version**: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "**Schema**: schema_v{} (`facts/schema_v1.sql`)",
        codelore_lib::cli_api::facts::schema::CURRENT_SCHEMA_VERSION
    );
    println!("**Analyses**: {} registered", AnalysisName::all().len());
    println!("**Output formats**: csv | json | sarif | markdown | parquet | sqlite | html | spa");
    println!(
        "**Pinned third-party**:\n  - gix {gix}\n  - DuckDB {duckdb}\n  - tree-sitter 0.25.x (Rust/Python/Java/JS/TS/TSX/C++)",
        gix = codelore_lib::cli_api::provenance::GIX_VERSION,
        duckdb = codelore_lib::cli_api::provenance::DUCKDB_VERSION,
    );
    println!("\n**Cache root**:");
    if let Some(dir) = dirs::cache_dir() {
        println!("  {}/codelore/", dir.display());
    } else {
        println!("  <unavailable on this platform>");
    }
    println!(
        "\n**SPA feature**: {}",
        if cfg!(feature = "spa") {
            "ENABLED"
        } else {
            "disabled (build with --features spa to opt in)"
        }
    );
    println!("\n_For per-analysis SQL + citations, run `codelore explain <topic>`._");
    Ok(())
}

/// Markdown dump of every supported analysis. Seeds the planned
/// full static-HTML doc site.
#[allow(clippy::unnecessary_wraps)] // dispatcher uniformity — every arm returns Result<()>
fn run_docs_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    println!("# CodeLore — Analysis catalogue\n");
    println!(
        "Auto-generated from `AnalysisName::all()`. Run `codelore explain <topic>` for per-analysis citations and formulas. The full citation chain lives in `docs/research-foundations.md`.\n"
    );
    println!("## Supported analyses\n");
    for analysis in AnalysisName::all() {
        println!("- `{}`", analysis.as_str());
    }
    println!("\n## Output formats\n");
    for fmt in &[
        ("csv", "code-maat-compatible flat tables"),
        ("json", "stable JSON shape per row type"),
        (
            "ndjson",
            "newline-delimited JSON — one row per line for stream consumers (LSP, `jq -c`, CI pipelines)",
        ),
        ("sarif", "SARIF 2.1.0 — surfaces in GitHub Code Scanning"),
        ("markdown", "GFM tables for `$GITHUB_STEP_SUMMARY`"),
        (
            "gha",
            "GitHub Actions workflow commands — `::error::` / `::warning::` / `::notice::` on stdout, surfaced as inline PR annotations",
        ),
        ("parquet", "columnar bulk export for analytical pipelines"),
        ("sqlite", "full DuckDB fact-store dump"),
        ("html", "self-contained per-analysis HTML report"),
        (
            "spa",
            "single-file interactive dashboard (opt-in via `spa` feature)",
        ),
    ] {
        println!("- `{}` — {}", fmt.0, fmt.1);
    }
    println!("\n## Conventions\n");
    println!("- Files alive at HEAD only (deleted files excluded from path-aggregating analyses)");
    println!("- Mailmap + `.codelore-teams` + `.codelorebots` consulted at ingest time");
    println!("- `.gitignore` / `.codeloreignore` honoured");
    println!("- `--time-bucket` supported on: hotspots, coupling, soc, code-health");
    println!(
        "\n## Reproducibility\n\nEvery file output is paired with a `.provenance.json` sidecar capturing the run's full `Options` shape. SQLite outputs embed the equivalent inside the `provenance` table."
    );
    println!(
        "\n_See also: `codelore profile` for operational telemetry, `codelore schema <type>` for row schemas, `docs/research-foundations.md` for citations._"
    );
    Ok(())
}

/// Emit shell-completion script for the given shell to stdout. The
/// `clap_complete` derive macro consumes our existing clap spec —
/// no hand-maintained completion files.
fn run_completions_cmd(args: &args::CompletionsArgs) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(args.shell, &mut cmd, bin_name, &mut std::io::stdout());
}

/// Print formula + citation + SQL for the named metric or analysis.
/// With no topic, lists every supported topic. Makes the auditable-
/// formulas brand promise tactile on the CLI side — `codelore
/// explain hotspot-score` is the answer to "why does this file have
/// score 8.4?".
#[allow(clippy::too_many_lines)] // catalogue table — splitting would hurt readability
fn run_explain_cmd(args: &args::ExplainArgs) -> Result<()> {
    let topics: &[(&str, &str, &str, &str)] = &[
        (
            "hotspot-score",
            "Tornhill 2018 — Software Design X-Rays",
            "percentile_rank(revisions) × percentile_rank(cognitive) × (100 − code_health) / 4. Range [0, 10].",
            "See analyses/hotspots.rs::SQL (file_revs + file_complexity + joined CTEs).",
        ),
        (
            "code-health",
            "code-health composite: biomarker structural risk (Complex/Large Method, God Class, DRY, Shotgun Surgery, Deep Nesting, Many Args, Complex Conditional) fused with behavioral signal (Nagappan & Ball 2005 churn + Mockus & Herbsleb 2002 ownership); coupling centrality enters once via the Shotgun Surgery biomarker (Tornhill 2018); self-relative percentile banding (Alves/Ypma/Visser 2010) plus an additive corpus-relative percentile when a calibration artifact is active",
            "100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv), where structural_risk is a weighted sum of biomarker intensities (complex-method 0.22, god-class 0.18, large-method 0.12, dry 0.12, shotgun-surgery 0.12, deep-nesting 0.10, many-args 0.07, complex-conditional 0.07); band from structural_risk thresholds (≥0.55 red, ≥0.28 yellow, else green); percentile = per-language PERCENT_RANK of structural_risk.",
            "See analyses/code_health.rs.",
        ),
        (
            "refactoring-targets",
            "effort-aware refactoring priority: (code-health structural_risk × hotspot_score) ÷ inspection effort, with a ManualUp baseline (Popt / PofB20 framing)",
            "priority = (structural_risk × hotspot_score) / max(loc, 25). Ranked DESC. `manual_up_rank` ranks the same files by ascending LOC (the \"inspect the small dense files first\" baseline the composite is meant to beat); `dominant_type` is the file's highest-intensity biomarker.",
            "See analyses/refactoring_targets.rs.",
        ),
        (
            "mi",
            "Coleman 1994 + SEI 1997",
            "171 − 5.2·log₂(V) − 0.23·CC − 16.2·log₂(SLOC) + 50·sin(√(2.4·comments%)). file-level `kind='unit'` entry.",
            "Surfaced by rust-code-analysis via codelore-rca.",
        ),
        (
            "coupling-density",
            "Newman 2010 §6.10 — graph density",
            "edges / (V·(V−1)/2) where V is the candidate node set (files with revs ≥ min_revs) and edges are Fisher-significant coupling pairs.",
            "See analyses/coupling.rs::density.",
        ),
        (
            "hotspots",
            "Tornhill 2015 + Bird et al. 2011",
            "Per-file behavioural risk surface: revisions × max(cognitive) × code-health composite. The flagship CodeLore analysis.",
            "See analyses/hotspots.rs.",
        ),
        (
            "hotspot-velocity",
            "Change-acceleration early warning (recent vs baseline churn)",
            "Per file: acceleration = recent_per_week − baseline_per_week over a 30-day recent window vs the 90 days before it, anchored at MAX(commits.date). Positive = heating up (becoming a hotspot before its all-time count shows it); negative = cooling down.",
            "See analyses/hotspot_velocity.rs.",
        ),
        (
            "god-classes",
            "Brown et al. 1998 *AntiPatterns* §3.1 + Riel 1996 *Object-Oriented Design Heuristics*",
            "(cognitive / 100.0) × (fan_in + fan_out). Ranks files where all three pull up.",
            "See analyses/god_classes.rs.",
        ),
        (
            "architecture-roles",
            "Baldwin, MacCormack & Rusnak 2014 — Hidden Structure (Research Policy 43:8)",
            "Per-file Core/Shared/Control/Periphery from transitive visibility fan-in (vfi) / fan-out (vfo) on the import graph: Core = the largest cyclic group; Shared = vfi≥core, vfo<core; Control = vfi<core, vfo≥core; Periphery = both below. reach_pct = vfo/n×100; mean(vfo/n) = MacCormack propagation cost.",
            "See analyses/architecture_roles.rs + analyses/import_graph.rs::reachability.",
        ),
        (
            "instability",
            "Martin 1994 — OO Design Quality Metrics (Clean Architecture 2017)",
            "Per file: afferent coupling ca (files importing it / in-degree), efferent coupling ce (files it imports / out-degree), instability I = ce/(ca+ce) in [0,1]. 0 = stable (depended-on, depends on nothing), 1 = unstable. Resolved import graph; Abstractness/Distance need symbol data and are out of scope.",
            "See analyses/instability.rs.",
        ),
        (
            "cycle-health",
            "behavioral heat + extraction candidate per import cycle",
            "Per non-trivial SCC of the resolved import graph: heat_pct = the cycle \
             members' share of repo LOC churn over the trailing --window-days window \
             (same anchoring as effort-exposure); verdict = live when at least one \
             member appears in a window commit (a zero-LOC touch still counts), fossil \
             otherwise; extract_candidate = the member whose trial removal minimises \
             the largest surviving SCC of the induced subgraph (Tarjan per member, ties \
             by fewest surviving cyclic nodes then lexicographic path); \
             predicted_pc_drop = whole-graph MacCormack propagation-cost delta if the \
             candidate were extracted. Trial removal and the prediction run only for \
             cycles of ≤ 64 members; above that bound the prediction is absent and the \
             candidate falls back to the highest in-cycle degree.",
            "See analyses/cycle_health.rs.",
        ),
        (
            "defect-validation",
            "Śliwerski, Zimmermann & Zeller 2005 (SZZ) + Kim et al. 2006 (AG-SZZ)",
            "Reads an own-repo defect-calibration artifact (built by `codelore \
             calibrate-defects`) and reports its evidence as flat (metric, value) \
             rows: the band table (share of defect-introducing changes that landed \
             in files red / yellow / green at the time), AUC and precision@k of \
             HEAD structural_risk against the defect-implicated file labels, mining \
             tallies, and the weight-tuning decision with both validation AUCs. \
             Association, not causation — a defect touching a red file is evidence \
             the score ranks it high, not proof the score caused the defect. Reads \
             the artifact only; without one it emits zero rows and a stderr hint.",
            "See analyses/defect_validation.rs + defect_calibration/.",
        ),
        (
            "architecture-metrics",
            "Lakos 1996 (CCD/ACD/NCCD) + MacCormack/Rusnak/Baldwin 2006/2014",
            "Repo-level (metric, value) rows: propagation_cost = density of the transitive-closure matrix; acd = mean transitive dependency set size; nccd = CCD / balanced-binary-tree CCD (<1 flat, >1 layered, >2 likely cyclic); dependency_cycles, largest_cycle; architecture_type = hierarchical / core-periphery / multi-core.",
            "See analyses/architecture_metrics.rs.",
        ),
        (
            "architecture-trend",
            "Architectural decay over the commit sequence",
            "Recomputes propagation cost, dependency-cycle count and largest tangle at up to 12 historical revs (evenly spaced across history), rebuilding the import graph in memory at each by reading + resolving source blobs at that rev. Shows whether structure is decaying and roughly when it started. Heavier than the SQL-only analyses (it re-parses source per sample); computed on demand, never cached.",
            "See analyses/architecture_trend.rs.",
        ),
        (
            "health-trend",
            "Repo health (architectural + code + combined) over the commit sequence",
            "Computes three 0-100 scores at up to 12 historical revs (evenly spaced): \
             architectural health (structural — propagation cost + dependency tangle), \
             code health (the rev-parameterized code-health engine with duplication \
             excluded, averaged over files), and their equal blend. Bands: green >= 70, \
             yellow 40-69, red < 40. Rebuilds the import graph + re-scans complexity per \
             sample, so it is heavier than SQL-only analyses; computed on demand, never \
             cached.",
            "See analyses/health_trend.rs.",
        ),
        (
            "effort-exposure",
            "Engineering effort distribution across code-health bands",
            "For each code-health band (red / yellow / green) reports the percentage of \
             files, SLOC, trailing-window commits, and LOC churn in that band. Answers \
             the hero KPI question: are we spending most effort fighting fires in red code \
             or extending healthy green code? Commit-share Wilson 95% CI is included per \
             band. Window anchors to the repo's last commit date (not wall-clock) via \
             --window-days (default 90).",
            "See analyses/effort_exposure.rs.",
        ),
        (
            "code-familiarity",
            "Decayed-knowledge familiarity score for the active team",
            "Computes what fraction of SLOC is actively known by current contributors \
             (authors with ≥1 commit in the trailing window). Uses exponentially-decayed \
             knowledge shares (Jabrayilzade et al., ICSE-SEIP 2022). Also reports islands \
             percentage: SLOC in files where one person holds ≥80% of knowledge with no \
             meaningful backup. Low familiarity or high islands percentage signals knowledge \
             risk. Verdict threshold configurable via [gates] code_familiarity_min in \
             .codelore-thresholds.toml (default 70.0).",
            "See analyses/code_familiarity.rs.",
        ),
        (
            "team-composition",
            "Contribution-span tenure buckets with behavioral veteran gate and onboarding velocity",
            "Buckets each author by contribution span (last − first commit): onboarded \
             (<90 d), experienced (90–364 d), veteran (≥365 d). Veterans who have not \
             touched a breadth of files comparable to the current 80%-core set are capped \
             at 'experienced' (veteran_breadth_ok = false). Also reports onboarding_weeks: \
             how many weeks from an author's first commit to their first week in the weekly \
             80%-core set. Authors whose first commit falls within the project's first 12 \
             weeks (founders) receive NULL for onboarding_weeks.",
            "See analyses/team_composition.rs.",
        ),
        (
            "coordination-needs",
            "Per-file coordination overhead: fragmentation, interleave, co-change entropy",
            "For each file reports: knowledge fragmentation (1 − HHI, 0 = single owner, \
             near 1 = evenly spread knowledge); author-switch interleave between adjacent \
             commits (0 = always same author, 1 = always alternating); and co-change graph \
             entropy contribution (EASE 2025, arXiv 2504.18511; window-scoped, commits \
             touching >30 files excluded). Tier: single (1 author) | low (frag<0.25) | \
             medium | high (frag≥0.50 AND interleave≥0.50). Joined with code-health band \
             so high-fragmentation files in the red band surface first.",
            "See analyses/coordination_needs.rs.",
        ),
        (
            "marginal-owner-risk",
            "Ownership concentration × code-health fusion: files where active authors have shallow familiarity",
            "For each file in the yellow or red health band, reports the maximum knowledge \
             share held by any author who committed within window_days. Risk tiers: high \
             (red band AND top active share <0.10); elevated ((red AND <0.30) OR (yellow \
             AND <0.10)). Rows that do not meet either threshold are excluded. The \
             ownership × code-quality interaction is correlational, not causal \
             (Palomba et al., EASE 2023, arXiv 2304.11636).",
            "See analyses/marginal_owner_risk.rs.",
        ),
        (
            "release-cadence",
            "Inter-release gap statistics from git tags (median, IQR, trend)",
            "Filters tags by --release-tag-glob (default 'v*'), then computes the \
             number of days between consecutive release tags. Emits one row per \
             matched tag (date, days_since_prev) plus a synthetic '__summary__' \
             row carrying the median gap, IQR, and a trend label: 'accelerating' \
             (negative OLS slope < −0.1 d/release), 'slowing' (slope > +0.1), or \
             'stable' (within ±0.1). Tags are proxies for releases, not \
             deployments; cadence reflects tagging discipline as much as actual \
             release velocity. First tag always has no predecessor gap.",
            "See analyses/release_cadence.rs.",
        ),
        (
            "delivery-metrics",
            "Repo-level delivery flow distributions: batch size, branch duration, rework, and lead-proxy (p50/p75/p90)",
            "Five percentile distributions over merge units and commits: batch_size_files \
             (distinct paths per merge), batch_size_loc (LOC churn per merge), \
             branch_duration_hours (merge date − earliest branch-side author date), \
             rework_pct (hunk-overlap within --rework-window-days, approximate), and \
             lead_proxy_hours (author→committer date gap, positive only, non-merge commits). \
             Requires commit_parents table (schema v4) and merges ingested with \
             include_merges=true. Branch metrics are unreliable on squash/rebase workflows \
             (emits a warning when merge count < 3 and commit count > 50).",
            "See analyses/delivery_metrics.rs.",
        ),
        (
            "function-xray",
            "Per-function change frequency for a single target file (Gall et al. ICSM 2003 HistoryFinder)",
            "Requires --target <repo-relative-path>. For each function/method alive at HEAD \
             in the target file, counts revisions where at least one hunk overlapped the \
             function's line span. Hunk-overlap attribution is more accurate than file-level \
             blame: it uses the span at change time. Pure deletions (new_lines=0) are attributed \
             to the function whose span contained the deleted anchor line. Sorted by change_freq \
             DESC.",
            "See analyses/function_xray.rs.",
        ),
        (
            "function-coupling",
            "Per-function-pair co-change frequency with Fisher significance for a single target file",
            "Requires --target <repo-relative-path>. For each pair of HEAD-alive functions in the \
             target file that co-changed (both touched in the same revision) in ≥2 revisions, \
             emits the pair with co-change count, per-function change counts, confidence \
             (co/min(a,b)), and two-tailed Fisher exact p-value. \
             Sorted by p-value ASC. Research: Adams et al. ICSM 2006.",
            "See analyses/function_coupling.rs.",
        ),
        (
            "cycle-origins",
            "Commit-level archaeology for dependency cycles",
            "For each dependency cycle at HEAD, binary-searches history (reading + resolving source at past revisions) to find the earliest commit where that cycle existed — the commit that closed the loop. Reports the forming commit's SHA + date per cycle. Assumes a cycle, once formed, stays formed; traces the largest cycles first to bound cost.",
            "See analyses/cycle_origins.rs.",
        ),
        (
            "dependency-cycles",
            "Tarjan 1972 SCC + Fontana et al. 2017 (Arcan) Cyclic Dependency smell",
            "Non-trivial strongly-connected components (size ≥ 2) of the structural import graph — files that import each other transitively. cycle_id groups a tangle; size is its member count. Accuracy follows the import resolver's language coverage.",
            "See analyses/dependency_cycles.rs + analyses/import_graph.rs.",
        ),
        (
            "modularity-violations",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8) + Baldwin/MacCormack 2014 hidden structure",
            "Fisher-significant co-change pairs (from coupling) with NO directed dependency path between them (transitive reachability, either direction) — implicit cross-module dependencies. Ranked by coupling degree. Accuracy follows the import resolver's language coverage.",
            "See analyses/modularity_violations.rs.",
        ),
        (
            "unstable-interface",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8)",
            "revisions × coupled_dependents, gated on fan_in ≥ 3 and revisions ≥ min_revs. A widely-imported file that changes often and co-changes with its dependents, so its instability propagates.",
            "See analyses/unstable_interface.rs.",
        ),
        (
            "crossing",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8)",
            "A structural 'X' — fan_in ≥ 3 AND fan_out ≥ 3 — that co-changes with ≥1 importer AND ≥1 import, coupling upstream and downstream through itself. crossing_score = coupled_upstream + coupled_downstream.",
            "See analyses/crossing.rs.",
        ),
        (
            "bus-factor",
            "Filatov 2010 (commits mode) / Cury & Avelino SBES'24 (doe mode)",
            "Min number of authors whose departure would leave a module unmaintained. \
             Default mode (--knowledge-model commits): smallest set covering ≥80% of \
             module commits (Filatov 2010). DOE mode (--knowledge-model doe): greedy \
             truck-factor procedure — repeatedly remove the author expert on the most \
             remaining files until >50% of files have no expert (Cury & Avelino, \
             SBES'24 arXiv 2408.08733). DOE mode emits the same per-module row shape \
             with model='doe'.",
            "See analyses/bus_factor.rs.",
        ),
        (
            "stale-code",
            "code-age follow-up + Sonar 'trivial' threshold",
            "Files alive at HEAD AND untouched ≥12 months AND max(cognitive) ≤ 5. Intersection minimises false positives.",
            "See analyses/stale_code.rs.",
        ),
        (
            "pair-programming",
            "Co-Authored-By trailer convention (GitHub 2017)",
            "Counts commits where ≥1 `Co-Authored-By:` trailer present, by unique author pair.",
            "See analyses/pair_programming.rs.",
        ),
        (
            "lead-time",
            "DORA 2018 Accelerate",
            "Seconds between commit author-date and committer-date (proxy for in-flight review time). Schema_v3 carries only committer-date; schema_v4 will add author-date for real values.",
            "See analyses/lead_time.rs.",
        ),
        (
            "knowledge-islands",
            "T8 design + Bird et al. 2011 risk-author",
            "Per-file bus-factor risk: primary author hasn't committed in `--departed-threshold-days` days AND no substantial other owners.",
            "See analyses/knowledge_islands.rs.",
        ),
        (
            "communities",
            "Leiden algorithm (Traag, Waltman, van Eck 2019)",
            "Modularity-optimising community detection on the Fisher-significant coupling graph. Surfaces Conway's-law clusters.",
            "See analyses/communities.rs.",
        ),
        (
            "centrality",
            "Newman 2010 §7",
            "Per-file degree, weighted-degree, and PageRank on the Fisher-significant coupling graph.",
            "See analyses/centrality.rs.",
        ),
        (
            "architecture-violations",
            "Layered architecture rules (Buschmann et al. 1996)",
            "Imports that cross a forbidden layer boundary per `.codelore-arch-rules.toml`. Empty rule set → empty output.",
            "See arch_rules/mod.rs + analyses/arch_violations.rs.",
        ),
        (
            "kamei-risk",
            "Kamei et al. 2013 (Just-In-Time Software Defect Prediction)",
            "Per-commit 14-feature vector (la, ld, nf, nd, ns, entropy, fix, ndev, age, nuc, exp, rexp, sexp, lt). Composite risk dimension explanation in the SPA's Delivery Risk Sparkline.",
            "See output/spa.rs::run_kamei_risk + facts/schema_v1.sql commits table.",
        ),
        (
            "revisions",
            "Nagappan & Ball 2005 — relative churn predicts defect density",
            "COUNT(rev) per file — distinct commits touching the path. Gated by --min-revs, ordered by n-revs descending.",
            "See analyses/revisions.rs.",
        ),
        (
            "authors",
            "Bird, Nagappan, Murphy, Devanbu & Zeller 2011 — \"Don't Touch My Code!\"",
            "Distinct canonical authors per file (n-authors = COUNT(DISTINCT author)), split into human vs bot via .mailmap + bot/AI attribution; n-revs = Σ per-author commit counts.",
            "See analyses/authors.rs.",
        ),
        (
            "ownership",
            "Mockus & Herbsleb 2002 + Hirschman 1980 (Herfindahl–Hirschman index)",
            "Fractal Value = 1 − Σᵢ (aᵢ / nc)², where aᵢ is author i's commit count on the file and nc the file's total commits. 0 = single owner, → 1 = fragmented. main-author = author with the most revisions.",
            "See analyses/ownership.rs.",
        ),
        (
            "code-age",
            "Tornhill 2015 — Your Code as a Crime Scene (software half-life)",
            "Whole calendar months between the file's latest commit (at-or-before the --age-time-now anchor, default now) and the anchor: 12·(yr−yr) + (mo−mo) − 1 if the anchor day-of-month is earlier than the last-commit day. Only files live at the anchor.",
            "See analyses/code_age.rs.",
        ),
        (
            "soc",
            "Tornhill 2018 — Software Design X-Rays (Sum of Coupling)",
            "Σ (commit_size − 1) over every commit the file appears in; a solo commit contributes 0. Per-file centrality across the change-coupling graph. Gated by --min-soc.",
            "See analyses/soc.rs.",
        ),
        (
            "abs-churn",
            "Nagappan & Ball 2005 — relative code churn predicts defect density",
            "Per calendar day across the repo: SUM(lines added), SUM(lines deleted), COUNT(commits). The absolute-churn time series.",
            "See analyses/churn.rs.",
        ),
    ];
    match &args.topic {
        None => {
            println!("Supported topics:");
            for (name, _, _, _) in topics {
                println!("  {name}");
            }
            println!("\nUsage: codelore explain <topic>");
            Ok(())
        }
        Some(topic) => {
            let found = topics.iter().find(|(n, ..)| n.eq_ignore_ascii_case(topic));
            match found {
                Some((name, citation, formula, source)) => {
                    println!("# {name}\n");
                    println!("**Citation**\n  {citation}\n");
                    println!("**Formula**\n  {formula}\n");
                    println!("**Source**\n  {source}\n");
                    println!(
                        "**Foundations**\n  See docs/research-foundations.md for the full citation chain."
                    );
                    Ok(())
                }
                None => match resolve_explain_file(&args.repo, topic) {
                    Some(repo_relative) => run_explain_file(args, &repo_relative),
                    None => Err(CodeLoreError::Analysis(format!(
                        "unknown topic `{topic}` — run `codelore explain` (no arg) to list \
                         supported topics, or pass an existing file path (with --repo) to print \
                         that file's evidence dossier"
                    ))
                    .into()),
                },
            }
        }
    }
}

/// Resolve an `explain` argument that missed the topic table to a repo-relative
/// source-file path, or `None` when it names no existing file.
///
/// The argument is joined onto `--repo`; `Path::join` lets an absolute argument
/// replace the repo, so a repo-relative `src/x.rs`, a `--repo`-prefixed path,
/// and an absolute path to the same file all resolve to the same target. The
/// fact store keys on repo-relative, forward-slash paths, so the resolved path
/// is made relative to `--repo` and its separators are normalized to `/`.
fn resolve_explain_file(repo: &std::path::Path, arg: &str) -> Option<String> {
    let candidate = repo.join(arg);
    if !candidate.is_file() {
        return None;
    }
    let relative = match candidate.strip_prefix(repo) {
        Ok(stripped) => stripped.to_path_buf(),
        Err(_) => std::path::PathBuf::from(arg),
    };
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Print the deterministic evidence dossier for a repo-relative source file,
/// and — with `--llm` — an advisory grounded narrative plus its citation-check
/// stamp.
///
/// This surface is strictly read-only: it opens (or ingests) the fact store and
/// assembles a fact sheet from the same analyses the CLI already exposes, never
/// touching an analysis row, a gate verdict, or a provenance manifest. Analysis
/// `min_revs` is forced to 1 so any single named file can be explained — the
/// default corpus gate would otherwise hide most files from their own dossier.
/// That 1-revision floor also applies to the dossier's hotspot, coupling, and
/// ownership sections, so their numbers can differ from a default `analyze` run
/// that gates low-revision files out.
///
/// Without `--llm`, when this file's own previously generated narrative exists
/// for a now-changed fact sheet, a one-line staleness note is printed. With
/// `--llm`, a missing LLM configuration is a hard error carrying a setup hint.
fn run_explain_file(args: &args::ExplainArgs, repo_relative: &str) -> Result<()> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::enrichment::client::{LlmEnv, resolve_client};
    use codelore_lib::cli_api::enrichment::fact_sheet::FileFactSheet;
    use codelore_lib::cli_api::enrichment::prompt::Lens;
    use codelore_lib::cli_api::enrichment::{cache, engine};

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);
    let defect_calibration = codelore_lib::cli_api::quality_gates::resolve_defect_calibration(
        args.defect_calibration.clone(),
        &args.repo,
    )
    .context("resolve defect calibration")?;
    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: 1,
        defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        ..Options::default()
    };
    let repo = GixRepo::open(&args.repo)
        .with_context(|| format!("open git repo at {}", args.repo.display()))?;
    let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
        .context("open or ingest the fact store")?;
    let sheet = FileFactSheet::build(&db, &repo, &opts, repo_relative)
        .with_context(|| format!("build the evidence dossier for {repo_relative}"))?;

    print!("{}", sheet.to_human_text());

    if args.llm {
        let client = resolve_client(&LlmEnv::from_process_env()).context(
            "configure an LLM endpoint — set CODELORE_LLM_MODEL for a local OpenAI-compatible \
             runner (e.g. a model from `ollama list`), or ANTHROPIC_API_KEY for Anthropic; see \
             the CODELORE_LLM_* variables in the docs",
        )?;
        let canonical = sheet.to_canonical_text();
        let values = sheet.numeric_values();
        let result = engine::narrate(
            client.as_ref(),
            Lens::FileDiagnosis,
            repo_relative,
            engine::SheetFacts {
                text: &canonical,
                values: &values,
            },
            &cache_root,
            &args.repo,
            args.llm_refresh,
        )
        .context("generate the advisory narrative")?;
        println!("\n{}", result.narrative);
        println!("{}", engine::stamp(&result));
    } else if let Some(latest) = cache::latest_for_subject(&cache_root, &args.repo, repo_relative)
        && latest.fact_digest != sheet.digest()
    {
        println!("note: cached narrative is stale — evidence changed; re-run with --llm");
    }

    Ok(())
}

/// JSON Schema export. The CLI surfaces the row-type catalogue and
/// emits a minimal envelope per type today; full `JsonSchema` derive
/// on every row type is a planned enhancement (~80 LOC of
/// `#[derive(JsonSchema)]` adds across the analyses module) and
/// will populate the `items` shape once `schemars` derive lands.
fn run_schema_cmd(args: &args::SchemaArgs) -> Result<()> {
    let row_types: Vec<&str> = AnalysisName::all().iter().map(|a| a.as_str()).collect();
    match &args.row_type {
        None => {
            println!("Supported row types ({}):", row_types.len());
            for name in &row_types {
                println!("  {name}");
            }
            println!(
                "\nUsage: codelore schema <row-type>\n\nNote: today's emitter ships the row-type catalogue and a minimal envelope. The full JSON Schema documents populate the `items` shape once `schemars` derive is applied to every analyses/* row type."
            );
            Ok(())
        }
        Some(name) => {
            if row_types.contains(&name.as_str()) {
                println!(
                    "{{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"$id\": \"https://codelore.dev/schemas/{name}.json\",\n  \"title\": \"{name}\",\n  \"type\": \"array\",\n  \"items\": {{\n    \"$comment\": \"Full row-shape schema populates once schemars derive is applied.\"\n  }}\n}}"
                );
                Ok(())
            } else {
                Err(CodeLoreError::Analysis(format!(
                    "unknown row type `{name}` — run `codelore schema` (no arg) to list supported row types"
                ))
                .into())
            }
        }
    }
}

fn run_diff_cmd(args: &DiffArgs) -> Result<()> {
    let (output, head_db, head_opts) = diff::run_diff(args).context("codelore diff")?;

    // Advisory LLM narrative. Best-effort and format-scoped: it is produced only
    // for `text`/`markdown` with `--llm`, any failure degrades to a stderr
    // warning, and it never touches the deterministic output below or the
    // `should_fail` exit code.
    let format = args.format.as_str();
    let narrative: Option<(String, String)> = if args.llm {
        match format {
            "text" | "markdown" => diff_llm_narrative(args, &output),
            _ => {
                eprintln!("note: --llm applies to text/markdown output only; ignored for {format}");
                None
            }
        }
    } else {
        None
    };

    let mut out: Box<dyn Write> = match args.output.as_ref() {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    diff_output::emit(
        &mut out,
        &output,
        format,
        &args.repo,
        Some((&head_db, &head_opts)),
        narrative,
    )?;
    drop(out);

    if diff::should_fail(args, &output) {
        // Per spec §6.6: analysis-failure exit code is 4.
        std::process::exit(4);
    }
    Ok(())
}

/// Produce the advisory LLM narrative for a diff run, degrading gracefully.
///
/// The `DiffOutput` is flattened into a deterministic [`DiffFactSheet`], a chat
/// client is resolved from the `CODELORE_LLM_*` environment, and the narrative
/// is generated under [`Lens::DiffNarrative`] with a citation-check stamp. Any
/// failure — no endpoint configured, network error, or narration error — is
/// reported on stderr and yields `None`, so the caller's deterministic output
/// and exit code stay untouched. The narrative cache lives under the default
/// cache root (diff has no `--cache-dir`).
fn diff_llm_narrative(args: &DiffArgs, output: &diff::DiffOutput) -> Option<(String, String)> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::enrichment::client::{LlmEnv, resolve_client};
    use codelore_lib::cli_api::enrichment::engine;
    use codelore_lib::cli_api::enrichment::fact_sheet::DiffFactSheet;
    use codelore_lib::cli_api::enrichment::prompt::Lens;

    let sheet = DiffFactSheet::from_sections(diff_fact_sections(output));
    let canonical = sheet.to_canonical_text();
    let values = sheet.numeric_values();
    let cache_root = default_cache_root();

    let result = resolve_client(&LlmEnv::from_process_env()).and_then(|client| {
        engine::narrate(
            client.as_ref(),
            Lens::DiffNarrative,
            "diff",
            engine::SheetFacts {
                text: &canonical,
                values: &values,
            },
            &cache_root,
            &args.repo,
            args.llm_refresh,
        )
    });
    match result {
        Ok(result) => Some((result.narrative.clone(), engine::stamp(&result))),
        Err(e) => {
            eprintln!("warning: llm narrative unavailable: {e}");
            None
        }
    }
}

/// Flatten a `DiffOutput` into ordered fact-sheet sections for the advisory diff
/// narrative. Only sections with data are emitted, and every numeric value is
/// rendered through the shared [`fmt_num`] formatter so the narrative's citation
/// check can match each quoted number back to a fact.
fn diff_fact_sections(output: &diff::DiffOutput) -> Vec<(String, Vec<(String, String)>)> {
    use codelore_lib::cli_api::enrichment::fact_sheet::fmt_num;

    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();

    // verdict — change-level health ratio, verdict, and change counts.
    if let Some(dh) = &output.delta_health {
        let mut facts = vec![("verdict".to_string(), dh.verdict.clone())];
        if let Some(ratio) = dh.ratio {
            facts.push(("ratio".to_string(), fmt_num(ratio)));
        }
        facts.push(("added".to_string(), dh.counts.added.to_string()));
        facts.push(("modified".to_string(), dh.counts.modified.to_string()));
        facts.push(("removed".to_string(), dh.counts.removed.to_string()));
        facts.push(("skipped".to_string(), dh.counts.skipped.to_string()));
        sections.push(("verdict".to_string(), facts));
    }

    // gates — [diff] quality-gate violations.
    if !output.gate_violations.is_empty() {
        let mut facts = Vec::new();
        for (i, v) in output.gate_violations.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.gate"), v.gate.clone()));
            facts.push((format!("{n}.path"), v.path.clone()));
            facts.push((format!("{n}.actual"), v.actual.clone()));
            facts.push((format!("{n}.threshold"), v.threshold.clone()));
        }
        sections.push(("gates".to_string(), facts));
    }

    // entrants — files newly entering the top-N hotspot list.
    if !output.hotspots.rank_entrants.is_empty() {
        let mut facts = Vec::new();
        for (i, h) in output.hotspots.rank_entrants.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.path"), h.path.clone()));
            facts.push((format!("{n}.hotspot_score"), fmt_num(h.hotspot_score)));
            facts.push((format!("{n}.revisions"), h.revisions.to_string()));
            facts.push((format!("{n}.cognitive"), fmt_num(h.cognitive)));
            facts.push((format!("{n}.code_health"), fmt_num(h.code_health)));
        }
        sections.push(("entrants".to_string(), facts));
    }

    // score-increased — existing hotspots whose score grew past the threshold.
    if !output.hotspots.score_increased.is_empty() {
        let mut facts = Vec::new();
        for (i, s) in output.hotspots.score_increased.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.path"), s.path.clone()));
            facts.push((format!("{n}.base_score"), fmt_num(s.base_score)));
            facts.push((format!("{n}.head_score"), fmt_num(s.head_score)));
            facts.push((format!("{n}.delta"), fmt_num(s.delta)));
        }
        sections.push(("score-increased".to_string(), facts));
    }

    // absences — historically-coupled files omitted from the PR.
    if !output.coupling_absences.is_empty() {
        let mut facts = Vec::new();
        for (i, a) in output.coupling_absences.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.touched_file"), a.touched_file.clone()));
            facts.push((format!("{n}.expected_partner"), a.expected_partner.clone()));
            facts.push((
                format!("{n}.historical_coupling"),
                fmt_num(a.historical_coupling),
            ));
            facts.push((format!("{n}.fisher_p"), fmt_num(a.fisher_p)));
            facts.push((
                format!("{n}.historical_shared_revs"),
                a.historical_shared_revs.to_string(),
            ));
        }
        sections.push(("absences".to_string(), facts));
    }

    // clones — new clone-family members introduced by the PR.
    if !output.clones.new_families.is_empty() {
        let mut facts = Vec::new();
        for (i, c) in output.clones.new_families.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.clone_group_id"), c.clone_group_id.to_string()));
            facts.push((format!("{n}.entity"), c.entity.clone()));
            facts.push((format!("{n}.function"), c.function.clone()));
            facts.push((format!("{n}.start_line"), c.start_line.to_string()));
            facts.push((format!("{n}.end_line"), c.end_line.to_string()));
            facts.push((format!("{n}.node_count"), c.node_count.to_string()));
        }
        sections.push(("clones".to_string(), facts));
    }

    sections
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
