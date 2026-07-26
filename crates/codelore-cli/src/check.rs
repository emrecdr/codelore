//! `codelore check` — repository quality-gate evaluation.
//!
//! Loads the configured thresholds, runs the gate-relevant analyses against the
//! repository HEAD, evaluates every configured gate, records each verdict in the
//! gate-run ledger, optionally emits a SARIF document, and exits 0 (pass) or 1
//! (fail). Also serves `--history` (print the ledger without evaluating) and
//! `--ratchet` (compare against the stored regression snapshot).

use anyhow::{Context, Result};

use crate::args::{self, CheckFormat};
use crate::{notice_corpus_lens_absent, write_github_output};

/// Quality-gate check. Loads thresholds, runs the hotspots analysis
/// against the repo, evaluates each row against the gates, and
/// exits 0 (pass) or 1 (fail). Writes `result=pass|fail` to
/// `$GITHUB_OUTPUT` for direct GitHub Actions step-output
/// consumption.
#[allow(clippy::too_many_lines)]
pub(crate) fn run_check_cmd(args: &args::CheckArgs) -> Result<()> {
    use codelore_lib::cli_api::Options;
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::facts::FactsDb;
    use codelore_lib::cli_api::quality_gates::Thresholds;
    use codelore_lib::cli_api::quality_gates::ledger::{
        GateRunRecord, append_gate_runs, format_history, now_utc_ts, read_gate_runs,
    };
    use codelore_lib::cli_api::quality_gates::ratchet::{
        RatchetMetrics, RatchetOutcome, evaluate_ratchet, format_ratchet_outcome, read_snapshot,
        snapshot_from_metrics, write_snapshot,
    };
    use codelore_lib::cli_api::repo::{GixRepo, Repo as _};

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    // --history: print ledger without running any gate evaluations. Write the
    // (potentially many-row) table through a propagating `write!` rather than
    // `print!` so a reader closing the pipe early (`codelore check --history |
    // head`) routes the BrokenPipe up to `main`'s quiet-exit arm, not a panic.
    if args.history {
        use std::io::Write as _;
        let records = read_gate_runs(&cache_root, &args.repo).context("read gate-run ledger")?;
        let mut out = std::io::stdout().lock();
        write!(out, "{}", format_history(&records, 20)).context("write gate-run history")?;
        return Ok(());
    }

    let thresholds = if let Some(path) = &args.thresholds_file {
        Thresholds::from_path(path).context("load thresholds file")?
    } else {
        Thresholds::discover(&args.repo).context("discover thresholds file")?
    };

    if thresholds.is_empty() && !args.ratchet {
        if !args.quiet {
            eprintln!(
                "codelore check: no thresholds configured (no `.codelore-thresholds.toml` at repo root); vacuously passing."
            );
        }
        write_github_output("result", "pass");
        // A vacuous pass under `--format sarif` must still emit a valid
        // zero-result SARIF document to stdout — the documented upload-sarif
        // pipeline (docs/advanced-usage.md §11.8) breaks if a run prints
        // nothing. Reuse the check emitter with an empty violation set.
        if matches!(args.format, CheckFormat::Sarif) {
            let repo = GixRepo::open(&args.repo).context("open repo")?;
            let head_sha = repo.head_sha().context("get HEAD sha")?;
            emit_check_sarif(
                &args.repo,
                &head_sha,
                &[],
                &std::collections::HashMap::new(),
            )?;
        }
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
        calibration: args.calibration.clone(),
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
    let ts = now_utc_ts();

    // Open the external-findings sidecar without creating it, only when it
    // holds findings. `None` when absent OR present-but-empty — both mean "no
    // external findings to evaluate", and the gate is skipped gracefully inside
    // evaluate_all_gates.
    let external_store = if thresholds.gates.max_findings_in_hot_files.is_some() {
        codelore_lib::cli_api::external::ExternalStore::open_nonempty(&cache_root, &args.repo)
            .context("open external store")?
    } else {
        None
    };

    let (violations, mut ledger_records, hotspot_count, code_health) = evaluate_all_gates(
        &thresholds,
        &db,
        &opts,
        &head_sha,
        &ts,
        external_store.as_ref(),
    )
    .context("evaluate gates")?;

    // ── Per-gate notices (stderr) ────────────────────────────────────────────
    // Rendered from the ledger records the evaluator already produced, so the
    // compute layer prints nothing itself. Emitted before the ratchet/report
    // branches so both paths surface them; stderr keeps stdout a clean SARIF
    // document in --format sarif; suppressed under --quiet.
    if !args.quiet {
        emit_gate_notices(&ledger_records);
    }
    // One-per-run hint when the corpus lens is inactive — the check path always
    // computes code-health rows, which carry no corpus_percentile without an
    // artifact. Suppressed under --quiet (handled inside).
    notice_corpus_lens_absent(&opts, args.quiet);

    // ── Ratchet ───────────────────────────────────────────────────────────────
    if args.ratchet {
        // Build ratchet metrics from already-computed gate outputs.
        let worst_health = code_health
            .iter()
            .map(|r| r.score)
            .fold(f64::INFINITY, f64::min);
        // red_effort_pct: read from the effort-exposure ledger record if present.
        let red_effort_pct = ledger_records
            .iter()
            .find(|r| r.gate == "max_red_effort_pct")
            .map(|r| r.value);
        // dependency_cycles: read from the arch ledger record if present.
        let dep_cycles = ledger_records
            .iter()
            .find(|r| r.gate == "max_dependency_cycles")
            .map(|r| r.value);
        let metrics = RatchetMetrics {
            code_health_min_observed: if worst_health.is_infinite() {
                None
            } else {
                Some(worst_health)
            },
            red_effort_pct_observed: red_effort_pct,
            dependency_cycles_observed: dep_cycles,
        };

        match read_snapshot(&args.repo).context("read ratchet snapshot")? {
            None => {
                // First run: initialize.
                let snap = snapshot_from_metrics(&metrics);
                write_snapshot(&args.repo, &snap).context("write ratchet snapshot")?;
                let tracked: Vec<&str> = [
                    metrics
                        .code_health_min_observed
                        .map(|_| "code_health_min_observed"),
                    metrics
                        .red_effort_pct_observed
                        .map(|_| "red_effort_pct_observed"),
                    metrics
                        .dependency_cycles_observed
                        .map(|_| "dependency_cycles_observed"),
                ]
                .into_iter()
                .flatten()
                .collect();
                emit_ratchet_message(
                    args,
                    &format!(
                        "✅ ratchet initialized — tracking {} metric(s): {}. \
                         Configure max_red_effort_pct / max_dependency_cycles gates to ratchet \
                         effort and cycles. Commit `.codelore-ratchet.toml` to enable regression detection.\n",
                        tracked.len(),
                        if tracked.is_empty() {
                            "(none)".to_owned()
                        } else {
                            tracked.join(", ")
                        },
                    ),
                );
                ledger_records.push(GateRunRecord {
                    ts: ts.clone(),
                    head_sha: head_sha.clone(),
                    gate: "ratchet".into(),
                    threshold: 0.0,
                    value: 0.0,
                    verdict: "initialized".into(),
                    mode: "ratchet".into(),
                });
                append_gate_runs(&cache_root, &args.repo, &ledger_records);
                // `--format sarif` must still yield a valid document on the
                // ratchet exit paths, exactly like the non-ratchet path — the
                // gates already ran, so emit their violations before returning.
                emit_check_sarif_when_requested(args, &db, &opts, &head_sha, &violations)?;
                return Ok(());
            }
            Some(snap) => {
                let outcome = evaluate_ratchet(&snap, &metrics);
                emit_ratchet_message(args, &format_ratchet_outcome(&outcome));
                let (verdict, ratchet_failed) = match &outcome {
                    RatchetOutcome::Improved { .. } => ("improved", false),
                    RatchetOutcome::Regressed { .. } => ("regressed", true),
                };
                ledger_records.push(GateRunRecord {
                    ts: ts.clone(),
                    head_sha: head_sha.clone(),
                    gate: "ratchet".into(),
                    threshold: 0.0,
                    value: 0.0,
                    verdict: verdict.into(),
                    mode: "ratchet".into(),
                });
                append_gate_runs(&cache_root, &args.repo, &ledger_records);
                // Emit the standard check SARIF on both ratchet outcomes before
                // returning/bailing, so `--format sarif` always yields a valid
                // document. Exit code is unchanged: a regression still bails.
                emit_check_sarif_when_requested(args, &db, &opts, &head_sha, &violations)?;
                if ratchet_failed {
                    anyhow::bail!("ratchet: regression detected — see above");
                }
                // Tighten: rewrite snapshot with improved values.
                let tightened = snapshot_from_metrics(&metrics);
                write_snapshot(&args.repo, &tightened).context("tighten ratchet snapshot")?;
                return Ok(());
            }
        }
    }

    // ── Ledger write (IO errors warn, never alter exit code) ─────────────────
    append_gate_runs(&cache_root, &args.repo, &ledger_records);

    // ── SARIF emission (when --format sarif) ─────────────────────────────────
    emit_check_sarif_when_requested(args, &db, &opts, &head_sha, &violations)?;

    // ── Report ────────────────────────────────────────────────────────────────
    let degraded_count = ledger_records
        .iter()
        .filter(|r| r.verdict == "degraded")
        .count();

    if violations.is_empty() {
        if degraded_count > 0 {
            let warning = format!(
                "⚠ codelore check: WARNING — {degraded_count} gate(s) degraded (non-degraded gates pass)"
            );
            // SARIF mode keeps stdout a clean SARIF document, so the warning
            // goes to stderr; text mode prints it to stdout with the report.
            if matches!(args.format, CheckFormat::Sarif) {
                eprintln!("{warning}");
            } else {
                println!("{warning}");
            }
        } else if matches!(args.format, CheckFormat::Text) {
            println!("✅ codelore check: PASS ({hotspot_count} files evaluated)");
        }
        write_github_output("result", "pass");
        write_github_output("violations", "0");
        Ok(())
    } else {
        eprintln!(
            "❌ codelore check: FAIL — {} violation(s)",
            violations.len()
        );
        if !args.quiet && matches!(args.format, CheckFormat::Text) {
            for v in &violations {
                eprintln!(
                    "  - {gate}: {path} — actual {actual} vs threshold {threshold}",
                    gate = v.gate,
                    path = v.path,
                    actual = v.actual,
                    threshold = v.threshold,
                );
            }
        }
        write_github_output("result", "fail");
        write_github_output("violations", &violations.len().to_string());
        // Inside GitHub Actions, emit each violation as an inline `::error`
        // annotation so the failing gate shows up against the file in the
        // PR's Files-changed view — not just as a red check.
        if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && matches!(args.format, CheckFormat::Text)
        {
            let mut stdout = std::io::stdout();
            codelore_lib::cli_api::output::gha::write_gate_violations_gha(&violations, &mut stdout)
                .context("emit gate annotations")?;
        }
        // Plain anyhow::bail carries no CodeLoreError, so main()'s chain-walk
        // falls through to the default exit code 1. Gate failure exits 1 by
        // design; typed CodeLoreError variants are reserved for repo/output
        // failures (exit codes 3, 4, 5).
        anyhow::bail!("{} gate violation(s) — see above", violations.len());
    }
}

/// Emit the per-gate skip / degraded notices to stderr, derived from the
/// ledger records the evaluator produced. Keeping this out of the compute layer
/// means `evaluate_all_gates` stays print-free and the notice wording lives
/// beside the rest of `run_check_cmd`'s reporting.
fn emit_gate_notices(
    ledger_records: &[codelore_lib::cli_api::quality_gates::ledger::GateRunRecord],
) {
    for r in ledger_records {
        match (r.gate.as_str(), r.verdict.as_str()) {
            ("max_findings_in_hot_files", "skipped") => eprintln!(
                "  ⚠ max_findings_in_hot_files: skipped — run `codelore ingest-sarif` first"
            ),
            ("corpus_percentile_max", "skipped") => eprintln!(
                "  ⚠ corpus_percentile_max: skipped — no corpus percentile data (no calibration artifact active, or no analyzed file resolved a percentile)"
            ),
            ("code_health_min", "degraded") => eprintln!(
                "  ⚠ code_health_min: degraded — health scan returned no rows on a non-empty repo"
            ),
            _ => {}
        }
    }
}

/// Gate result bundle: violations + ledger records from one gate group.
type GateGroupResult = (
    Vec<codelore_lib::cli_api::quality_gates::GateViolation>,
    Vec<codelore_lib::cli_api::quality_gates::ledger::GateRunRecord>,
);

/// Build one ledger record for a simple scalar gate.
fn make_rec(
    gate: &str,
    threshold: f64,
    value: f64,
    failed: bool,
    ts: &str,
    head_sha: &str,
) -> codelore_lib::cli_api::quality_gates::ledger::GateRunRecord {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;
    GateRunRecord {
        ts: ts.to_owned(),
        head_sha: head_sha.to_owned(),
        gate: gate.to_owned(),
        threshold,
        value,
        verdict: if failed { "failed" } else { "passed" }.to_owned(),
        mode: "check".to_owned(),
    }
}

/// Evaluate hotspot-based gates (`cognitive_max`, `hotspot_score_max`).
/// Returns the gate result bundle and the hotspot rows (reused by
/// `run_finding_hotspot_overlap_with` to avoid a second hotspot query).
fn eval_hotspot_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    ts: &str,
    head_sha: &str,
) -> Result<(
    GateGroupResult,
    Vec<codelore_lib::cli_api::analyses::hotspots::HotspotRow>,
)> {
    use codelore_lib::cli_api::analyses::hotspots::run_hotspots;
    use codelore_lib::cli_api::quality_gates::evaluate_full_tree;
    // The gate must see the whole population — a `--rows` display cap must
    // never change which files the gate evaluates. `with_no_row_limit` is a
    // no-op when no cap is set, so the gate outcome is unaffected today and
    // stays correct if a row cap is ever threaded into this path.
    let hotspots = run_hotspots(db, &opts.with_no_row_limit()).context("run hotspots")?;
    let hs_violations = evaluate_full_tree(thresholds, &hotspots);
    let g = &thresholds.gates;
    let mut recs = Vec::new();
    if let Some(max) = g.cognitive_max {
        let failed = hs_violations.iter().any(|v| v.gate == "cognitive_max");
        let value = hotspots
            .iter()
            .map(|r| r.cognitive)
            .fold(f64::NAN, f64::max);
        recs.push(make_rec(
            "cognitive_max",
            max,
            if value.is_nan() { 0.0 } else { value },
            failed,
            ts,
            head_sha,
        ));
    }
    if let Some(max) = g.hotspot_score_max {
        let failed = hs_violations.iter().any(|v| v.gate == "hotspot_score_max");
        let value = hotspots
            .iter()
            .map(|r| r.hotspot_score)
            .fold(f64::NAN, f64::max);
        recs.push(make_rec(
            "hotspot_score_max",
            max,
            if value.is_nan() { 0.0 } else { value },
            failed,
            ts,
            head_sha,
        ));
    }
    Ok(((hs_violations, recs), hotspots))
}

/// Evaluate `code_health_min` gate with degraded-detection.
/// Returns the gate result bundle + the raw `CodeHealthRow` vec (reused by ratchet).
fn eval_code_health_gate(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    ts: &str,
    head_sha: &str,
) -> Result<(
    GateGroupResult,
    Vec<codelore_lib::cli_api::analyses::code_health::CodeHealthRow>,
)> {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;
    use codelore_lib::cli_api::quality_gates::{GateViolation, evaluate_code_health_gate};
    // Gate over the whole population — a `--rows` display cap must not change
    // which files the gate evaluates (no-op today; correct if a cap is ever
    // threaded in).
    let code_health = codelore_lib::cli_api::analyses::code_health::run_code_health(
        db,
        &opts.with_no_row_limit(),
    )
    .context("run code-health")?;
    let g = &thresholds.gates;
    let Some(min) = g.code_health_min else {
        return Ok(((Vec::new(), Vec::new()), code_health));
    };
    let ch_violations = evaluate_code_health_gate(thresholds, &code_health);
    // Degraded: empty result when the repo has scorable files.
    let degraded = code_health.is_empty() && {
        db.query_row("SELECT COUNT(*) FROM complexity_metrics", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0)
            > 0
    };
    let worst = code_health
        .iter()
        .map(|r| r.score)
        .fold(f64::INFINITY, f64::min);
    let verdict = if degraded {
        "degraded"
    } else if ch_violations.is_empty() {
        "passed"
    } else {
        "failed"
    };
    let rec = GateRunRecord {
        ts: ts.to_owned(),
        head_sha: head_sha.to_owned(),
        gate: "code_health_min".into(),
        threshold: min,
        value: if worst.is_infinite() { 0.0 } else { worst },
        verdict: verdict.to_owned(),
        mode: "check".into(),
    };
    let mut violations = Vec::new();
    if degraded && g.fail_on_degraded {
        violations.push(GateViolation {
            gate: "code_health_min".into(),
            path: "(degraded)".into(),
            actual: "no-data".into(),
            threshold: format!("{min:.1}"),
        });
    } else {
        violations.extend(ch_violations);
    }
    Ok(((violations, vec![rec]), code_health))
}

/// Evaluate architecture gates (`max_dependency_cycles`, `max_propagation_cost`).
fn eval_arch_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    ts: &str,
    head_sha: &str,
) -> Result<GateGroupResult> {
    let (arch_v, measured) =
        codelore_lib::cli_api::quality_gates::evaluate_architecture_gate_measured(thresholds, db)
            .context("evaluate architecture gate")?;
    let g = &thresholds.gates;
    let mut recs = Vec::new();
    if let (Some(max), Some(m)) = (g.max_dependency_cycles, measured) {
        let failed = arch_v.iter().any(|v| v.gate == "max_dependency_cycles");
        recs.push(make_rec(
            "max_dependency_cycles",
            f64::from(max),
            f64::from(m.cycle_count),
            failed,
            ts,
            head_sha,
        ));
    }
    if let (Some(max), Some(m)) = (g.max_propagation_cost, measured) {
        let failed = arch_v.iter().any(|v| v.gate == "max_propagation_cost");
        recs.push(make_rec(
            "max_propagation_cost",
            max,
            m.propagation_cost,
            failed,
            ts,
            head_sha,
        ));
    }
    Ok((arch_v, recs))
}

/// Evaluate all configured gates and build ledger records for this run.
///
/// Returns `(violations, ledger_records, hotspot_count, code_health_rows)`.
/// `code_health_rows` is returned so callers (e.g. `--ratchet`) can extract
/// ratchet metrics without re-running the analysis.
///
/// `external_store` is the pre-opened sidecar for the
/// `max_findings_in_hot_files` gate. Pass `Some(store)` when the sidecar exists
/// and holds findings; `None` when absent or empty (gate skipped, no sidecar
/// created).
///
/// This is a pure compute layer: it records each gate's verdict in the returned
/// ledger records (including `"skipped"` and `"degraded"`) and prints nothing.
/// `run_check_cmd` renders the skip/degraded notices from those records.
#[allow(clippy::type_complexity, clippy::too_many_lines)]
fn evaluate_all_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    head_sha: &str,
    ts: &str,
    external_store: Option<&codelore_lib::cli_api::external::ExternalStore>,
) -> Result<(
    Vec<codelore_lib::cli_api::quality_gates::GateViolation>,
    Vec<codelore_lib::cli_api::quality_gates::ledger::GateRunRecord>,
    usize,
    Vec<codelore_lib::cli_api::analyses::code_health::CodeHealthRow>,
)> {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;

    let mut violations = Vec::new();
    let mut recs = Vec::new();
    let g = &thresholds.gates;

    let ((hs_v, hs_r), hotspot_rows) = eval_hotspot_gates(thresholds, db, opts, ts, head_sha)?;
    let hotspot_count = hotspot_rows.len();
    violations.extend(hs_v);
    recs.extend(hs_r);

    let ((ch_v, ch_r), code_health) = eval_code_health_gate(thresholds, db, opts, ts, head_sha)?;
    violations.extend(ch_v);
    recs.extend(ch_r);

    if g.disallow_clone_type_1 {
        let clone_v = codelore_lib::cli_api::quality_gates::evaluate_clone_gate(thresholds, db)
            .context("evaluate clone gate")?;
        let count = clone_v
            .first()
            .and_then(|v| v.actual.parse::<f64>().ok())
            .unwrap_or(0.0);
        recs.push(make_rec(
            "disallow_clone_type_1",
            0.0,
            count,
            !clone_v.is_empty(),
            ts,
            head_sha,
        ));
        violations.extend(clone_v);
    }

    let (arch_v, arch_r) = eval_arch_gates(thresholds, db, ts, head_sha)?;
    violations.extend(arch_v);
    recs.extend(arch_r);

    if let Some(max) = g.max_red_effort_pct {
        // Reuse the code-health rows already computed for `code_health_min` —
        // effort-exposure's band table derives from the same HEAD scan, and
        // the measured red-band churn share must be recorded on passing runs
        // too (the ratchet and `--history` read it from the ledger).
        let rows =
            codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure_with_health(
                db,
                &opts.with_no_row_limit(),
                &code_health,
            )
            .context("run effort-exposure for gate")?;
        let value = rows
            .iter()
            .find(|r| r.band == "red")
            .map_or(0.0, |r| r.churn_share_pct);
        let effort_v =
            codelore_lib::cli_api::quality_gates::evaluate_effort_exposure_rows(max, &rows);
        recs.push(make_rec(
            "max_red_effort_pct",
            max,
            value,
            !effort_v.is_empty(),
            ts,
            head_sha,
        ));
        violations.extend(effort_v);
    }

    if let Some(min) = g.code_familiarity_min {
        let rows =
            codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                .context("run code-familiarity for gate")?;
        // Measured familiarity is recorded pass or fail; an empty row set
        // (no recognized source files) records 0.0 with a vacuous pass.
        // Unlike `code_health_min` this gate has no degraded sentinel: an
        // empty result IS the documented no-source-files contract, not a
        // scan failure.
        let value = rows.first().map_or(0.0, |r| r.familiarity_pct);
        let fam_v = codelore_lib::cli_api::quality_gates::evaluate_familiarity_rows(min, &rows);
        recs.push(make_rec(
            "code_familiarity_min",
            min,
            value,
            !fam_v.is_empty(),
            ts,
            head_sha,
        ));
        violations.extend(fam_v);
    }

    // ── max_findings_in_hot_files gate ───────────────────────────────────────
    if let Some(threshold) = g.max_findings_in_hot_files {
        // The gate is skipped (not failed) when the sidecar is absent OR present
        // but empty — both mean "no external findings to evaluate yet".
        // `external_store` already collapses those two states to `None` (via
        // `open_nonempty`), mirroring the MCP overlap tool. The `"skipped"`
        // verdict recorded here is what run_check_cmd renders the notice from.
        match external_store {
            None => {
                recs.push(GateRunRecord {
                    ts: ts.to_owned(),
                    head_sha: head_sha.to_owned(),
                    gate: "max_findings_in_hot_files".into(),
                    threshold: f64::from(threshold),
                    value: 0.0,
                    verdict: "skipped".into(),
                    mode: "check".into(),
                });
            }
            Some(store) => {
                // Reuse already-computed hotspot and code-health rows so we
                // don't re-run those analyses a second time (mirrors how
                // max_red_effort_pct reuses code_health above).
                let overlap_rows = codelore_lib::cli_api::analyses::finding_hotspot_overlap::run_finding_hotspot_overlap_with(
                    store,
                    &hotspot_rows,
                    &code_health,
                )
                .context("run finding-hotspot-overlap for gate")?;
                let act_now_count = overlap_rows
                    .iter()
                    .filter(|r| r.priority == "act-now")
                    .count();
                let overlap_v = codelore_lib::cli_api::quality_gates::evaluate_finding_overlap_rows(
                    threshold,
                    &overlap_rows,
                );
                #[allow(clippy::cast_precision_loss)]
                // act_now_count is a repo-scale count; precision loss negligible
                let act_now_f64 = act_now_count as f64;
                recs.push(GateRunRecord {
                    ts: ts.to_owned(),
                    head_sha: head_sha.to_owned(),
                    gate: "max_findings_in_hot_files".into(),
                    threshold: f64::from(threshold),
                    value: act_now_f64,
                    verdict: if overlap_v.is_empty() {
                        "passed"
                    } else {
                        "failed"
                    }
                    .into(),
                    mode: "check".into(),
                });
                violations.extend(overlap_v);
            }
        }
    }

    // ── corpus_percentile_max gate ───────────────────────────────────────────
    if let Some(max) = g.corpus_percentile_max {
        // Reuse the already-computed code-health rows. The lens is active only
        // when a calibration artifact is (`--calibration` or an embedded world
        // corpus); without one every row carries `corpus_percentile = None`.
        // That is a SKIP (not a pass, not a fail) — there is no reference corpus
        // to compare against — mirroring the max_findings sidecar-absent skip.
        let has_calibration = code_health.iter().any(|r| r.corpus_percentile.is_some());
        if has_calibration {
            let corpus_v = codelore_lib::cli_api::quality_gates::evaluate_corpus_percentile_rows(
                max,
                &code_health,
            );
            let value = code_health
                .iter()
                .filter_map(|r| r.corpus_percentile)
                .fold(0.0, f64::max);
            recs.push(make_rec(
                "corpus_percentile_max",
                max,
                value,
                !corpus_v.is_empty(),
                ts,
                head_sha,
            ));
            violations.extend(corpus_v);
        } else {
            recs.push(GateRunRecord {
                ts: ts.to_owned(),
                head_sha: head_sha.to_owned(),
                gate: "corpus_percentile_max".into(),
                threshold: max,
                value: 0.0,
                verdict: "skipped".into(),
                mode: "check".into(),
            });
        }
    }

    Ok((violations, recs, hotspot_count, code_health))
}

/// Emit a check SARIF document for `violations` (with their `evidence` chains)
/// to stdout. Canonicalizes `repo` for the artifact-URI prefix, falling back to
/// the path as-given when canonicalization fails (e.g. the path does not exist
/// on disk). Shared by the vacuous-pass path (empty violations + evidence) and
/// the violation path so the canonicalize + stdout + writer wiring lives once.
/// Write a ratchet status message to the right stream for the output format.
///
/// Under `--format sarif` the message goes to stderr so stdout stays a clean
/// SARIF document (mirroring the report path, which routes verdict lines to
/// stderr in SARIF mode); in text mode it goes to stdout with the rest of the
/// report. `msg` is written verbatim, so the caller supplies any trailing
/// newline.
fn emit_ratchet_message(args: &args::CheckArgs, msg: &str) {
    if matches!(args.format, CheckFormat::Sarif) {
        eprint!("{msg}");
    } else {
        print!("{msg}");
    }
}

/// Emit the check SARIF document to stdout when `--format sarif` is set;
/// no-op otherwise.
///
/// Collects a commit evidence chain for each violated per-file path and hands
/// the violations + evidence to [`emit_check_sarif`]. Shared by the normal
/// check path and every `--ratchet` exit path so the flag combination always
/// yields a valid document rather than silently emitting nothing.
fn emit_check_sarif_when_requested(
    args: &args::CheckArgs,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    head_sha: &str,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
) -> Result<()> {
    use codelore_lib::cli_api::quality_gates::evidence::{EvidenceCommit, evidence_for_path};
    use std::collections::HashMap;

    if !matches!(args.format, CheckFormat::Sarif) {
        return Ok(());
    }

    // Collect evidence only for violated per-file paths (not repo-wide). A
    // failed lookup degrades that result to chainless (empty evidence) rather
    // than failing the SARIF emission; the failure is systemic and repeats per
    // path, so warn at most once per run via the ⚠-prefixed stderr convention.
    let mut evidence_map: HashMap<String, Vec<EvidenceCommit>> = HashMap::new();
    let mut evidence_warned = false;
    for v in violations {
        if v.path != "(repo-wide)" && v.path != "(degraded)" {
            evidence_map.entry(v.path.clone()).or_insert_with(|| {
                evidence_for_path(db, opts, &v.path, 5).unwrap_or_else(|e| {
                    if !evidence_warned {
                        evidence_warned = true;
                        eprintln!(
                            "  ⚠ check: evidence lookup failed ({e}); SARIF results will be emitted without commit chains"
                        );
                    }
                    Vec::new()
                })
            });
        }
    }

    emit_check_sarif(&args.repo, head_sha, violations, &evidence_map)
}

fn emit_check_sarif(
    repo: &std::path::Path,
    head_sha: &str,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
    evidence: &std::collections::HashMap<
        String,
        Vec<codelore_lib::cli_api::quality_gates::evidence::EvidenceCommit>,
    >,
) -> Result<()> {
    let repo_root = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut stdout = std::io::stdout();
    codelore_lib::cli_api::output::sarif::write_check_sarif(
        violations,
        evidence,
        &repo_root,
        head_sha,
        &mut stdout,
    )
    .context("emit check SARIF")
}
