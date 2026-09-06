//! `codelore check` — repository quality-gate evaluation.
//!
//! Loads the configured thresholds, runs the gate-relevant analyses against the
//! repository HEAD, evaluates every configured gate, records each verdict in the
//! gate-run ledger, optionally emits a SARIF document, and exits 0 (pass) or 1
//! (fail). Also serves `--history` (print the ledger without evaluating) and
//! `--ratchet` (compare against the stored regression snapshot).

use anyhow::{Context, Result};

use crate::args::{self, CheckFormat};
use crate::{
    CORPUS_PERCENTILE_SKIP_REASON, new_code_skip_reason, notice_corpus_lens_absent,
    vacuous_pass_notice, write_github_output,
};

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

    // Validate the repository BEFORE reading thresholds. `Thresholds::discover`
    // resolves `<repo>/.codelore-thresholds.toml` and returns the default,
    // empty set when that file does not exist — which a nonexistent repo root
    // satisfies trivially. The vacuous-pass branch below then reported PASS
    // and wrote `result=pass`, so a typo'd `--repo` in a workflow produced a
    // green gate. Opening the repository first makes a bad path an exit-3
    // repository error, the way `analyze` and `diff` already treat it, and
    // leaves the vacuous pass to mean what it says: a real repository with no
    // thresholds configured.
    //
    // This surface had the sharper symptom: its vacuous branch opened the
    // repository only under `--format sarif`, so the SAME bad path exited 3
    // as SARIF and 0 as text — the verdict depended on how the output was
    // asked for.
    // `GixRepo::open` succeeds on an unborn HEAD — it calls `gix::open` and
    // nothing else — so resolving HEAD is a SECOND condition, not a detail of
    // the first. Without it this guard closed the general case and left the
    // narrow one: a `git init` with no commits still passed vacuously as text
    // and failed as SARIF, because only the SARIF arm went on to touch HEAD.
    // `analyze`'s pre-flight already treats both as repository errors.
    {
        use codelore_lib::cli_api::repo::Repo as _;
        let repo = codelore_lib::cli_api::repo::GixRepo::open(&args.repo).context("open repo")?;
        repo.head_sha().context("resolve HEAD")?;
    }

    let thresholds = if let Some(path) = &args.thresholds_file {
        Thresholds::from_path(path).context("load thresholds file")?
    } else {
        Thresholds::discover(&args.repo).context("discover thresholds file")?
    };

    if thresholds.is_empty() && !args.ratchet {
        if !args.quiet {
            eprintln!("{}", vacuous_pass_notice("check"));
        }
        write_github_output("result", "pass");
        // Every other exit path writes both keys; a vacuous pass had been
        // writing only `result`, so a workflow reading `outputs.violations`
        // got an empty string instead of a count.
        write_github_output("violations", "0");
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
    // Witness the ingest before any gate runs: a real HEAD over an empty commit
    // store is the truncated-checkout signature (a shallow merge-tip fetch under
    // the default merge filter ingests zero history), on which every gate would
    // pass over no data. Turn that silent green into a hard, distinct error.
    db.ensure_ingest_witnessed(&head_sha)?;
    // Defence in depth: a shallow checkout that DID ingest a commit or two still
    // carries only partial history, which quietly weakens every behavioral gate.
    // Warn loudly and name the cause; SARIF mode keeps stdout a clean document.
    // The same signal discriminates the `new_code` skip disclosure below (a
    // truncated checkout reads identically to a young repository at that query).
    let shallow_checkout = repo.is_shallow();
    if shallow_checkout {
        let warning = "⚠ codelore check: shallow checkout detected (.git/shallow present) — \
             history is truncated by fetch-depth, so the behavioral gates (hotspots, \
             effort-exposure, new-code) evaluate only partial history. Re-run against full \
             history (fetch-depth: 0) for an authoritative verdict.";
        // Stderr in every mode: warnings and verdicts are run METADATA, and
        // the documented contract keeps them on stderr so `codelore check >
        // log` never splits the story (stdout stays the report document).
        eprintln!("{warning}");
    }
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

    let (mut violations, mut ledger_records, hotspot_count, code_health) = evaluate_all_gates(
        &thresholds,
        &db,
        &repo,
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
        // Run-level first: one scan degrades every complexity-derived gate, so
        // the cause is stated once above the per-gate lines rather than
        // repeated beneath each of them. Fires whether or not
        // `fail_on_degraded` turns it into a failure — opting out of the
        // failure is exactly when the operator still wants the diagnosis.
        if let Some((scored, eligible)) = thin_scan_coverage(&db)? {
            let affected: Vec<&str> = ledger_records
                .iter()
                .filter(|r| r.verdict == "degraded")
                .map(|r| r.gate.as_str())
                .filter(|g| COMPLEXITY_DERIVED_GATES.contains(g))
                .collect();
            if !affected.is_empty() {
                eprintln!("  ⚠ {}", thin_scan_notice(scored, eligible, &affected));
            }
        }
        emit_gate_notices(&ledger_records, shallow_checkout);
    }
    // One-per-run hint when the corpus lens is inactive — the check path always
    // computes code-health rows, which carry no corpus_percentile without an
    // artifact. Suppressed under --quiet (handled inside).
    notice_corpus_lens_absent(&opts, args.quiet);

    // ── Ratchet ───────────────────────────────────────────────────────────────
    if args.ratchet {
        // Build ratchet metrics from already-computed gate outputs.
        //
        // Every metric is tracked only when its gate is configured — the
        // README states this, and it is what makes the ratchet a tightening
        // of bounds the user chose rather than a new bound they did not.
        // `red_effort_pct` and `dependency_cycles` get it for free by reading
        // ledger records, which exist only if their gate ran. Code health does
        // not: `run_code_health` runs unconditionally (other gates consume its
        // rows), so reading the worst score straight off the scan recorded a
        // floor for a gate that was never configured — and a later benign
        // refactor that nudged that score down by recomputation noise failed
        // the run against a bound the user never set.
        let worst_health = if thresholds.gates.code_health_min.is_some() {
            code_health
                .iter()
                .map(|r| r.score)
                .fold(f64::INFINITY, f64::min)
        } else {
            f64::INFINITY
        };
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

    // ── fail_on_skipped policy ───────────────────────────────────────────────
    // A gate recorded "skipped" becomes a violation so the run fails rather than
    // greening on a gate that never evaluated. The ledger write above keeps the
    // honest "skipped" verdict; only the exit-facing set gains rows. Placed
    // after the ratchet block (whose exit is regression-driven) so the policy
    // scopes to the normal check exit path.
    violations.extend(crate::skipped_gate_violations(
        &ledger_records,
        thresholds.gates.fail_on_skipped,
    ));

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
            // Stderr in every mode — the FAIL branch already lives there,
            // and the documented contract keeps every verdict line on
            // stderr so `codelore check > log` captures pass and fail
            // symmetrically (stdout stays the report document).
            eprintln!("{warning}");
        } else {
            // Emitted in SARIF mode too: the docs promise a verdict line
            // regardless of format, and stdout stays the clean document.
            eprintln!("✅ codelore check: PASS ({hotspot_count} files evaluated)");
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
/// The `code_health_min` degraded message, naming which of the two causes
/// fired and — for a thin scan — how thin.
///
/// Built as a value rather than printed inline so it can be asserted. The
/// magnitude is the whole point of the message: "too thin to judge" does not
/// tell an operator whether the scan missed one file or nine hundred, and that
/// is the number that decides whether to investigate.
fn blind_health_notice() -> String {
    "code_health_min: degraded — the health scan returned no rows on a repository \
     that carries analyzable source; the gate reports no verdict rather than one \
     drawn from a blind scan"
        .to_string()
}

/// The run-level message for a scan too thin to support any gate that reads
/// `complexity_metrics`.
///
/// Emitted once per run rather than once per affected gate: the cause is one
/// scan, and repeating it under each of ten gate names would bury the counts
/// that are the actionable part. Built as a value so those counts are
/// asserted — a disclosure that stops naming its magnitude is true and useless.
fn thin_scan_notice(scored: u64, eligible: u64, affected: &[&str]) -> String {
    format!(
        "scan coverage: degraded — the HEAD complexity scan scored {scored} of \
         {eligible} eligible files, below the coverage floor. Gates reading that \
         table describe only the part of the repository it measured, and a thinner \
         scan reads as a healthier one: {}. Re-ingest on a full clone, or set \
         `fail_on_degraded = false` to keep the disclosure without the failure.",
        affected.join(", ")
    )
}

fn emit_gate_notices(
    ledger_records: &[codelore_lib::cli_api::quality_gates::ledger::GateRunRecord],
    shallow_checkout: bool,
) {
    for r in ledger_records {
        match (r.gate.as_str(), r.verdict.as_str()) {
            ("max_findings_in_hot_files", "skipped") => eprintln!(
                "  ⚠ max_findings_in_hot_files: skipped — run `codelore ingest-sarif` first"
            ),
            ("corpus_percentile_max", "skipped") => {
                eprintln!("  ⚠ corpus_percentile_max: skipped — {CORPUS_PERCENTILE_SKIP_REASON}");
            }
            ("hotspot_anchored_max", "skipped") => eprintln!(
                "  ⚠ hotspot_anchored_max: skipped — no anchored hotspot data (no calibration artifact active, or no analyzed file's language is covered by the corpus)"
            ),
            // Two causes reach this verdict, and the magnitude is what decides
            // Only the blind cause reaches this gate now. Thin coverage
            // degrades ten gates rather than this one, so it is disclosed once
            // at run level instead of repeated under each gate's name.
            ("code_health_min", "degraded") => {
                eprintln!("  ⚠ {}", blind_health_notice());
            }
            ("new_code", "skipped") => eprintln!(
                "  ⚠ new_code: skipped — {}",
                new_code_skip_reason(r.threshold, shallow_checkout)
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
    use codelore_lib::cli_api::analyses::hotspots::run_hotspots_anchored;
    use codelore_lib::cli_api::quality_gates::evaluate_full_tree;
    // The gate must see the whole population — a `--rows` display cap must
    // never change which files the gate evaluates. `with_no_row_limit` is a
    // no-op when no cap is set, so the gate outcome is unaffected today and
    // stays correct if a row cap is ever threaded into this path.
    //
    // `run_hotspots_anchored` also fills `hotspot_score_anchored` so the
    // `hotspot_anchored_max` gate (evaluated in `evaluate_all_gates`) reads it
    // off these same rows; the always-on `cognitive_max` / `hotspot_score_max`
    // gates below are unaffected by the additive field.
    let hotspots = run_hotspots_anchored(db, &opts.with_no_row_limit()).context("run hotspots")?;
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

/// The HEAD scan's counts when its coverage fell below the floor, `None`
/// otherwise.
///
/// Two callers need this: the gate, which turns it into a violation and an
/// exit code, and the operator notice, which only reports it. They read the
/// same store in the same run and must not describe it differently, so the
/// mapping from verdict to counts lives here rather than at each site — the
/// same reason the floor comparison itself sits beside its constant.
fn thin_scan_coverage(db: &codelore_lib::cli_api::facts::FactsDb) -> Result<Option<(u64, u64)>> {
    Ok(
        match db
            .head_scan_coverage_verdict()
            .context("read scan coverage")?
        {
            codelore_lib::cli_api::facts::ScanCoverageVerdict::Below { scored, eligible } => {
                Some((scored, eligible))
            }
            _ => None,
        },
    )
}

/// Evaluate `code_health_min` gate with degraded-detection.
/// Returns the gate result bundle + the raw `CodeHealthRow` vec (reused by ratchet).
fn eval_code_health_gate(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    repo: &impl codelore_lib::cli_api::repo::Repo,
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
    // Blind: the health scan returned nothing, yet the repository actually
    // carries analyzable source. The witness reads the HEAD tree directly rather
    // than counting `complexity_metrics`, which derives from the same
    // changes⋈commits join as the (empty) health set and so empties in lockstep
    // — it cannot witness an ingest that went blind. A source-less tree (docs or
    // config only) legitimately yields no rows and stays a vacuous pass. The `&&`
    // short-circuit keeps the tree walk off every healthy run.
    let blind = code_health.is_empty()
        && codelore_lib::cli_api::quality_gates::head_has_scorable_source(repo, opts);
    // Thin coverage is the other way this gate can green on blindness, but it
    // is NOT handled here: it degrades every gate that reads
    // `complexity_metrics`, not this one, and deciding it inside this
    // evaluator put it below the `code_health_min` early return above — so a
    // threshold file configuring any other complexity-derived gate never
    // reached it at all. It is applied once in `evaluate_all_gates`, where
    // every gate's record exists. What stays here is the cause that genuinely
    // belongs to this gate: an empty health set on a repository that carries
    // analyzable source, which no other gate can witness.
    let degraded = blind;
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
            // Only the blind cause reaches here now, and `no-data` is exactly
            // true of it. The thin-scan cause carries counts instead, and is
            // reported by the run-level pass that owns it.
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
/// Gates whose measurement derives from `complexity_metrics`, and which are
/// therefore untrustworthy when the HEAD scan that filled that table reached
/// too little of the repository.
///
/// Every entry reads the table, directly or as the universe it seeds:
/// `import_graph` selects its node set straight from it, so a thin scan
/// shrinks the graph and reports FEWER cycles — the reads-thinner-as-better
/// shape the floor exists to catch, and the reason the architecture gates are
/// here rather than treated as structural.
///
/// `disallow_clone_type_1` is the sole exemption: clone detection runs its own
/// HEAD scan, whose coverage this figure says nothing about.
///
/// Deliberately placed against [`assert_every_gate_is_classified`]: that
/// destructure already fails to compile when a gate is added, so the edit that
/// forces a new gate to be wired is the same edit that must decide whether it
/// reads complexity. A list kept anywhere else would drift silently.
const COMPLEXITY_DERIVED_GATES: &[&str] = &[
    "cognitive_max",
    "hotspot_score_max",
    "hotspot_anchored_max",
    "code_health_min",
    "max_dependency_cycles",
    "max_propagation_cost",
    "max_red_effort_pct",
    "code_familiarity_min",
    "max_findings_in_hot_files",
    "corpus_percentile_max",
];

/// Compile-time exhaustiveness anchor for the AUTHORITATIVE gate surface,
/// mirroring the destructure the MCP `check_gates` tool already carries:
/// adding a field to `Gates` fails to compile HERE until the new gate is
/// classified — wired into one of the `eval_*` functions below, or
/// documented as a policy flag / modifier. Without this anchor, a gate
/// added to `Gates` and `.codelore-thresholds.toml` could be configured
/// and silently enforce nothing in CI (the structural half of the
/// ledger's warning-table finding): `check` evaluated gates across
/// independent branches with nothing holding the set together, while the
/// advisory MCP surface was the only one guarded.
fn assert_every_gate_is_classified(g: &codelore_lib::cli_api::quality_gates::Gates) {
    use codelore_lib::cli_api::quality_gates::Gates;
    let Gates {
        cognitive_max: _,               // evaluated: eval_hotspot_gates; complexity-derived
        hotspot_score_max: _,           // evaluated: eval_hotspot_gates; complexity-derived
        hotspot_anchored_max: _,        // evaluated: eval_hotspot_gates; complexity-derived
        code_health_min: _,             // evaluated: eval_code_health_gate; complexity-derived
        disallow_clone_type_1: _,       // evaluated: the clone gate; NOT complexity-derived
        max_dependency_cycles: _,       // evaluated: eval_arch_gates; complexity-derived
        max_propagation_cost: _,        // evaluated: eval_arch_gates; complexity-derived
        max_red_effort_pct: _,          // evaluated: the effort gate; complexity-derived
        code_familiarity_min: _,        // evaluated: the familiarity gate; complexity-derived
        max_findings_in_hot_files: _,   // evaluated: the external-findings gate; complexity-derived
        corpus_percentile_max: _,       // evaluated: the corpus-lens gate; complexity-derived
        fail_on_degraded: _,            // policy: degraded-gate exit semantics, not a gate
        fail_on_skipped: _,             // policy: cross-surface exit-code semantics
        red_effort_exempt_improving: _, // modifier of max_red_effort_pct
    } = g;
}

/// Mark every complexity-derived gate in `recs` as degraded, returning the
/// names marked.
///
/// Split out of `evaluate_all_gates` so the rule can be tested without a
/// repository whose scan is actually thin — forcing that needs a corrupt pack
/// or a blobless clone, so the alternative is no coverage at all for the pass
/// that decides which gates a thin scan invalidates.
///
/// A `skipped` gate stays skipped: it evaluated nothing, so there is nothing to
/// distrust. Both `passed` and `failed` become `degraded`, which keeps the
/// precedence the code-health gate used while it owned this rule — "we could
/// not judge" outranks either judgement drawn from a scan too thin to support
/// one.
fn degrade_complexity_derived(
    recs: &mut [codelore_lib::cli_api::quality_gates::ledger::GateRunRecord],
) -> Vec<&'static str> {
    let mut affected = Vec::new();
    for r in recs {
        if r.verdict == "skipped" {
            continue;
        }
        if let Some(name) = COMPLEXITY_DERIVED_GATES.iter().find(|g| **g == r.gate) {
            "degraded".clone_into(&mut r.verdict);
            affected.push(*name);
        }
    }
    affected
}

#[allow(clippy::type_complexity, clippy::too_many_lines)]
fn evaluate_all_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    repo: &impl codelore_lib::cli_api::repo::Repo,
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
    assert_every_gate_is_classified(g);

    let ((hs_v, hs_r), hotspot_rows) = eval_hotspot_gates(thresholds, db, opts, ts, head_sha)?;
    let hotspot_count = hotspot_rows.len();
    violations.extend(hs_v);
    recs.extend(hs_r);

    let ((ch_v, ch_r), code_health) =
        eval_code_health_gate(thresholds, db, repo, opts, ts, head_sha)?;
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
        //
        // With the improving-churn exemption on, decompose the red band's
        // window churn (a scoped window-start parse of the red files only, via
        // the repo) so the gate compares the DEGRADING share; otherwise stay on
        // the base SQL rows — no extra scan on the default path.
        use codelore_lib::cli_api::analyses::effort_exposure;
        let exempt = g.red_effort_exempt_improving;
        let no_limit = opts.with_no_row_limit();
        let rows = if exempt {
            effort_exposure::run_effort_exposure_decomposed(db, repo, &no_limit, &code_health)
        } else {
            effort_exposure::run_effort_exposure_with_health(db, &no_limit, &code_health)
        }
        .context("run effort-exposure for gate")?;
        // The recorded value is the effective gated number: the degrading share
        // when exempting (falling back to the total red share if the split is
        // unavailable), else the full red share.
        let red = rows.iter().find(|r| r.band == "red");
        let value = if exempt {
            red.and_then(|r| r.churn_share_degrading_pct)
                .or_else(|| red.map(|r| r.churn_share_pct))
                .unwrap_or(0.0)
        } else {
            red.map_or(0.0, |r| r.churn_share_pct)
        };
        let effort_v = codelore_lib::cli_api::quality_gates::evaluate_effort_exposure_rows_exempt(
            max, exempt, &rows,
        );
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

    // ── [new_code] two-band period gate ──────────────────────────────────────
    if let Some(nc) = &thresholds.new_code {
        // Reuse the HEAD code-health rows already computed for `code_health_min`
        // (the born band's scores + the live-source universe) and the
        // effort-exposure window-start machinery (the touched band's net
        // movement) — no second health scan on this path.
        use codelore_lib::cli_api::analyses::new_code;
        let scope = new_code::run_new_code_scope(db, repo, opts, nc.window_days, &code_health)
            .context("run new-code scope for gate")?;
        if scope.window_start_present {
            let nc_v = codelore_lib::cli_api::quality_gates::evaluate_new_code_rows(nc, &scope);
            // Per-band ledger records so `--history` shows each obligation. The
            // ratchet reads its own typed metrics, not these, so new gate names
            // are display-only here.
            if let Some(floor) = nc.born_health_min {
                let worst = scope
                    .born
                    .iter()
                    .map(|(_, s)| *s)
                    .fold(f64::INFINITY, f64::min);
                recs.push(make_rec(
                    "born_health_min",
                    floor,
                    if worst.is_finite() { worst } else { 0.0 },
                    nc_v.iter().any(|v| v.gate == "born_health_min"),
                    ts,
                    head_sha,
                ));
            }
            if nc.touched_no_degradation {
                let worst = scope
                    .touched
                    .iter()
                    .map(|(_, n)| *n)
                    .fold(f64::INFINITY, f64::min);
                recs.push(make_rec(
                    "touched_no_degradation",
                    0.0,
                    if worst.is_finite() { worst } else { 0.0 },
                    nc_v.iter().any(|v| v.gate == "touched_no_degradation"),
                    ts,
                    head_sha,
                ));
            }
            violations.extend(nc_v);
        } else {
            // History shallower than the window ⇒ no legacy baseline to contrast
            // the working set against. Skip with disclosure, mirroring the
            // corpus_percentile_max / hotspot_anchored_max skip convention.
            recs.push(GateRunRecord {
                ts: ts.to_owned(),
                head_sha: head_sha.to_owned(),
                gate: "new_code".into(),
                threshold: f64::from(nc.window_days),
                value: 0.0,
                verdict: "skipped".into(),
                mode: "check".into(),
            });
        }
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

    // ── hotspot_anchored_max gate ────────────────────────────────────────────
    if let Some(max) = g.hotspot_anchored_max {
        // Reuse the hotspot rows already computed for the hotspot gates —
        // `eval_hotspot_gates` runs them through `run_hotspots_anchored`, so the
        // anchor is populated exactly when a calibration artifact is active
        // (`--calibration` or the embedded world corpus). Without one every row
        // carries `hotspot_score_anchored = None`, which is a SKIP (not a pass,
        // not a fail): there is no reference corpus to compare against. Mirrors
        // the corpus_percentile_max skip above.
        let has_anchor = hotspot_rows
            .iter()
            .any(|r| r.hotspot_score_anchored.is_some());
        if has_anchor {
            let anchored_v = codelore_lib::cli_api::quality_gates::evaluate_hotspot_anchored_rows(
                max,
                &hotspot_rows,
            );
            let value = hotspot_rows
                .iter()
                .filter_map(|r| r.hotspot_score_anchored)
                .fold(0.0, f64::max);
            recs.push(make_rec(
                "hotspot_anchored_max",
                max,
                value,
                !anchored_v.is_empty(),
                ts,
                head_sha,
            ));
            violations.extend(anchored_v);
        } else {
            recs.push(GateRunRecord {
                ts: ts.to_owned(),
                head_sha: head_sha.to_owned(),
                gate: "hotspot_anchored_max".into(),
                threshold: max,
                value: 0.0,
                verdict: "skipped".into(),
                mode: "check".into(),
            });
        }
    }

    // ── Thin scan degrades every gate that reads the thin table ────────────
    // The floor used to be consumed inside `eval_code_health_gate`, below that
    // gate's own `code_health_min` early return, so a threshold file that
    // configured any OTHER complexity-derived gate never reached it: for those
    // configs the enforcement was not weakened, it was absent. `cognitive_max`
    // on a blobless clone scoring forty of five thousand files passed clean.
    //
    // Applied here, once, where every gate's record already exists. A skipped
    // gate stays skipped — it evaluated nothing, so there is nothing to
    // distrust — while both `passed` and `failed` become `degraded`, keeping
    // the precedence the code-health gate already used when it owned this.
    if let Some((scored, eligible)) = thin_scan_coverage(db)? {
        let affected = degrade_complexity_derived(&mut recs);
        if !affected.is_empty() && g.fail_on_degraded {
            violations.push(codelore_lib::cli_api::quality_gates::GateViolation {
                gate: "scan_coverage".into(),
                path: "(degraded)".into(),
                actual: format!("{scored}/{eligible} files scanned"),
                // Qualitative, matching the `no new cycles` / `>= 0` thresholds
                // beside it. The numeric floor stays `pub(crate)` in the lib so
                // nothing out here can re-apply it and drift from the one
                // comparison that decides the verdict.
                threshold: "\u{2265} coverage floor".into(),
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
        // Evidence is a per-file commit chain, so it is meaningless for a
        // pseudo-path. Same canonical predicate the emitters use, rather than
        // a third copy of the literal list.
        if !codelore_lib::cli_api::quality_gates::evaluators::is_pseudo_path(&v.path) {
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

#[cfg(test)]
mod complexity_degradation_tests {
    use super::{COMPLEXITY_DERIVED_GATES, degrade_complexity_derived};
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;

    fn rec(gate: &str, verdict: &str) -> GateRunRecord {
        GateRunRecord {
            ts: "2026-09-06T00:00:00Z".into(),
            head_sha: "abc123".into(),
            gate: gate.into(),
            threshold: 1.0,
            value: 1.0,
            verdict: verdict.into(),
            mode: "check".into(),
        }
    }

    #[test]
    fn a_gate_that_reads_the_thin_table_is_degraded_whether_it_passed_or_failed() {
        // The bug this closes: the floor was consumed below `code_health_min`'s
        // own early return, so a config setting only `cognitive_max` never
        // reached it and passed clean on a scan covering a fraction of the tree.
        let mut recs = vec![
            rec("cognitive_max", "passed"),
            rec("code_health_min", "failed"),
        ];
        let affected = degrade_complexity_derived(&mut recs);
        assert_eq!(affected.len(), 2, "both gates read complexity_metrics");
        assert!(
            recs.iter().all(|r| r.verdict == "degraded"),
            "a judgement drawn from a scan too thin to support one is not a judgement"
        );
    }

    #[test]
    fn the_clone_gate_is_left_alone() {
        // Clone detection runs its own HEAD scan; this coverage figure says
        // nothing about it. Degrading it here would fail a run for a limitation
        // that does not apply to the gate the user configured.
        let mut recs = vec![rec("disallow_clone_type_1", "passed")];
        let affected = degrade_complexity_derived(&mut recs);
        assert!(
            affected.is_empty(),
            "the clone gate does not read complexity_metrics"
        );
        assert_eq!(recs[0].verdict, "passed");
    }

    #[test]
    fn a_skipped_gate_stays_skipped() {
        // It evaluated nothing, so there is nothing to distrust — and marking it
        // degraded would turn a gate that never ran into a build failure.
        let mut recs = vec![rec("corpus_percentile_max", "skipped")];
        let affected = degrade_complexity_derived(&mut recs);
        assert!(affected.is_empty());
        assert_eq!(recs[0].verdict, "skipped");
    }

    #[test]
    fn the_classified_set_is_not_empty_and_excludes_the_one_exemption() {
        // Anti-vacuity: an empty list would make every assertion above pass
        // while the pass degraded nothing at all.
        assert!(!COMPLEXITY_DERIVED_GATES.is_empty());
        assert!(!COMPLEXITY_DERIVED_GATES.contains(&"disallow_clone_type_1"));
        assert!(COMPLEXITY_DERIVED_GATES.contains(&"max_dependency_cycles"));
    }
}

#[cfg(test)]
mod degraded_notice_tests {
    use super::{blind_health_notice, thin_scan_notice};

    #[test]
    fn a_thin_scan_notice_carries_both_counts() {
        let msg = thin_scan_notice(40, 5200, &["cognitive_max"]);
        assert!(
            msg.contains("40") && msg.contains("5200"),
            "the magnitude is what the operator acts on, but the message omitted it: {msg}"
        );
    }

    #[test]
    fn a_thin_scan_notice_names_the_gates_it_degraded() {
        // The counts say the scan was thin; the gate list says what that cost.
        // Without it the operator cannot tell whether the run they care about
        // was affected at all.
        let msg = thin_scan_notice(40, 5200, &["cognitive_max", "max_dependency_cycles"]);
        assert!(
            msg.contains("cognitive_max") && msg.contains("max_dependency_cycles"),
            "every degraded gate must be named: {msg}"
        );
    }

    #[test]
    fn a_thin_scan_is_not_described_as_having_returned_nothing() {
        // A thin scan DID return rows. Describing it as empty sends the
        // operator debugging a problem they do not have.
        let msg = thin_scan_notice(40, 5200, &["cognitive_max"]);
        assert!(
            !msg.contains("no rows"),
            "a partial scan returned rows by definition: {msg}"
        );
    }

    #[test]
    fn the_blind_notice_names_the_other_cause_and_invents_no_counts() {
        let msg = blind_health_notice();
        assert!(msg.contains("no rows"), "blind cause must be named: {msg}");
        assert!(
            !msg.contains("eligible files"),
            "there is no ratio to report when the scan returned nothing: {msg}"
        );
    }
}
