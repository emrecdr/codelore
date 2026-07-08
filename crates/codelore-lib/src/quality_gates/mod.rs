//! Quality-gates: parse `.codelore-thresholds.toml` at the repo root
//! and evaluate a fact store against the declared gates. Used by
//! `codelore check` to power CI quality-gate enforcement.
//!
//! ## Config schema
//!
//! ```toml
//! [gates]
//! cognitive_max = 30        # any file exceeding fails
//! code_health_min = 60      # any file below fails (composite code-health score, 0..=100)
//! hotspot_score_max = 8.0   # any file above fails
//! disallow_clone_type_1 = true
//! max_dependency_cycles = 0 # no import-graph cycles allowed
//! max_propagation_cost = 0.15  # change-reach ceiling (0..1)
//! max_red_effort_pct = 30.0    # red-band churn ceiling (%, [0, 100])
//!
//! [diff]
//! delta_code_health_min = -5  # health may drop at most 5 pts in a PR
//! new_hotspot_max = 0         # zero new hotspots allowed
//! no_new_cycles = true        # a PR may not introduce a dependency cycle
//! ```
//!
//! ## Why thresholds-in-repo vs CLI flags
//!
//! Thresholds live with the *codebase*, not the *invocation*. That
//! means the gate is the same whether a contributor's pre-push hook,
//! GitHub Actions, an IDE plugin, or a release pipeline runs the
//! check. Per the [`feedback_modernize_dont_migrate`](../../../.claude/memory/feedback_modernize_dont_migrate.md)
//! memory: thresholds files predate `CodeLore` (`CodeMaat` reads CSV
//! rules); ours integrates with `--group-file`, `--mailmap`, and the
//! existing convention-naming pattern — same data, deeper DX.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::{CodeLoreError, Result};

/// Conventional filename auto-discovered at the repo root.
pub const THRESHOLDS_FILENAME: &str = ".codelore-thresholds.toml";

// `deny_unknown_fields` on all three structs so a typo in the user's
// `.codelore-thresholds.toml` surfaces as a parse error instead of
// silently disabling the gate. Without this, `cognative_max` (typo of
// `cognitive_max`) or `disallow_clone_type1` (missing underscore)
// parses as the default `None`/`false` — the gate appears wired but
// does nothing on every PR.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    #[serde(default)]
    pub gates: Gates,
    #[serde(default)]
    pub diff: DiffGates,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gates {
    /// Maximum cognitive complexity per file. Files exceeding fail
    /// the gate.
    pub cognitive_max: Option<f64>,
    /// Minimum code-health score per file. Files below fail.
    pub code_health_min: Option<f64>,
    /// Maximum hotspot score per file.
    pub hotspot_score_max: Option<f64>,
    /// Disallow ANY Type-1 clone families.
    #[serde(default)]
    pub disallow_clone_type_1: bool,
    /// Maximum number of dependency cycles (non-trivial import-graph
    /// SCCs) allowed repo-wide. Exceeding fails the gate. `0` enforces a
    /// fully acyclic architecture.
    pub max_dependency_cycles: Option<u32>,
    /// Maximum propagation cost (import-graph transitive-closure density,
    /// `[0,1]`). Exceeding fails — a ceiling on "a change to a random
    /// file reaches this fraction of the system".
    pub max_propagation_cost: Option<f64>,
    /// Maximum share of window LOC churn (lines added + deleted) allowed
    /// to land in the `red` code-health band, as a percentage (`[0, 100]`).
    /// Exceeding fails — enforces "don't spend most engineering effort
    /// fighting fires in the worst-health files." Missing red band
    /// (zero red files) counts as 0 %, which passes any positive threshold.
    pub max_red_effort_pct: Option<f64>,
    /// Minimum code-familiarity score (`[0, 100]`). Below this threshold
    /// the `code-familiarity` analysis emits a `"risky"` verdict. When
    /// absent the analysis applies a built-in default of 70.0.
    pub code_familiarity_min: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffGates {
    /// Maximum allowed drop in median code-health between base and
    /// head. A drop of more than this magnitude fails the gate.
    pub delta_code_health_min: Option<f64>,
    /// Maximum number of NEW hotspots a PR may introduce.
    pub new_hotspot_max: Option<u32>,
    /// When true, a PR that introduces a NEW dependency cycle (more
    /// import-graph cycles at head than at base) fails the gate — the
    /// "don't let me merge a cycle" guard.
    #[serde(default)]
    pub no_new_cycles: bool,
    /// Minimum delta-health ratio (0–100): the share of changed-function
    /// weight ending low-risk or improved. A ratio below this fails.
    /// Skipped entirely on `no-code-change` diffs (no ratio, no signal).
    pub delta_health_min: Option<f64>,
    /// When true, a `degrading` delta-health verdict fails the gate.
    #[serde(default)]
    pub deny_degrading_verdict: bool,
}

impl Thresholds {
    /// Auto-discover `.codelore-thresholds.toml` at the repo root.
    /// Returns the default (no gates configured) when the file is
    /// absent — gates are opt-in.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Analysis`] on I/O or parse errors.
    pub fn discover(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join(THRESHOLDS_FILENAME);
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::from_path(&path)
    }

    /// Parse a thresholds file from disk.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Analysis`] on I/O or parse errors.
    pub fn from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).map_err(|e| {
            // Read-side input failure (unreadable `--thresholds-file`) →
            // exit 3, mirroring `team_map::load`. The parse failure below
            // stays `Analysis` (exit 4).
            CodeLoreError::RepoIo(std::io::Error::new(
                e.kind(),
                format!("read thresholds {}: {e}", path.display()),
            ))
        })?;
        Self::from_text(&raw).map_err(|e| {
            CodeLoreError::Analysis(format!("parse thresholds {}: {e}", path.display()))
        })
    }

    /// Parse from in-memory TOML text. Used by tests + `from_path`.
    ///
    /// # Errors
    ///
    /// Returns a `String` description of the parse failure.
    pub fn from_text(raw: &str) -> std::result::Result<Self, String> {
        toml::from_str(raw).map_err(|e| e.to_string())
    }

    /// True when no gate is configured. Callers can short-circuit
    /// the check entirely on empty config.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gates.cognitive_max.is_none()
            && self.gates.code_health_min.is_none()
            && self.gates.hotspot_score_max.is_none()
            && !self.gates.disallow_clone_type_1
            && self.gates.max_dependency_cycles.is_none()
            && self.gates.max_propagation_cost.is_none()
            && self.gates.max_red_effort_pct.is_none()
            && self.gates.code_familiarity_min.is_none()
            && self.diff.delta_code_health_min.is_none()
            && self.diff.new_hotspot_max.is_none()
            && !self.diff.no_new_cycles
            && self.diff.delta_health_min.is_none()
            && !self.diff.deny_degrading_verdict
    }
}

/// One detected gate violation.
#[derive(Debug, Clone)]
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
    let g = &thresholds.gates;
    if g.max_dependency_cycles.is_none() && g.max_propagation_cost.is_none() {
        return Ok(Vec::new());
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
    Ok(out)
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

/// Pure inner comparison for the `max_red_effort_pct` gate.
///
/// Finds the `"red"` band row in `rows`; if none, treats churn as 0 %
/// (an all-green / all-yellow repo passes any positive threshold).
pub(crate) fn evaluate_effort_exposure_rows(
    threshold: f64,
    rows: &[crate::analyses::effort_exposure::EffortExposureRow],
) -> Vec<GateViolation> {
    let actual = rows
        .iter()
        .find(|r| r.band == "red")
        .map_or(0.0, |r| r.churn_share_pct);
    if actual > threshold {
        vec![GateViolation {
            gate: "max_red_effort_pct".into(),
            path: "(repo-wide)".into(),
            actual: format!("{actual:.2}"),
            threshold: format!("{threshold:.2}"),
        }]
    } else {
        Vec::new()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_default() {
        let t = Thresholds::from_text("").unwrap();
        assert!(t.is_empty());
    }

    #[test]
    fn unknown_key_at_root_is_rejected() {
        // A typo at the root table level (e.g. `gate` instead of
        // `gates`) must fail the parse, not silently drop the value.
        let raw = "[gate]\ncognitive_max = 30\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd table should reject");
        assert!(
            err.contains("unknown field") || err.contains("gate"),
            "expected 'unknown field' in error, got: {err}"
        );
    }

    #[test]
    fn unknown_key_in_gates_is_rejected() {
        // `cognative_max` (transposed letters) used to parse as
        // default-disabled — the exact failure mode this guards against.
        let raw = "[gates]\ncognative_max = 30\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd gate key should reject");
        assert!(
            err.contains("unknown field") || err.contains("cognative"),
            "expected 'unknown field' in error, got: {err}"
        );
    }

    #[test]
    fn unknown_key_in_diff_is_rejected() {
        let raw = "[diff]\nnew_hotspot_maximum = 5\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd diff key should reject");
        assert!(
            err.contains("unknown field") || err.contains("new_hotspot_maximum"),
            "expected 'unknown field' in error, got: {err}"
        );
    }

    #[test]
    fn parses_full_gate_set() {
        let raw = r"
[gates]
cognitive_max = 30
code_health_min = 60
hotspot_score_max = 8.0
disallow_clone_type_1 = true

[diff]
delta_code_health_min = -5
new_hotspot_max = 0
";
        let t = Thresholds::from_text(raw).unwrap();
        assert_eq!(t.gates.cognitive_max, Some(30.0));
        assert_eq!(t.gates.code_health_min, Some(60.0));
        assert_eq!(t.gates.hotspot_score_max, Some(8.0));
        assert!(t.gates.disallow_clone_type_1);
        assert_eq!(t.diff.delta_code_health_min, Some(-5.0));
        assert_eq!(t.diff.new_hotspot_max, Some(0));
        assert!(!t.is_empty());
    }

    fn make_row(
        path: &str,
        cognitive: f64,
        code_health: f64,
        hotspot: f64,
    ) -> crate::analyses::hotspots::HotspotRow {
        crate::analyses::hotspots::HotspotRow {
            path: path.to_string(),
            revisions: 1,
            cognitive,
            code_health,
            hotspot_score: hotspot,
            mi: None,
            mi_rank: None,
            ai_pct: None,
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

    #[test]
    fn is_empty_accounts_for_delta_health_keys() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        assert!(!t.is_empty());
        let t = Thresholds::from_text("[diff]\ndeny_degrading_verdict = true\n").unwrap();
        assert!(!t.is_empty());
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

    #[test]
    fn effort_exposure_unknown_key_rejected_by_deny_unknown_fields() {
        // Typo guard: `max_red_effort_percentage` must not silently parse.
        let raw = "[gates]\nmax_red_effort_percentage = 30.0\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("max_red_effort"),
            "expected 'unknown field' in error: {err}"
        );
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
}
