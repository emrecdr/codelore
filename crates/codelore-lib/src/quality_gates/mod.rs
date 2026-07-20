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
//! delta_code_health_min_per_file = 0.0  # gate-only: no changed file may lower its own health
//! new_file_health_min = 50.0  # gate-only: added files must clear this floor
//!
//! [calibration]
//! defect_artifact = "defects.calib.json"  # repo-declared --defect-calibration default
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

pub mod evidence;
pub mod ledger;
pub mod ratchet;

use std::fs;
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub calibration: CalibrationConfig,
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
    /// Maximum number of `"act-now"` rows in the `finding-hotspot-overlap`
    /// analysis. Fails the gate when the count exceeds the threshold.
    ///
    /// The gate is **skipped** (not failed, not passed) when the external
    /// findings sidecar is absent or empty — a missing sidecar means
    /// `codelore ingest-sarif` has not been run, not that there are zero
    /// findings. The skip is recorded in the ledger with
    /// `verdict = "skipped"` and printed as a distinct warning line; it
    /// does not affect the exit code.
    pub max_findings_in_hot_files: Option<u32>,
    /// Maximum corpus-relative percentile (`[0,1]`) any file may reach. A
    /// file whose worst raw metric sits above this fraction of a reference
    /// corpus fails the gate — a ceiling on "how extreme is this file versus
    /// the world (or your org)".
    ///
    /// The gate is **skipped** (not failed, not passed) when no calibration
    /// artifact is active — no embedded world corpus and no `--calibration`
    /// override means code-health rows carry no `corpus_percentile`, so there
    /// is nothing to compare against. The skip is recorded in the ledger with
    /// `verdict = "skipped"` and printed as a distinct warning line; it does
    /// not affect the exit code.
    pub corpus_percentile_max: Option<f64>,
    /// When `true` (the **default**), a gate whose underlying analysis
    /// produced no evaluable data where data was expected is recorded as
    /// `verdict = "degraded"` and treated as a failure — a gate must not
    /// green on blindness. Set to `false` to downgrade degraded gates from
    /// failure to a warning (the degraded verdict is still printed and
    /// recorded in the ledger).
    ///
    /// Adapted from the explicit-degradation contract in SAST tooling:
    /// an analyzer that silently returns "no findings" on an empty scan
    /// is indistinguishable from one that found nothing. Degraded status
    /// makes the difference explicit.
    #[serde(default = "default_fail_on_degraded")]
    pub fail_on_degraded: bool,
}

fn default_fail_on_degraded() -> bool {
    true
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
    /// Per-file floor on the working-tree projected code-health delta,
    /// evaluated only by `codelore gate` / the `gate_changes` MCP tool: every
    /// changed file whose `projected − baseline` score delta falls below this
    /// floor fails the gate (one violation per file). `codelore diff` does
    /// not evaluate this key — its `delta_code_health_min` sibling stays the
    /// whole-repo-median gate on both surfaces.
    pub delta_code_health_min_per_file: Option<f64>,
    /// Minimum projected code-health score an ADDED file must meet, evaluated
    /// only by `codelore gate` / the `gate_changes` MCP tool. An added file
    /// has no baseline — its `delta_code_health_min_per_file` delta is always
    /// `None`, so it evades that floor — and its small footprint rarely moves
    /// the whole-repo median enough to trip `delta_code_health_min` either.
    /// Without this floor a freshly added low-health file (a new god-class,
    /// say) can clear every other gate. One violation per offending added
    /// file, with the file's projected score as the measured value.
    /// `codelore diff` does not evaluate this key. Deleted files (no
    /// projected score) never trigger it.
    pub new_file_health_min: Option<f64>,
}

/// The `[calibration]` section: repo-declared analysis calibration, applied
/// wherever the equivalent CLI flag is accepted. This is a config *selector*,
/// not a gate — its presence never enables gate evaluation on its own.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationConfig {
    /// Path to a `defects.calib.json` defect-calibration artifact, relative
    /// to the repo root (absolute paths are used as-is). Overridden by the
    /// `--defect-calibration` CLI flag and the MCP server's startup flag;
    /// absent everywhere means uncalibrated.
    pub defect_artifact: Option<PathBuf>,
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
            && self.gates.max_findings_in_hot_files.is_none()
            && self.gates.corpus_percentile_max.is_none()
            && self.diff.delta_code_health_min.is_none()
            && self.diff.new_hotspot_max.is_none()
            && !self.diff.no_new_cycles
            && self.diff.delta_health_min.is_none()
            && !self.diff.deny_degrading_verdict
            && self.diff.delta_code_health_min_per_file.is_none()
            && self.diff.new_file_health_min.is_none()
        // Note: fail_on_degraded=true is the default and does not make a
        // threshold non-empty by itself — it only affects how degraded
        // verdicts from other gates are handled. Likewise `[calibration]`
        // is deliberately excluded from this expression: it selects a
        // defect-calibration artifact for analyses to consume, it does not
        // configure a gate, so a thresholds file containing only
        // `[calibration]` still leaves `check` vacuously passing.
    }
}

/// Resolve the effective defect-calibration artifact path for a repo:
/// an explicit flag wins; otherwise the discovered thresholds file's
/// `[calibration] defect_artifact` (relative paths joined to the repo
/// root); otherwise `None` (uncalibrated).
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on I/O or parse errors discovering the
/// thresholds file.
pub fn resolve_defect_calibration(
    cli_flag: Option<PathBuf>,
    repo_root: &Path,
) -> Result<Option<PathBuf>> {
    if cli_flag.is_some() {
        return Ok(cli_flag);
    }
    let thresholds = Thresholds::discover(repo_root)?;
    Ok(thresholds.calibration.defect_artifact.map(|p| {
        if p.is_absolute() {
            p
        } else {
            repo_root.join(p)
        }
    }))
}

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
            corpus_percentile: None,
            beyond_corpus: false,
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

    #[test]
    fn parses_corpus_percentile_max_gate() {
        let raw = "[gates]\ncorpus_percentile_max = 0.9\n";
        let t = Thresholds::from_text(raw).unwrap();
        assert_eq!(t.gates.corpus_percentile_max, Some(0.9));
        assert!(!t.is_empty());
    }

    #[test]
    fn unknown_corpus_gate_key_is_rejected() {
        // A near-miss of the new key must reject, not silently disable the gate.
        let raw = "[gates]\ncorpus_percentile_maximum = 0.9\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd corpus gate key should reject");
        assert!(
            err.contains("unknown field") || err.contains("corpus_percentile_maximum"),
            "expected 'unknown field' in error, got: {err}"
        );
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

    #[test]
    fn per_file_floor_key_parses_and_makes_thresholds_non_empty() {
        let t = Thresholds::from_text("[diff]\ndelta_code_health_min_per_file = 0.0\n").unwrap();
        assert_eq!(t.diff.delta_code_health_min_per_file, Some(0.0));
        assert!(!t.is_empty());
    }

    #[test]
    fn per_file_floor_unknown_key_rejected() {
        // Typo guard: deny_unknown_fields must catch a near-miss spelling.
        let raw = "[diff]\ndelta_code_health_min_per_flie = 0.0\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("delta_code_health_min_per_flie"),
            "expected 'unknown field' in error: {err}"
        );
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
                make_gate_delta("src/gone.rs", None),        // deleted (no projected) → ignored
            ],
            Some(60.0),
            Some(59.0),
            Vec::new(),
        );
        let v = evaluate_gate_thresholds(&t, &report);
        assert_eq!(v.len(), 1, "only the below-floor added file violates: {v:?}");
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

    #[test]
    fn new_file_floor_key_parses_and_makes_thresholds_non_empty() {
        let t = Thresholds::from_text("[diff]\nnew_file_health_min = 50.0\n").unwrap();
        assert_eq!(t.diff.new_file_health_min, Some(50.0));
        assert!(!t.is_empty());
    }

    #[test]
    fn new_file_floor_unknown_key_rejected() {
        // Typo guard: deny_unknown_fields must catch a near-miss spelling.
        let raw = "[diff]\nnew_file_health_minimum = 50.0\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("new_file_health_minimum"),
            "expected 'unknown field' in error: {err}"
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
    fn finding_overlap_gate_toml_key_parses() {
        let t = Thresholds::from_text("[gates]\nmax_findings_in_hot_files = 3\n").unwrap();
        assert_eq!(t.gates.max_findings_in_hot_files, Some(3));
        assert!(!t.is_empty());
    }

    #[test]
    fn finding_overlap_gate_unknown_key_rejected() {
        // Typo guard: deny_unknown_fields must catch bad spellings.
        let raw = "[gates]\nmax_findings_in_hot_file = 3\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("max_findings_in_hot_file"),
            "expected 'unknown field' in error: {err}"
        );
    }

    // ───────── fail_on_degraded TOML parsing ─────────

    #[test]
    fn fail_on_degraded_false_parses() {
        let t = Thresholds::from_text("[gates]\nfail_on_degraded = false\n").unwrap();
        assert!(!t.gates.fail_on_degraded);
    }

    #[test]
    fn fail_on_degraded_defaults_to_true_when_omitted() {
        let t = Thresholds::from_text("[gates]\ncode_health_min = 50.0\n").unwrap();
        assert!(t.gates.fail_on_degraded);
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

    // ───────── [calibration] section ─────────

    #[test]
    fn calibration_section_parses_and_does_not_make_thresholds_non_empty() {
        let t = Thresholds::from_text(
            "[calibration]\ndefect_artifact = \"artifacts/defects.calib.json\"\n",
        )
        .expect("parse");
        assert_eq!(
            t.calibration.defect_artifact.as_deref(),
            Some(std::path::Path::new("artifacts/defects.calib.json"))
        );
        // A calibration-only file configures no gates: `check` must keep
        // vacuously passing, exactly like a fail_on_degraded-only file.
        assert!(t.is_empty(), "calibration alone must not enable gates");
    }

    #[test]
    fn calibration_section_rejects_unknown_keys() {
        let err = Thresholds::from_text("[calibration]\ndefect_artefact = \"x\"\n");
        assert!(err.is_err(), "deny_unknown_fields must reject the typo");
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn resolve_defect_calibration_prefers_cli_flag_over_section() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(THRESHOLDS_FILENAME),
            "[calibration]\ndefect_artifact = \"from-section.json\"\n",
        )
        .expect("write thresholds");
        let cli = Some(PathBuf::from("/explicit/flag.json"));
        let resolved = resolve_defect_calibration(cli.clone(), dir.path()).expect("resolve");
        assert_eq!(resolved, cli, "CLI flag wins");
        let fallback = resolve_defect_calibration(None, dir.path()).expect("resolve");
        assert_eq!(
            fallback,
            Some(dir.path().join("from-section.json")),
            "section fills None, relative path joined to repo root"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn resolve_defect_calibration_without_section_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            resolve_defect_calibration(None, dir.path()).expect("resolve"),
            None
        );
    }
}
