//! Gate evaluators: pure comparisons of analysis result sets (or a fact store)
//! against the configured [`Thresholds`], each returning the list of
//! [`GateViolation`]s it found.
//!
//! Every evaluator is a noop (empty result) when its governing threshold is
//! unset, so a caller can invoke the family unconditionally and let the
//! configured gates select themselves. The threshold config these read lives in
//! the sibling [`config`](super::config) module.

use super::config::Thresholds;

/// One detected gate violation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GateViolation {
    pub gate: String,
    pub path: String,
    pub actual: String,
    pub threshold: String,
}

/// Evaluate the `disallow_clone_type_1` gate by counting Type-1
/// clone families (`similarity = 1.0`) in the fact store. When the
/// gate is off this is a noop; when on, every distinct clone group
/// of similarity 1.0 surfaces as one violation row.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors.
pub fn evaluate_clone_gate(
    thresholds: &Thresholds,
    db: &crate::facts::FactsDb,
) -> crate::Result<Vec<GateViolation>> {
    if !thresholds.gates.disallow_clone_type_1 {
        return Ok(Vec::new());
    }
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT COUNT(DISTINCT clone_group_id) FROM clones \
             WHERE similarity = 1.0",
        )
        .map_err(|e| crate::CodeLoreError::Analysis(format!("prepare clone-gate: {e}")))?;
    let count: i64 = stmt
        .query_row([], |r| r.get(0))
        .map_err(|e| crate::CodeLoreError::Analysis(format!("query clone-gate: {e}")))?;
    if count == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![GateViolation {
        gate: "disallow_clone_type_1".into(),
        path: "(repo-wide)".into(),
        actual: count.to_string(),
        threshold: "0".into(),
    }])
}

/// Evaluate the architecture `[gates]` (`max_dependency_cycles`,
/// `max_propagation_cost`) against the import graph in the fact store.
/// Builds the graph once via the shared kernel; a noop (no graph build)
/// when neither gate is configured.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors
/// (propagated from the import-graph build).
pub fn evaluate_architecture_gate(
    thresholds: &Thresholds,
    db: &crate::facts::FactsDb,
) -> crate::Result<Vec<GateViolation>> {
    Ok(evaluate_architecture_gate_measured(thresholds, db)?.0)
}

/// Architecture metrics measured for gate evaluation, returned so callers
/// can record the observed values (ledger, ratchet) on passing runs too.
#[derive(Debug, Clone, Copy)]
pub struct ArchMeasured {
    /// Import-graph strongly-connected-component count.
    pub cycle_count: u32,
    /// Propagation cost (0..1 reach density).
    pub propagation_cost: f64,
}

/// [`evaluate_architecture_gate`] returning the measured metrics alongside
/// the violations. `None` when neither architecture gate is configured (the
/// import graph is not built at all).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors
/// (propagated from the import-graph build).
pub fn evaluate_architecture_gate_measured(
    thresholds: &Thresholds,
    db: &crate::facts::FactsDb,
) -> crate::Result<(Vec<GateViolation>, Option<ArchMeasured>)> {
    let g = &thresholds.gates;
    if g.max_dependency_cycles.is_none() && g.max_propagation_cost.is_none() {
        return Ok((Vec::new(), None));
    }
    let graph = crate::analyses::import_graph::build_import_graph(db)?;
    let m = crate::analyses::import_graph::graph_metrics(&graph);

    let mut out = Vec::new();
    if let Some(max) = g.max_dependency_cycles
        && m.cycle_count > max
    {
        out.push(GateViolation {
            gate: "max_dependency_cycles".into(),
            path: "(repo-wide)".into(),
            actual: m.cycle_count.to_string(),
            threshold: max.to_string(),
        });
    }
    if let Some(max) = g.max_propagation_cost
        && m.propagation_cost > max
    {
        out.push(GateViolation {
            gate: "max_propagation_cost".into(),
            path: "(repo-wide)".into(),
            actual: format!("{:.4}", m.propagation_cost),
            threshold: format!("{max:.4}"),
        });
    }
    Ok((
        out,
        Some(ArchMeasured {
            cycle_count: m.cycle_count,
            propagation_cost: m.propagation_cost,
        }),
    ))
}

/// Evaluate the `[diff]` section against a base→head delta.
///
/// `new_hotspot_count` is the number of files that newly enter the
/// top-N hotspot ranking at head (i.e. weren't in the base ranking).
/// `delta_code_health` is `head_median_health − base_median_health` —
/// a positive value means health improved, a negative value means it
/// dropped. Convention follows the `[diff] delta_code_health_min`
/// gate: the configured threshold is the floor for the delta, so the
/// gate fails when `delta_code_health < delta_code_health_min`.
///
/// Returns an empty vec when no `[diff]` gates are configured (the
/// caller's empty-thresholds short-circuit covers this, but the
/// internal early-return keeps the function safe to call from
/// downstream tooling that wires `[diff]` standalone).
#[must_use]
pub fn evaluate_diff_gate(
    thresholds: &Thresholds,
    new_hotspot_count: u32,
    delta_code_health: f64,
    base_cycles: u32,
    head_cycles: u32,
    delta_health_ratio: Option<f64>,
    delta_health_verdict: Option<&str>,
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    let d = &thresholds.diff;
    if let Some(max) = d.new_hotspot_max
        && new_hotspot_count > max
    {
        out.push(GateViolation {
            gate: "new_hotspot_max".into(),
            path: "(diff-summary)".into(),
            actual: new_hotspot_count.to_string(),
            threshold: max.to_string(),
        });
    }
    if let Some(min) = d.delta_code_health_min
        && delta_code_health < min
    {
        out.push(GateViolation {
            gate: "delta_code_health_min".into(),
            path: "(diff-summary)".into(),
            actual: format!("{delta_code_health:+.2}"),
            threshold: format!("{min:+.2}"),
        });
    }
    if d.no_new_cycles && head_cycles > base_cycles {
        out.push(GateViolation {
            gate: "no_new_cycles".into(),
            path: "(diff-summary)".into(),
            actual: format!("{head_cycles} cycles (base {base_cycles})"),
            threshold: format!("≤ {base_cycles}"),
        });
    }
    if let Some(min) = d.delta_health_min
        && let Some(ratio) = delta_health_ratio
        && ratio < min
    {
        out.push(GateViolation {
            gate: "delta_health_min".into(),
            path: "(diff-summary)".into(),
            actual: format!("{ratio:.1}"),
            threshold: format!("\u{2265} {min:.1}"),
        });
    }
    if d.deny_degrading_verdict && delta_health_verdict == Some("degrading") {
        out.push(GateViolation {
            gate: "deny_degrading_verdict".into(),
            path: "(diff-summary)".into(),
            actual: "degrading".into(),
            threshold: "verdict != degrading".into(),
        });
    }
    out
}

/// Evaluate the `[diff]` gates that apply to a working-tree change-set report
/// (`codelore gate` / the `gate_changes` MCP tool).
///
/// Four keys apply to the working-tree surface, with equal-passes
/// boundaries mirroring [`evaluate_diff_gate`]:
///
/// - `delta_code_health_min` — floor on the whole-repo-median delta
///   (`projected − baseline`), the same semantics the key carries on `diff`.
///   Skipped when either median is absent (no scoreable files); callers
///   surface the skip as a notice.
/// - `delta_code_health_min_per_file` — floor on each changed file's own
///   `projected − baseline` delta; one violation per offending file, with the
///   file's delta as the measured value.
/// - `new_file_health_min` — floor on each ADDED file's own projected score
///   (added files carry no delta, so they never reach the previous gate);
///   one violation per offending added file, with the file's projected
///   score as the measured value. Deleted files never trigger it.
/// - `no_new_cycles` — cyclic-node MEMBERSHIP comparison: one violation per
///   path that is cyclic in the projection but not at HEAD. This deliberately
///   diverges from `diff`'s cycle-count comparison — membership names the
///   files and still fires when two existing cycles merge into one bigger
///   tangle (which DROPS the count).
///
/// The remaining `[diff]` keys (`new_hotspot_max`, `delta_health_min`,
/// `deny_degrading_verdict`) are diff-only and never evaluated here.
#[must_use]
pub fn evaluate_gate_thresholds(
    thresholds: &Thresholds,
    report: &crate::change_set::ChangeSetReport,
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    let d = &thresholds.diff;
    if let Some(min) = d.delta_code_health_min
        && let (Some(base), Some(projected)) = (
            report.health.baseline_median,
            report.health.projected_median,
        )
    {
        let delta = projected - base;
        if delta < min {
            out.push(GateViolation {
                gate: "delta_code_health_min".into(),
                path: "(change-set)".into(),
                actual: format!("{delta:+.2}"),
                threshold: format!("{min:+.2}"),
            });
        }
    }
    if let Some(min) = d.delta_code_health_min_per_file {
        for row in &report.health.deltas {
            if let Some(delta) = row.delta
                && delta < min
            {
                out.push(GateViolation {
                    gate: "delta_code_health_min_per_file".into(),
                    path: row.path.clone(),
                    actual: format!("{delta:+.2}"),
                    threshold: format!("{min:+.2}"),
                });
            }
        }
    }
    if let Some(min) = d.new_file_health_min {
        for row in &report.health.deltas {
            // Added files (identified by the honest-absence reason, which also
            // covers rename destinations) carry no baseline delta, so the
            // per-file floor above never sees them. A deleted file has no
            // projected score and is filtered out by the `Some(score)` match.
            if row.reason.as_deref() == Some(crate::change_set::REASON_NEW_FILE)
                && let Some(score) = row.projected_score
                && score < min
            {
                out.push(GateViolation {
                    gate: "new_file_health_min".into(),
                    path: row.path.clone(),
                    actual: format!("{score:.1}"),
                    threshold: format!("{min:.1}"),
                });
            }
        }
    }
    if d.no_new_cycles {
        for path in &report.newly_cyclic_paths {
            out.push(GateViolation {
                gate: "no_new_cycles".into(),
                path: path.clone(),
                actual: "newly cyclic".into(),
                threshold: "no new cycles".into(),
            });
        }
    }
    out
}

/// Evaluate the `[gates]` section against a hotspots result set.
/// Returns the list of violations.
#[must_use]
pub fn evaluate_full_tree(
    thresholds: &Thresholds,
    hotspots: &[crate::analyses::hotspots::HotspotRow],
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    let g = &thresholds.gates;
    for row in hotspots {
        if let Some(max) = g.cognitive_max
            && row.cognitive > max
        {
            out.push(GateViolation {
                gate: "cognitive_max".into(),
                path: row.path.clone(),
                actual: format!("{:.0}", row.cognitive),
                threshold: format!("{max:.0}"),
            });
        }
        if let Some(max) = g.hotspot_score_max
            && row.hotspot_score > max
        {
            out.push(GateViolation {
                gate: "hotspot_score_max".into(),
                path: row.path.clone(),
                actual: format!("{:.2}", row.hotspot_score),
                threshold: format!("{max:.2}"),
            });
        }
    }
    out
}

/// Pure inner comparison for the `hotspot_anchored_max` gate.
///
/// One violation per file whose `hotspot_score_anchored` exceeds `max` (strictly
/// greater — equal-to-ceiling passes, mirroring the other `_max` gates). Rows
/// with no anchored score (uncovered language, or no calibration artifact
/// active) carry no comparison and never violate.
///
/// Kept out of [`evaluate_full_tree`] — which evaluates the always-on
/// `cognitive_max` / `hotspot_score_max` gates — because the anchored gate is
/// corpus-dependent and skippable. The **skip** path (no calibration active ⇒
/// every row is `None`) lives in the CLI layer; this function's all-`None`
/// result is simply an empty violation set, mirroring
/// [`evaluate_corpus_percentile_rows`].
#[must_use]
pub fn evaluate_hotspot_anchored_rows(
    max: f64,
    rows: &[crate::analyses::hotspots::HotspotRow],
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    for row in rows {
        if let Some(score) = row.hotspot_score_anchored
            && score > max
        {
            out.push(GateViolation {
                gate: "hotspot_anchored_max".into(),
                path: row.path.clone(),
                actual: format!("{score:.2}"),
                threshold: format!("{max:.2}"),
            });
        }
    }
    out
}

/// Evaluate the `code_health_min` gate against the COMPOSITE `code-health`
/// score (`run_code_health`), not the hotspots inline cognitive-only proxy.
/// This is the score `--analysis code-health` reports, so a file the analysis
/// bands `red` is the file the gate flags — the two agree.
///
/// Kept separate from [`evaluate_full_tree`] (which evaluates the genuinely
/// hotspot-scoped `cognitive_max` / `hotspot_score_max` gates) because
/// `code_health_min` is a property of the code-health analysis, whose file set
/// (files with complexity data) differs from the hotspots file set.
#[must_use]
pub fn evaluate_code_health_gate(
    thresholds: &Thresholds,
    code_health: &[crate::analyses::code_health::CodeHealthRow],
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    let Some(min) = thresholds.gates.code_health_min else {
        return out;
    };
    for row in code_health {
        if row.score < min {
            out.push(GateViolation {
                gate: "code_health_min".into(),
                path: row.path.clone(),
                actual: format!("{:.1}", row.score),
                threshold: format!("{min:.1}"),
            });
        }
    }
    out
}

/// Pure inner comparison for the `corpus_percentile_max` gate.
///
/// One violation per file whose `corpus_percentile` exceeds `max` (strictly
/// greater — equal-to-ceiling passes, mirroring the other `_max` gates). Rows
/// with no `corpus_percentile` (uncovered language, or no calibration active)
/// carry no comparison and never violate.
///
/// Public so the CLI layer can evaluate the corpus gate over the code-health
/// rows it already holds, without re-running the analysis. The **skip** path
/// (no calibration active ⇒ every row is `None`) lives in the CLI layer; this
/// function's all-`None` result is simply an empty violation set.
#[must_use]
pub fn evaluate_corpus_percentile_rows(
    max: f64,
    rows: &[crate::analyses::code_health::CodeHealthRow],
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    for row in rows {
        if let Some(pct) = row.corpus_percentile
            && pct > max
        {
            out.push(GateViolation {
                gate: "corpus_percentile_max".into(),
                path: row.path.clone(),
                actual: format!("{pct:.2}"),
                threshold: format!("{max:.2}"),
            });
        }
    }
    out
}

/// Pure inner comparison for the `max_red_effort_pct` gate.
///
/// Finds the `"red"` band row in `rows`; if none, treats churn as 0 %
/// (an all-green / all-yellow repo passes any positive threshold).
/// Public so callers that already hold the effort-exposure rows (the
/// `check` command computes them once for the gate, its ledger record,
/// and the ratchet) can evaluate without re-running the analysis.
#[must_use]
pub fn evaluate_effort_exposure_rows(
    threshold: f64,
    rows: &[crate::analyses::effort_exposure::EffortExposureRow],
) -> Vec<GateViolation> {
    evaluate_effort_exposure_rows_exempt(threshold, false, rows)
}

/// [`evaluate_effort_exposure_rows`] with the improving-churn exemption.
///
/// When `exempt_improving` is `false` this is byte-for-byte identical to
/// [`evaluate_effort_exposure_rows`] — the red band's full `churn_share_pct` is
/// compared against `threshold`, and the violation's `actual` is that value.
///
/// When `exempt_improving` is `true` AND the red row carries a decomposition
/// ([`churn_share_degrading_pct`] populated — see
/// [`run_effort_exposure_decomposed`]), only the *degrading* share is compared:
/// churn that landed in red files whose own health did not improve over the
/// window. The violation message discloses all three numbers ("red churn 18.30%
/// of which improving 12.10% exempt → 6.20% vs ceiling 15.00") so the exemption
/// is never silent. If the exemption is requested but the row has no
/// decomposition (analysis run without repo access), the gate falls back to the
/// full red share — the safe direction (no unearned exemption).
///
/// [`churn_share_degrading_pct`]: crate::analyses::effort_exposure::EffortExposureRow::churn_share_degrading_pct
/// [`run_effort_exposure_decomposed`]: crate::analyses::effort_exposure::run_effort_exposure_decomposed
#[must_use]
pub fn evaluate_effort_exposure_rows_exempt(
    threshold: f64,
    exempt_improving: bool,
    rows: &[crate::analyses::effort_exposure::EffortExposureRow],
) -> Vec<GateViolation> {
    let red = rows.iter().find(|r| r.band == "red");
    let total = red.map_or(0.0, |r| r.churn_share_pct);
    // The decomposed degrading share is only used when the exemption is on and
    // the row actually carries it; otherwise the comparison stays on `total`,
    // keeping the default path byte-identical to the pre-exemption behaviour.
    let degrading = red.and_then(|r| r.churn_share_degrading_pct);
    let (effective, decomposed) = match (exempt_improving, degrading) {
        (true, Some(d)) => (d, true),
        _ => (total, false),
    };
    if effective <= threshold {
        return Vec::new();
    }
    let actual = if decomposed {
        let improving = red.and_then(|r| r.churn_share_improving_pct).unwrap_or(0.0);
        format!("{effective:.2} (red {total:.2}, improving {improving:.2} exempt)")
    } else {
        format!("{effective:.2}")
    };
    vec![GateViolation {
        gate: "max_red_effort_pct".into(),
        path: "(repo-wide)".into(),
        actual,
        threshold: format!("{threshold:.2}"),
    }]
}

/// Evaluate the `max_red_effort_pct` gate: fails when the `red`
/// code-health band's window churn share exceeds the configured ceiling.
///
/// A noop (returns empty) when `max_red_effort_pct` is absent from the
/// thresholds — the analysis is not run at all.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] if `run_effort_exposure`
/// fails (e.g. `DuckDB` error during temp-table setup or SQL execution).
pub fn evaluate_effort_exposure_gate(
    thresholds: &Thresholds,
    db: &crate::facts::FactsDb,
    opts: &crate::Options,
) -> crate::Result<Vec<GateViolation>> {
    let Some(threshold) = thresholds.gates.max_red_effort_pct else {
        return Ok(Vec::new());
    };
    let rows = crate::analyses::effort_exposure::run_effort_exposure(db, opts)?;
    Ok(evaluate_effort_exposure_rows(threshold, &rows))
}

/// Pure inner comparison for the `code_familiarity_min` gate.
///
/// Public so callers that already hold the familiarity rows (the `check`
/// command reads the measured percentage for its ledger record from the
/// same rows) can evaluate without re-running the analysis.
#[must_use]
pub fn evaluate_familiarity_rows(
    threshold: f64,
    rows: &[crate::analyses::code_familiarity::CodeFamiliarityRow],
) -> Vec<GateViolation> {
    // Empty rows means no recognized source files — gate vacuously passes
    // (no SLOC to measure, so no familiarity risk to enforce).
    let Some(row) = rows.first() else {
        return Vec::new();
    };
    if row.familiarity_pct < threshold {
        vec![GateViolation {
            gate: "code_familiarity_min".into(),
            path: "(repo-wide)".into(),
            actual: format!("{:.2}", row.familiarity_pct),
            threshold: format!("{threshold:.2}"),
        }]
    } else {
        Vec::new()
    }
}

/// Evaluate the `code_familiarity_min` gate: fails when the repository's
/// active-team familiarity score falls below the configured floor.
///
/// A noop (returns empty) when `code_familiarity_min` is absent from the
/// thresholds. When the repo has no recognized source files (empty
/// `complexity_metrics`) `run_code_familiarity` returns an empty vec, which
/// this gate treats as vacuously passing.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] if `run_code_familiarity`
/// fails (e.g. `DuckDB` error during materialization or SQL execution).
pub fn evaluate_familiarity_gate(
    thresholds: &Thresholds,
    db: &crate::facts::FactsDb,
    opts: &crate::Options,
) -> crate::Result<Vec<GateViolation>> {
    let Some(threshold) = thresholds.gates.code_familiarity_min else {
        return Ok(Vec::new());
    };
    let rows = crate::analyses::code_familiarity::run_code_familiarity(db, opts)?;
    Ok(evaluate_familiarity_rows(threshold, &rows))
}

/// Pure inner comparison for the `max_findings_in_hot_files` gate.
///
/// Counts `"act-now"` rows in `rows`; returns a single repo-wide violation
/// when that count exceeds `threshold`. `actual` is the act-now count;
/// `path` is `"(repo-wide)"`.
///
/// Called from the CLI layer after opening the external store and running
/// `run_finding_hotspot_overlap`. The gate is **skipped** (not evaluated at
/// all) when the store is absent or empty — that skip path lives in the CLI
/// layer and does not go through this function.
#[must_use]
pub fn evaluate_finding_overlap_rows(
    threshold: u32,
    rows: &[crate::analyses::finding_hotspot_overlap::FindingHotspotOverlapRow],
) -> Vec<GateViolation> {
    let act_now_count = rows.iter().filter(|r| r.priority == "act-now").count();
    if act_now_count as u64 > u64::from(threshold) {
        vec![GateViolation {
            gate: "max_findings_in_hot_files".into(),
            path: "(repo-wide)".into(),
            actual: act_now_count.to_string(),
            threshold: threshold.to_string(),
        }]
    } else {
        Vec::new()
    }
}

/// Pure inner comparison for the two-band `[new_code]` gate over a
/// [`NewCodeScope`](crate::analyses::new_code::NewCodeScope).
///
/// Two independent bands, each evaluated only when the config enables it:
///
/// - **`born_health_min`** — one violation per born-in-window file whose HEAD
///   code-health score is below the floor (strict `<`, mirroring
///   `code_health_min`). Skipped when `born_health_min` is absent.
/// - **`touched_no_degradation`** — one violation per touched-but-not-born file
///   whose net window health movement is negative. The fail test is
///   `net < -f64::EPSILON`, so a file that held steady — the typo-fix case that
///   nets exactly zero — passes; only a genuine net-degradation fails. The
///   `f64::EPSILON` guard is the gate layer's established float-noise band (the
///   same one [`ratchet`](super::ratchet) uses); the *semantic* noise immunity
///   comes from the delta-health banding upstream, which nets sub-risk-band
///   churn to zero. Skipped when `touched_no_degradation` is off.
///
/// Each violation's `gate` names its band, so the rendered message discloses
/// which obligation a file missed ("born in window" vs "net … over Nd"). A noop
/// (empty) when neither band is enabled or the scope is empty (including the
/// shallow-history skip, which yields empty `born`/`touched`).
#[must_use]
pub fn evaluate_new_code_rows(
    cfg: &super::config::NewCodeGates,
    scope: &crate::analyses::new_code::NewCodeScope,
) -> Vec<GateViolation> {
    let mut out = Vec::new();
    // Born band: HEAD score floor on files first seen inside the window.
    if let Some(floor) = cfg.born_health_min {
        for (path, score) in &scope.born {
            if *score < floor {
                out.push(GateViolation {
                    gate: "born_health_min".into(),
                    path: path.clone(),
                    actual: format!("{score:.1} (born in window)"),
                    threshold: format!("{floor:.1}"),
                });
            }
        }
    }
    // Touched band: non-degradation of net window health movement on files
    // touched (but not born) inside the window.
    if cfg.touched_no_degradation {
        for (path, net) in &scope.touched {
            if *net < -f64::EPSILON {
                out.push(GateViolation {
                    gate: "touched_no_degradation".into(),
                    path: path.clone(),
                    actual: format!("net {net:+.1} over {}d", cfg.window_days),
                    threshold: "\u{2265} 0".into(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(
        path: &str,
        cognitive: f64,
        cognitive_health: f64,
        hotspot: f64,
    ) -> crate::analyses::hotspots::HotspotRow {
        crate::analyses::hotspots::HotspotRow {
            path: path.to_string(),
            revisions: 1,
            cognitive,
            cognitive_health,
            hotspot_score: hotspot,
            mi: None,
            mi_rank: None,
            ai_pct: None,
            hotspot_score_anchored: None,
        }
    }

    #[test]
    fn cognitive_max_flags_offending_file() {
        let mut t = Thresholds::default();
        t.gates.cognitive_max = Some(30.0);
        let rows = vec![
            make_row("a.rs", 40.0, 80.0, 1.0),
            make_row("b.rs", 20.0, 90.0, 1.0),
        ];
        let v = evaluate_full_tree(&t, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, "a.rs");
        assert_eq!(v[0].gate, "cognitive_max");
    }

    fn make_ch_row(path: &str, score: f64) -> crate::analyses::code_health::CodeHealthRow {
        crate::analyses::code_health::CodeHealthRow {
            path: path.to_string(),
            cognitive: 0.0,
            score,
            structural_risk: 0.0,
            percentile: 0.0,
            band: "green".to_string(),
            corpus_percentile: None,
            beyond_corpus: false,
            corpus_percentile_ci_low: None,
            corpus_percentile_ci_high: None,
        }
    }

    #[test]
    fn code_health_min_flags_offending_file() {
        let mut t = Thresholds::default();
        t.gates.code_health_min = Some(70.0);
        // The gate reads the COMPOSITE code-health score (not the hotspots
        // inline proxy): 50 < 70 fails, 85 passes.
        let rows = vec![make_ch_row("a.rs", 50.0), make_ch_row("b.rs", 85.0)];
        let v = evaluate_code_health_gate(&t, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, "a.rs");
        assert_eq!(v[0].gate, "code_health_min");
    }

    #[test]
    fn code_health_gate_vacuous_when_unconfigured() {
        let t = Thresholds::default();
        let rows = vec![make_ch_row("a.rs", 0.0)];
        assert!(evaluate_code_health_gate(&t, &rows).is_empty());
    }

    fn make_corpus_row(
        path: &str,
        corpus_percentile: Option<f64>,
    ) -> crate::analyses::code_health::CodeHealthRow {
        crate::analyses::code_health::CodeHealthRow {
            path: path.to_string(),
            cognitive: 0.0,
            score: 100.0,
            structural_risk: 0.0,
            percentile: 0.0,
            band: "green".to_string(),
            corpus_percentile,
            beyond_corpus: false,
            corpus_percentile_ci_low: None,
            corpus_percentile_ci_high: None,
        }
    }

    #[test]
    fn corpus_percentile_max_flags_file_above_ceiling() {
        // One file sits above the ceiling, one at it, one below, one absent.
        let rows = vec![
            make_corpus_row("hot.rs", Some(0.95)),
            make_corpus_row("edge.rs", Some(0.90)),
            make_corpus_row("cool.rs", Some(0.10)),
            make_corpus_row("unknown.rs", None),
        ];
        let v = evaluate_corpus_percentile_rows(0.90, &rows);
        assert_eq!(v.len(), 1, "only the strictly-above file violates: {v:?}");
        assert_eq!(v[0].path, "hot.rs");
        assert_eq!(v[0].gate, "corpus_percentile_max");
        assert_eq!(v[0].actual, "0.95");
        assert_eq!(v[0].threshold, "0.90");
    }

    #[test]
    fn corpus_percentile_max_boundary_is_strictly_greater() {
        // Equal-to-ceiling passes (`> max`, not `>= max`).
        let rows = vec![make_corpus_row("edge.rs", Some(0.90))];
        assert!(evaluate_corpus_percentile_rows(0.90, &rows).is_empty());
    }

    #[test]
    fn corpus_percentile_max_ignores_none_rows() {
        // A file with no corpus percentile (uncovered language / no calibration)
        // never violates, no matter how low the ceiling.
        let rows = vec![make_corpus_row("unknown.rs", None)];
        assert!(evaluate_corpus_percentile_rows(0.0, &rows).is_empty());
    }

    /// A hotspot row carrying a given anchored score (other fields inert).
    fn make_anchored_row(
        path: &str,
        anchored: Option<f64>,
    ) -> crate::analyses::hotspots::HotspotRow {
        crate::analyses::hotspots::HotspotRow {
            path: path.into(),
            revisions: 1,
            cognitive: 0.0,
            cognitive_health: 100.0,
            hotspot_score: 0.0,
            mi: None,
            mi_rank: None,
            ai_pct: None,
            hotspot_score_anchored: anchored,
        }
    }

    #[test]
    fn hotspot_anchored_max_flags_file_above_ceiling() {
        // Above / at / below the ceiling, plus an uncovered (None) row.
        let rows = vec![
            make_anchored_row("hot.rs", Some(9.50)),
            make_anchored_row("edge.rs", Some(9.00)),
            make_anchored_row("cool.rs", Some(1.00)),
            make_anchored_row("uncovered.rs", None),
        ];
        let v = evaluate_hotspot_anchored_rows(9.0, &rows);
        assert_eq!(v.len(), 1, "only the strictly-above file violates: {v:?}");
        assert_eq!(v[0].path, "hot.rs");
        assert_eq!(v[0].gate, "hotspot_anchored_max");
        assert_eq!(v[0].actual, "9.50");
        assert_eq!(v[0].threshold, "9.00");
    }

    #[test]
    fn hotspot_anchored_max_boundary_is_strictly_greater() {
        // Equal-to-ceiling passes (`> max`, not `>= max`).
        let rows = vec![make_anchored_row("edge.rs", Some(9.0))];
        assert!(evaluate_hotspot_anchored_rows(9.0, &rows).is_empty());
    }

    #[test]
    fn hotspot_anchored_max_ignores_none_rows() {
        // A file with no anchored score (uncovered language / no calibration)
        // never violates, no matter how low the ceiling — this is the skip
        // contract's data half; the CLI layer turns all-None into a skip.
        let rows = vec![make_anchored_row("uncovered.rs", None)];
        assert!(evaluate_hotspot_anchored_rows(0.0, &rows).is_empty());
    }

    #[test]
    fn empty_thresholds_never_violates() {
        let t = Thresholds::default();
        let rows = vec![make_row("a.rs", 9999.0, 0.0, 99.0)];
        let v = evaluate_full_tree(&t, &rows);
        assert!(v.is_empty());
    }

    // ───────────────── [diff] gate ─────────────────

    #[test]
    fn diff_gate_vacuous_when_unconfigured() {
        let t = Thresholds::default();
        let v = evaluate_diff_gate(&t, 999, -100.0, 0, 5, None, None);
        assert!(
            v.is_empty(),
            "no [diff] gates ⇒ no violations regardless of inputs"
        );
    }

    #[test]
    fn diff_gate_new_hotspot_max_flags_excess() {
        let mut t = Thresholds::default();
        t.diff.new_hotspot_max = Some(2);
        let v = evaluate_diff_gate(&t, 5, 0.0, 0, 0, None, None);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "new_hotspot_max");
        assert_eq!(v[0].actual, "5");
        assert_eq!(v[0].threshold, "2");
    }

    #[test]
    fn diff_gate_new_hotspot_max_boundary_passes() {
        // Equal-to-threshold is allowed (`> max`, not `>= max`).
        let mut t = Thresholds::default();
        t.diff.new_hotspot_max = Some(3);
        let v = evaluate_diff_gate(&t, 3, 0.0, 0, 0, None, None);
        assert!(v.is_empty());
    }

    #[test]
    fn diff_gate_delta_health_min_flags_drop() {
        // delta_code_health_min = -5 means "drop no more than 5 pts".
        // A delta of -10 violates; the gate's actual/threshold formatting
        // includes the sign so the human-readable output is unambiguous.
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min = Some(-5.0);
        let v = evaluate_diff_gate(&t, 0, -10.0, 0, 0, None, None);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "delta_code_health_min");
        assert_eq!(v[0].actual, "-10.00");
        assert_eq!(v[0].threshold, "-5.00");
    }

    #[test]
    fn diff_gate_delta_health_min_improvement_passes() {
        // Positive delta (health improved) trivially clears any floor.
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min = Some(-5.0);
        let v = evaluate_diff_gate(&t, 0, 3.0, 0, 0, None, None);
        assert!(v.is_empty());
    }

    #[test]
    fn diff_gate_both_violations_surface_independently() {
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min = Some(0.0);
        t.diff.new_hotspot_max = Some(0);
        let v = evaluate_diff_gate(&t, 2, -1.0, 0, 0, None, None);
        assert_eq!(v.len(), 2);
        let gates: Vec<&str> = v.iter().map(|g| g.gate.as_str()).collect();
        assert!(gates.contains(&"new_hotspot_max"));
        assert!(gates.contains(&"delta_code_health_min"));
    }

    #[test]
    fn diff_gate_no_new_cycles_flags_a_new_loop() {
        let mut t = Thresholds::default();
        t.diff.no_new_cycles = true;
        // head introduces a cycle the base didn't have → violation.
        let v = evaluate_diff_gate(&t, 0, 0.0, 1, 2, None, None);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "no_new_cycles");
        // Same or fewer cycles than base → clean (refactors that remove a
        // cycle, or leave the count unchanged, never fail this gate).
        assert!(evaluate_diff_gate(&t, 0, 0.0, 2, 2, None, None).is_empty());
        assert!(evaluate_diff_gate(&t, 0, 0.0, 3, 1, None, None).is_empty());
    }

    #[test]
    fn delta_health_min_gate_fires_below_floor() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        let v = evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(42.0), Some("indeterminate"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "delta_health_min");
    }

    #[test]
    fn delta_health_min_gate_passes_at_floor_and_skips_no_code_change() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        assert!(evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(50.0), Some("indeterminate")).is_empty());
        // no-code-change ⇒ ratio None ⇒ vacuous pass.
        assert!(evaluate_diff_gate(&t, 0, 0.0, 0, 0, None, Some("no-code-change")).is_empty());
    }

    #[test]
    fn deny_degrading_verdict_gate() {
        let t = Thresholds::from_text("[diff]\ndeny_degrading_verdict = true\n").unwrap();
        let v = evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(10.0), Some("degrading"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "deny_degrading_verdict");
        assert!(evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(60.0), Some("indeterminate")).is_empty());
    }

    // ───────────────── gate (working-tree) evaluator ─────────────────

    fn make_gate_delta(path: &str, delta: Option<f64>) -> crate::change_set::FileDelta {
        crate::change_set::FileDelta {
            path: path.to_string(),
            kind: "modified".to_string(),
            baseline_score: delta.map(|_| 50.0),
            projected_score: delta.map(|d| 50.0 + d),
            delta,
            baseline_band: None,
            projected_band: None,
            reason: delta.map_or_else(|| Some("deleted at gate time".to_string()), |_| None),
        }
    }

    fn make_gate_report(
        deltas: Vec<crate::change_set::FileDelta>,
        baseline_median: Option<f64>,
        projected_median: Option<f64>,
        newly_cyclic: Vec<String>,
    ) -> crate::change_set::ChangeSetReport {
        crate::change_set::ChangeSetReport {
            head_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
            merge_in_progress: false,
            changes: Vec::new(),
            health: crate::change_set::HealthProjection {
                deltas,
                baseline_median,
                projected_median,
            },
            base_cyclic_paths: Vec::new(),
            newly_cyclic_paths: newly_cyclic,
            coupling_absences: Vec::new(),
            findings: Vec::new(),
        }
    }

    #[test]
    fn gate_thresholds_vacuous_when_unconfigured() {
        let t = Thresholds::default();
        let report = make_gate_report(
            vec![make_gate_delta("a.rs", Some(-50.0))],
            Some(90.0),
            Some(10.0),
            vec!["b.rs".to_string()],
        );
        assert!(
            evaluate_gate_thresholds(&t, &report).is_empty(),
            "no [diff] gates ⇒ no violations regardless of the report"
        );
    }

    #[test]
    fn gate_median_floor_fires_below_min() {
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min = Some(-5.0);
        let report = make_gate_report(Vec::new(), Some(60.0), Some(50.0), Vec::new());
        let v = evaluate_gate_thresholds(&t, &report);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "delta_code_health_min");
        assert_eq!(v[0].path, "(change-set)");
        assert_eq!(v[0].actual, "-10.00");
        assert_eq!(v[0].threshold, "-5.00");
    }

    #[test]
    fn gate_median_floor_passes_at_boundary() {
        // Equal-passes: a median delta exactly at the floor is allowed.
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min = Some(-5.0);
        let report = make_gate_report(Vec::new(), Some(60.0), Some(55.0), Vec::new());
        assert!(evaluate_gate_thresholds(&t, &report).is_empty());
    }

    #[test]
    fn gate_median_floor_skipped_when_median_absent() {
        // No scoreable median on either side ⇒ the gate is skipped, not
        // failed (callers surface the skip as a notice).
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min = Some(0.0);
        let no_base = make_gate_report(Vec::new(), None, Some(50.0), Vec::new());
        assert!(evaluate_gate_thresholds(&t, &no_base).is_empty());
        let no_projected = make_gate_report(Vec::new(), Some(50.0), None, Vec::new());
        assert!(evaluate_gate_thresholds(&t, &no_projected).is_empty());
    }

    #[test]
    fn gate_per_file_floor_flags_each_offending_file() {
        // Path-level: one violation per file below the floor; files at/above
        // the floor and files with no delta (deleted / unscoreable) never fire.
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min_per_file = Some(0.0);
        let report = make_gate_report(
            vec![
                make_gate_delta("a.rs", Some(-2.5)),
                make_gate_delta("b.rs", Some(-0.1)),
                make_gate_delta("c.rs", Some(1.0)),
                make_gate_delta("d.rs", None),
            ],
            Some(50.0),
            Some(50.0),
            Vec::new(),
        );
        let v = evaluate_gate_thresholds(&t, &report);
        assert_eq!(v.len(), 2, "exactly the two below-floor files: {v:?}");
        assert!(v.iter().all(|x| x.gate == "delta_code_health_min_per_file"));
        assert_eq!(v[0].path, "a.rs");
        assert_eq!(v[0].actual, "-2.50");
        assert_eq!(v[0].threshold, "+0.00");
        assert_eq!(v[1].path, "b.rs");
    }

    #[test]
    fn gate_per_file_floor_passes_at_boundary() {
        // Equal-passes: a delta exactly at the floor is allowed.
        let mut t = Thresholds::default();
        t.diff.delta_code_health_min_per_file = Some(-1.0);
        let report = make_gate_report(
            vec![make_gate_delta("a.rs", Some(-1.0))],
            Some(50.0),
            Some(50.0),
            Vec::new(),
        );
        assert!(evaluate_gate_thresholds(&t, &report).is_empty());
    }

    #[test]
    fn gate_no_new_cycles_names_each_newly_cyclic_path() {
        let mut t = Thresholds::default();
        t.diff.no_new_cycles = true;
        let report = make_gate_report(
            Vec::new(),
            Some(50.0),
            Some(50.0),
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        );
        let v = evaluate_gate_thresholds(&t, &report);
        assert_eq!(v.len(), 2, "one violation per newly cyclic path: {v:?}");
        assert!(v.iter().all(|x| x.gate == "no_new_cycles"));
        assert_eq!(v[0].path, "src/a.rs");
        assert_eq!(v[1].path, "src/b.rs");
        // No newly cyclic paths ⇒ clean, even with the gate on.
        let clean = make_gate_report(Vec::new(), Some(50.0), Some(50.0), Vec::new());
        assert!(evaluate_gate_thresholds(&t, &clean).is_empty());
    }

    #[test]
    fn gate_ignores_diff_only_keys() {
        // new_hotspot_max / delta_health_min / deny_degrading_verdict are
        // diff-only: the working-tree evaluator never fires them.
        let mut t = Thresholds::default();
        t.diff.new_hotspot_max = Some(0);
        t.diff.delta_health_min = Some(100.0);
        t.diff.deny_degrading_verdict = true;
        let report = make_gate_report(
            vec![make_gate_delta("a.rs", Some(-50.0))],
            Some(90.0),
            Some(10.0),
            Vec::new(),
        );
        assert!(evaluate_gate_thresholds(&t, &report).is_empty());
    }

    /// An ADDED file's delta row: baseline absent, projected present, no delta,
    /// tagged with the new-file honest-absence reason — the exact shape the
    /// engine produces for a freshly added file.
    fn make_added_delta(path: &str, projected_score: f64) -> crate::change_set::FileDelta {
        crate::change_set::FileDelta {
            path: path.to_string(),
            kind: "added".to_string(),
            baseline_score: None,
            projected_score: Some(projected_score),
            delta: None,
            baseline_band: None,
            projected_band: Some("red".to_string()),
            reason: Some(crate::change_set::REASON_NEW_FILE.to_string()),
        }
    }

    #[test]
    fn gate_new_file_floor_flags_low_added_file() {
        // A newly added low-health file evades both delta floors (it carries no
        // delta), so the whole-repo median barely moves and the per-file floor
        // skips it. new_file_health_min catches it on projected score alone.
        let mut t = Thresholds::default();
        t.diff.new_file_health_min = Some(50.0);
        let report = make_gate_report(
            vec![
                make_added_delta("src/god_class.rs", 20.0), // below floor → violation
                make_added_delta("src/tidy.rs", 80.0),      // above floor → clean
                make_gate_delta("src/edited.rs", Some(-40.0)), // modified, has delta → ignored
                make_gate_delta("src/gone.rs", None),       // deleted (no projected) → ignored
            ],
            Some(60.0),
            Some(59.0),
            Vec::new(),
        );
        let v = evaluate_gate_thresholds(&t, &report);
        assert_eq!(
            v.len(),
            1,
            "only the below-floor added file violates: {v:?}"
        );
        assert_eq!(v[0].gate, "new_file_health_min");
        assert_eq!(v[0].path, "src/god_class.rs");
        assert_eq!(v[0].actual, "20.0");
        assert_eq!(v[0].threshold, "50.0");
    }

    #[test]
    fn gate_new_file_floor_absent_key_is_byte_identical_noop() {
        // The SAME report with no new_file_health_min key must produce zero
        // new-file violations — added files stay unenforced by default.
        let t = Thresholds::default();
        let report = make_gate_report(
            vec![make_added_delta("src/god_class.rs", 1.0)],
            Some(60.0),
            Some(60.0),
            Vec::new(),
        );
        assert!(
            evaluate_gate_thresholds(&t, &report).is_empty(),
            "no new_file_health_min ⇒ no new-file gate at all"
        );
    }

    #[test]
    fn gate_new_file_floor_passes_at_boundary() {
        // Equal-passes: a projected score exactly at the floor is allowed.
        let mut t = Thresholds::default();
        t.diff.new_file_health_min = Some(50.0);
        let report = make_gate_report(
            vec![make_added_delta("src/edge.rs", 50.0)],
            Some(50.0),
            Some(50.0),
            Vec::new(),
        );
        assert!(evaluate_gate_thresholds(&t, &report).is_empty());
    }

    #[test]
    fn gate_new_file_floor_never_fires_on_deleted_file() {
        // A deleted file has no projected score; even a floor of 100 must not
        // fire on it (the reason is REASON_DELETED, and projected_score is None).
        let mut t = Thresholds::default();
        t.diff.new_file_health_min = Some(100.0);
        let report = make_gate_report(
            vec![make_gate_delta("src/gone.rs", None)],
            Some(50.0),
            Some(50.0),
            Vec::new(),
        );
        assert!(
            evaluate_gate_thresholds(&t, &report).is_empty(),
            "deleted files never trip the new-file floor"
        );
    }

    // ───────── max_red_effort_pct gate ─────────

    fn make_effort_row(
        band: &str,
        churn_share_pct: f64,
    ) -> crate::analyses::effort_exposure::EffortExposureRow {
        crate::analyses::effort_exposure::EffortExposureRow {
            band: band.to_string(),
            files: 1,
            loc_share_pct: 0.0,
            commit_share_pct: 0.0,
            churn_share_pct,
            commit_share_ci_low: 0.0,
            commit_share_ci_high: 0.0,
            churn_share_improving_pct: None,
            churn_share_degrading_pct: None,
        }
    }

    /// A red-band row carrying the improving/degrading decomposition, for the
    /// exemption evaluator tests.
    fn make_decomposed_red_row(
        churn_share_pct: f64,
        improving: f64,
        degrading: f64,
    ) -> crate::analyses::effort_exposure::EffortExposureRow {
        crate::analyses::effort_exposure::EffortExposureRow {
            band: "red".to_string(),
            files: 1,
            loc_share_pct: 0.0,
            commit_share_pct: 0.0,
            churn_share_pct,
            commit_share_ci_low: 0.0,
            commit_share_ci_high: 0.0,
            churn_share_improving_pct: Some(improving),
            churn_share_degrading_pct: Some(degrading),
        }
    }

    #[test]
    fn effort_exposure_gate_vacuous_when_unconfigured() {
        // When max_red_effort_pct is absent the gate must not fire regardless
        // of the row content (analysis never even runs in the real evaluator).
        let rows = vec![make_effort_row("red", 99.0)];
        let v = evaluate_effort_exposure_rows(f64::MAX, &rows);
        // Threshold f64::MAX means "nothing ever exceeds" — gate is vacuous.
        assert!(v.is_empty());
        // Also verify: is_empty() returns true when the field is None.
        let t = Thresholds::default();
        assert!(t.is_empty(), "default thresholds must be empty");
        let t = Thresholds::from_text("[gates]\nmax_red_effort_pct = 30.0\n").unwrap();
        assert!(
            !t.is_empty(),
            "max_red_effort_pct alone makes thresholds non-empty"
        );
    }

    #[test]
    fn effort_exposure_rows_fails_when_red_churn_exceeds_threshold() {
        // Red band has 50 % churn; threshold is 30 % → violation.
        let rows = vec![make_effort_row("red", 50.0), make_effort_row("green", 50.0)];
        let v = evaluate_effort_exposure_rows(30.0, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "max_red_effort_pct");
        assert_eq!(v[0].actual, "50.00");
        assert_eq!(v[0].threshold, "30.00");
        assert_eq!(v[0].path, "(repo-wide)");
    }

    #[test]
    fn effort_exposure_rows_passes_at_boundary() {
        // Exactly equal to the threshold is allowed (strictly greater fails).
        let rows = vec![make_effort_row("red", 30.0)];
        assert!(evaluate_effort_exposure_rows(30.0, &rows).is_empty());
    }

    #[test]
    fn effort_exposure_rows_passes_when_no_red_band() {
        // No red band row ⇒ 0.0 % red churn ⇒ passes any positive threshold.
        let rows = vec![make_effort_row("green", 100.0)];
        assert!(evaluate_effort_exposure_rows(0.001, &rows).is_empty());
        // And trivially passes threshold = 0.0 too (0.0 is not > 0.0).
        assert!(evaluate_effort_exposure_rows(0.0, &rows).is_empty());
    }

    #[test]
    fn effort_exposure_rows_passes_at_threshold_100() {
        // 100 % ceiling is the upper bound — even a 100% red-band repo passes.
        let rows = vec![make_effort_row("red", 100.0)];
        assert!(evaluate_effort_exposure_rows(100.0, &rows).is_empty());
    }

    // ───────── improving-churn exemption ─────────

    #[test]
    fn exempt_off_is_byte_identical_to_plain_evaluator() {
        // With the exemption off, the exempt evaluator must reproduce the plain
        // one exactly, even when a decomposition is present on the row.
        let rows = vec![make_decomposed_red_row(50.0, 40.0, 10.0)];
        let plain = evaluate_effort_exposure_rows(30.0, &rows);
        let exempt_off = evaluate_effort_exposure_rows_exempt(30.0, false, &rows);
        assert_eq!(format!("{plain:?}"), format!("{exempt_off:?}"));
        // Full 50 % share compared (not the 10 % degrading share) → fails at 30.
        assert_eq!(exempt_off.len(), 1);
        assert_eq!(exempt_off[0].actual, "50.00");
    }

    #[test]
    fn exempt_on_gates_only_the_degrading_share() {
        // red 50 %, of which 40 % improving / 10 % degrading. Ceiling 30 %:
        // total (50) would fail, but the degrading share (10) passes.
        let rows = vec![make_decomposed_red_row(50.0, 40.0, 10.0)];
        assert!(
            evaluate_effort_exposure_rows_exempt(30.0, true, &rows).is_empty(),
            "degrading share 10 % ≤ ceiling 30 % must pass under the exemption"
        );
    }

    #[test]
    fn exempt_on_violation_discloses_all_three_numbers() {
        // Degrading share above the ceiling still fails; the message carries the
        // degrading, total, and improving shares so the exemption is not silent.
        let rows = vec![make_decomposed_red_row(50.0, 10.0, 40.0)];
        let v = evaluate_effort_exposure_rows_exempt(30.0, true, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "max_red_effort_pct");
        assert_eq!(v[0].threshold, "30.00");
        assert_eq!(v[0].actual, "40.00 (red 50.00, improving 10.00 exempt)");
    }

    #[test]
    fn exempt_on_without_decomposition_falls_back_to_total() {
        // Exemption requested but the row carries no split (analysis ran without
        // repo access) → the gate compares the full red share, the safe
        // direction, and the message stays the plain single-number form.
        let rows = vec![make_effort_row("red", 50.0)];
        let v = evaluate_effort_exposure_rows_exempt(30.0, true, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].actual, "50.00",
            "no unearned exemption without a split"
        );
    }

    // ───────── code_familiarity_min gate ─────────

    fn make_familiarity_row(
        familiarity_pct: f64,
    ) -> crate::analyses::code_familiarity::CodeFamiliarityRow {
        crate::analyses::code_familiarity::CodeFamiliarityRow {
            scope: "repo".into(),
            familiarity_pct,
            active_authors: 1,
            total_authors: 1,
            islands_pct: 0.0,
            verdict: (if familiarity_pct >= 70.0 {
                "good"
            } else {
                "risky"
            })
            .into(),
        }
    }

    #[test]
    fn familiarity_gate_fires_when_score_below_threshold() {
        // familiarity_pct = 60.0, threshold = 70.0 → violation.
        let rows = vec![make_familiarity_row(60.0)];
        let v = evaluate_familiarity_rows(70.0, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "code_familiarity_min");
        assert_eq!(v[0].actual, "60.00");
        assert_eq!(v[0].threshold, "70.00");
        assert_eq!(v[0].path, "(repo-wide)");
    }

    #[test]
    fn familiarity_gate_passes_at_boundary() {
        // Exactly at threshold is allowed (strictly less fails).
        let rows = vec![make_familiarity_row(70.0)];
        assert!(evaluate_familiarity_rows(70.0, &rows).is_empty());
    }

    #[test]
    fn familiarity_gate_vacuous_when_rows_empty() {
        // No recognized source files → no rows → gate passes vacuously.
        assert!(evaluate_familiarity_rows(70.0, &[]).is_empty());
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn familiarity_gate_integration_passes_at_threshold_0() {
        // Full pipeline: ingest delivery_repo and verify the gate passes at
        // threshold 0.0 (floor). delivery_repo has 3 active authors and all
        // files touched within the 90-day window — familiarity_pct = 100.0.
        use crate::Options;
        use crate::facts::FactsDb;
        use crate::repo::GixRepo;

        let delivery = crate::test_support::delivery_repo::build();
        let repo = GixRepo::open(delivery.dir.path()).expect("open repo");
        let db = FactsDb::new_in_memory().expect("db");
        let opts = Options {
            repo_path: delivery.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");

        let thresholds =
            Thresholds::from_text("[gates]\ncode_familiarity_min = 0.0\n").expect("parse");
        let v = evaluate_familiarity_gate(&thresholds, &db, &opts).expect("evaluate gate");
        assert!(
            v.is_empty(),
            "threshold 0.0 must pass on delivery_repo: {v:?}"
        );
    }

    // ───────── max_findings_in_hot_files gate ─────────

    fn make_overlap_row(
        path: &str,
        priority: &str,
    ) -> crate::analyses::finding_hotspot_overlap::FindingHotspotOverlapRow {
        crate::analyses::finding_hotspot_overlap::FindingHotspotOverlapRow {
            path: path.to_string(),
            findings: 1,
            engines: "semgrep".to_string(),
            worst_level: "warning".to_string(),
            hotspot_score: 5.0,
            revs_percentile: 0.9,
            health_band: "red".to_string(),
            priority: priority.to_string(),
        }
    }

    #[test]
    fn finding_overlap_gate_fires_when_act_now_count_exceeds_threshold() {
        // threshold = 0, one act-now row → violation.
        let rows = vec![make_overlap_row("src/main.rs", "act-now")];
        let v = evaluate_finding_overlap_rows(0, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "max_findings_in_hot_files");
        assert_eq!(v[0].path, "(repo-wide)");
        assert_eq!(v[0].actual, "1");
        assert_eq!(v[0].threshold, "0");
    }

    #[test]
    fn finding_overlap_gate_passes_when_act_now_count_at_threshold() {
        // Exactly at threshold is allowed (strictly greater fails).
        let rows = vec![
            make_overlap_row("src/main.rs", "act-now"),
            make_overlap_row("src/lib.rs", "act-now"),
        ];
        // threshold = 2, count = 2 → passes (2 is not > 2).
        assert!(evaluate_finding_overlap_rows(2, &rows).is_empty());
    }

    #[test]
    fn finding_overlap_gate_ignores_non_act_now_rows() {
        // "plan" and "note" rows do not count towards the threshold.
        let rows = vec![
            make_overlap_row("src/main.rs", "plan"),
            make_overlap_row("src/lib.rs", "note"),
        ];
        let v = evaluate_finding_overlap_rows(0, &rows);
        assert!(v.is_empty(), "plan/note rows must not trigger the gate");
    }

    #[test]
    fn finding_overlap_gate_measured_value_is_act_now_count() {
        // The `actual` field records the raw act-now count for the ledger.
        let rows = vec![
            make_overlap_row("a.rs", "act-now"),
            make_overlap_row("b.rs", "act-now"),
            make_overlap_row("c.rs", "plan"),
        ];
        let v = evaluate_finding_overlap_rows(0, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].actual, "2", "actual must reflect only act-now count");
    }

    #[test]
    fn empty_code_health_rows_with_threshold_yields_no_lib_violations() {
        // When the analysis returns no rows the lib-level evaluator emits
        // no violations — the degraded detection and optional violation
        // injection live in the CLI layer (eval_code_health_gate helper).
        // This test pins that the pure-lib function is not itself the source
        // of a false-positive violation on empty input.
        let t = Thresholds::from_text("[gates]\ncode_health_min = 50.0\n").unwrap();
        let v = evaluate_code_health_gate(&t, &[]);
        assert!(v.is_empty(), "no violations from empty rows: {v:?}");
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn effort_exposure_gate_integration_passes_at_threshold_100() {
        // Full pipeline: ingest biomarker_repo and verify the gate with a
        // permissive threshold (100 %) passes cleanly. This proves the
        // evaluator wiring is correct end-to-end without depending on which
        // specific bands the fixture produces.
        use crate::Options;
        use crate::facts::FactsDb;
        use crate::repo::GixRepo;

        let bio = crate::test_support::biomarker_repo::build();
        let repo = GixRepo::open(bio.dir.path()).expect("open repo");
        let db = FactsDb::new_in_memory().expect("db");
        let opts = Options {
            repo_path: bio.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");

        let thresholds =
            Thresholds::from_text("[gates]\nmax_red_effort_pct = 100.0\n").expect("parse");
        let v = evaluate_effort_exposure_gate(&thresholds, &db, &opts).expect("evaluate gate");
        assert!(v.is_empty(), "threshold 100 must pass: {v:?}");
    }

    // ───────── evaluate_new_code_rows (two-band [new_code] gate) ─────────

    use crate::analyses::new_code::NewCodeScope;
    use crate::quality_gates::NewCodeGates;

    /// A config with both bands active: born floor 60, touched non-degradation
    /// on, over a 90-day window.
    fn nc_cfg() -> NewCodeGates {
        NewCodeGates {
            window_days: 90,
            born_health_min: Some(60.0),
            touched_no_degradation: true,
        }
    }

    #[test]
    fn new_code_born_below_floor_violates_with_born_message() {
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![("src/fresh.rs".into(), 41.2)],
            touched: vec![],
        };
        let v = evaluate_new_code_rows(&nc_cfg(), &scope);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "born_health_min");
        assert_eq!(v[0].path, "src/fresh.rs");
        assert!(
            v[0].actual.contains("born in window") && v[0].actual.contains("41.2"),
            "born violation discloses the band + score: {}",
            v[0].actual
        );
        assert_eq!(v[0].threshold, "60.0");
    }

    #[test]
    fn new_code_born_at_or_above_floor_passes() {
        // Boundary: score == floor passes (strict `<`), and a healthy born file
        // passes.
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![("at.rs".into(), 60.0), ("above.rs".into(), 80.0)],
            touched: vec![],
        };
        assert!(evaluate_new_code_rows(&nc_cfg(), &scope).is_empty());
    }

    #[test]
    fn new_code_born_band_skipped_when_floor_absent() {
        // No born_health_min ⇒ even a low-health born file is not flagged (only
        // the touched band would apply).
        let cfg = NewCodeGates {
            born_health_min: None,
            ..nc_cfg()
        };
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![("low.rs".into(), 3.0)],
            touched: vec![],
        };
        assert!(evaluate_new_code_rows(&cfg, &scope).is_empty());
    }

    #[test]
    fn new_code_touched_net_negative_violates_with_touched_message() {
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![],
            touched: vec![("src/legacy.rs".into(), -30.0)],
        };
        let v = evaluate_new_code_rows(&nc_cfg(), &scope);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "touched_no_degradation");
        assert_eq!(v[0].path, "src/legacy.rs");
        assert!(
            v[0].actual.contains("net -30.0") && v[0].actual.contains("90d"),
            "touched violation discloses net movement + window: {}",
            v[0].actual
        );
    }

    #[test]
    fn new_code_touched_net_zero_passes_typo_fix() {
        // A window that touched a file without moving any function across a risk
        // band nets exactly zero (the delta-health banding is the noise filter)
        // and must pass — the typo-fix-in-a-monolith case.
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![],
            touched: vec![("mono.rs".into(), 0.0)],
        };
        assert!(evaluate_new_code_rows(&nc_cfg(), &scope).is_empty());
    }

    #[test]
    fn new_code_touched_net_positive_passes() {
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![],
            touched: vec![("improved.rs".into(), 42.0)],
        };
        assert!(evaluate_new_code_rows(&nc_cfg(), &scope).is_empty());
    }

    #[test]
    fn new_code_touched_epsilon_boundary() {
        // The fail test is `net < -f64::EPSILON`: a float-noise-scale negative
        // passes (inside the gate layer's established noise band), while any real
        // degradation — the smallest possible being a single one-line function
        // crossing a band — fails. Net movement is a sum of exact-integer LOC
        // weights, so in practice the only "inside epsilon" value is zero.
        let inside = NewCodeScope {
            window_start_present: true,
            born: vec![],
            touched: vec![("noise.rs".into(), -f64::EPSILON * 0.5)],
        };
        assert!(
            evaluate_new_code_rows(&nc_cfg(), &inside).is_empty(),
            "a sub-epsilon negative is float noise and passes"
        );
        let real = NewCodeScope {
            window_start_present: true,
            born: vec![],
            touched: vec![("real.rs".into(), -1.0)],
        };
        assert_eq!(
            evaluate_new_code_rows(&nc_cfg(), &real).len(),
            1,
            "a real net degradation fails"
        );
    }

    #[test]
    fn new_code_touched_band_skipped_when_off() {
        let cfg = NewCodeGates {
            touched_no_degradation: false,
            ..nc_cfg()
        };
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![],
            touched: vec![("rotting.rs".into(), -99.0)],
        };
        assert!(evaluate_new_code_rows(&cfg, &scope).is_empty());
    }

    #[test]
    fn new_code_both_bands_flag_independently() {
        // A born-unhealthy file and a touched-degraded file each surface their
        // own band's violation; an untouched legacy file never enters the scope
        // (run_new_code_scope only carries born/touched), so it is exempt here by
        // construction.
        let scope = NewCodeScope {
            window_start_present: true,
            born: vec![("new_bad.rs".into(), 20.0), ("new_ok.rs".into(), 90.0)],
            touched: vec![
                ("touched_bad.rs".into(), -12.0),
                ("touched_ok.rs".into(), 5.0),
            ],
        };
        let v = evaluate_new_code_rows(&nc_cfg(), &scope);
        assert_eq!(v.len(), 2);
        assert!(
            v.iter()
                .any(|x| x.gate == "born_health_min" && x.path == "new_bad.rs")
        );
        assert!(
            v.iter()
                .any(|x| x.gate == "touched_no_degradation" && x.path == "touched_bad.rs")
        );
    }

    #[test]
    fn new_code_shallow_history_scope_yields_no_violations() {
        // The shallow-history skip: run_new_code_scope reports empty born/touched
        // with window_start_present=false; the evaluator has nothing to flag.
        let scope = NewCodeScope::default();
        assert!(!scope.window_start_present);
        assert!(evaluate_new_code_rows(&nc_cfg(), &scope).is_empty());
    }
}
