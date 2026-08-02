//! `codelore gate` — working-tree quality-gate evaluation.
//!
//! Projects the uncommitted working-tree edits onto code health and the import
//! graph vs HEAD (the change-set engine), evaluates the working-tree `[diff]`
//! gates against that projection, records each verdict in the gate-run ledger,
//! and exits 0 (pass) or 1 (fail) — the same exit contract as `codelore check`.
//! Writes `result=pass|fail` to `$GITHUB_OUTPUT` for GitHub Actions consumption.

use std::io::Write as _;

use anyhow::{Context, Result};
use codelore_lib::cli_api::Options;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::{GixRepo, Repo as _};

use crate::args::{self, GateFormat};
use crate::{GATE_DELTA_TABLE_ROWS, GATE_FINDINGS_ROWS, write_github_output};

/// Working-tree quality gate. Projects what the uncommitted edits do to code
/// health and the import graph vs HEAD (the change-set engine), evaluates the
/// working-tree `[diff]` gates against the projection, and exits 0 (pass) or
/// 1 (fail) — the same exit contract as `codelore check`. Writes
/// `result=pass|fail` to `$GITHUB_OUTPUT` for direct GitHub Actions
/// step-output consumption.
pub(crate) fn run_gate_cmd(args: &args::GateArgs) -> Result<()> {
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
    let head_sha = repo.head_sha().context("get HEAD sha")?;
    let db =
        FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root).context("ingest")?;
    // Witness the ingest before evaluating any gate: a real HEAD over an
    // empty commit store is the truncated-checkout signature (see
    // `check.rs`). `gate` has no `--after`/`--before` walk filter, so an
    // empty store is unambiguously the shallow-checkout case, never a
    // legitimate date-window skip.
    db.ensure_ingest_witnessed(&head_sha)?;

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
/// merge-in-progress note, the skip notice when `delta_code_health_min` is
/// configured but a whole-repo code-health median is unavailable on either
/// side (no scoreable files), and the same skip notice for the three
/// change-set-scoped gates when `report.changes` itself is empty — nothing
/// was measured, so the run reports skipped rather than a silent pass.
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
    if report.changes.is_empty() {
        if thresholds.diff.delta_code_health_min_per_file.is_some() {
            eprintln!("  ⚠ delta_code_health_min_per_file: skipped — no files in the change-set");
        }
        if thresholds.diff.new_file_health_min.is_some() {
            eprintln!("  ⚠ new_file_health_min: skipped — no files in the change-set");
        }
        if thresholds.diff.no_new_cycles {
            eprintln!("  ⚠ no_new_cycles: skipped — no files in the change-set");
        }
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
    // Propagating `writeln!` so this (potentially large) document survives an
    // early pipe close as a quiet exit rather than a print-macro panic.
    let rendered = serde_json::to_string_pretty(&doc).context("render gate JSON")?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{rendered}").context("write gate JSON")?;
    Ok(())
}

/// Print the advisory (non-verdict) text sections to stdout: one line per
/// finding (capped at [`GATE_FINDINGS_ROWS`] with a `(+n more findings)`
/// tail), then the per-file delta table in the engine's order (|delta|
/// descending, unscored rows last). Suppressed under `--quiet`.
fn render_gate_advisories(
    args: &args::GateArgs,
    report: &codelore_lib::change_set::ChangeSetReport,
) -> Result<()> {
    if args.quiet {
        return Ok(());
    }
    // Propagating `writeln!` over a locked stdout so this per-row advisory table
    // survives an early pipe close (`codelore gate | head`) as a quiet exit.
    let mut out = std::io::stdout().lock();
    for f in report.findings.iter().take(GATE_FINDINGS_ROWS) {
        writeln!(out, "[{}] {}: {}", f.kind, f.path, f.detail).context("write gate advisory")?;
    }
    let hidden_findings = report.findings.len().saturating_sub(GATE_FINDINGS_ROWS);
    if hidden_findings > 0 {
        writeln!(out, "(+{hidden_findings} more findings)").context("write gate advisory")?;
    }
    for d in report.health.deltas.iter().take(GATE_DELTA_TABLE_ROWS) {
        match (d.baseline_score, d.projected_score, d.delta) {
            (Some(b), Some(p), Some(delta)) => {
                writeln!(out, "{}  {b:.1} → {p:.1}  ({delta:+.1})", d.path)
                    .context("write gate advisory")?;
            }
            _ => writeln!(
                out,
                "{}  — {}",
                d.path,
                d.reason.as_deref().unwrap_or("not scored")
            )
            .context("write gate advisory")?,
        }
    }
    let hidden = report
        .health
        .deltas
        .len()
        .saturating_sub(GATE_DELTA_TABLE_ROWS);
    if hidden > 0 {
        writeln!(out, "(+{hidden} more files)").context("write gate advisory")?;
    }
    Ok(())
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
            render_gate_advisories(args, report)?;
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
        render_gate_advisories(args, report)?;
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
    use codelore_lib::cli_api::quality_gates::change_set_gate_verdict;
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
            change_set_gate_verdict(report, count("delta_code_health_min_per_file")),
        ));
    }
    if let Some(min) = d.new_file_health_min {
        records.push(rec(
            "new_file_health_min",
            min,
            count_f64("new_file_health_min"),
            change_set_gate_verdict(report, count("new_file_health_min")),
        ));
    }
    if d.no_new_cycles {
        records.push(rec(
            "no_new_cycles",
            0.0,
            count_f64("no_new_cycles"),
            change_set_gate_verdict(report, count("no_new_cycles")),
        ));
    }
    records
}
