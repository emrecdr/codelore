//! Threshold configuration: parse, discover, and validate
//! `.codelore-thresholds.toml`.
//!
//! The `[gates]` / `[diff]` / `[calibration]` sections deserialize into
//! [`Thresholds`]. `deny_unknown_fields` turns a mistyped key into a parse
//! error, and [`Thresholds::validate`] range-checks every numeric bound so a
//! degenerate value (`nan`, `inf`, out-of-domain) surfaces as a configuration
//! error rather than a silently vacuous gate. The evaluators that consume this
//! config against a fact store or analysis rows live in the sibling
//! [`evaluators`](super::evaluators) module.

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
    /// The optional `[new_code]` section — a period-scoped two-band gate.
    /// `None` when the section is absent (byte-identical to no feature);
    /// `Some` enables it. See [`NewCodeGates`].
    #[serde(default)]
    pub new_code: Option<NewCodeGates>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // gate config bag: independent policy toggles, not a state machine
pub struct Gates {
    /// Maximum cognitive complexity per file. Files exceeding fail
    /// the gate.
    pub cognitive_max: Option<f64>,
    /// Minimum code-health score per file. Files below fail.
    pub code_health_min: Option<f64>,
    /// Maximum hotspot score per file.
    pub hotspot_score_max: Option<f64>,
    /// Maximum corpus-anchored hotspot score per file (the `hotspot-score-anchored`
    /// column). Same `[0, 10]` scale as `hotspot_score_max`, but the file's
    /// complexity is ranked against the calibration corpus rather than the repo,
    /// so improving a file *in place* does not shift any other file's anchored
    /// score — the complexity term is corpus-anchored. The revisions term stays
    /// repo-relative by design, so a change to the revision population itself (a
    /// new file clearing `min_revs`, a refactor commit that reorders the churn
    /// ranks) can still move an untouched file's score; this ceiling carries
    /// margin above the measured worst to absorb that, keeping it an absolute
    /// standard rather than a moving target.
    ///
    /// The gate is **skipped** (not failed, not passed) when no calibration
    /// artifact is active — no embedded world corpus and no `--calibration`
    /// override means hotspot rows carry no `hotspot_score_anchored`, so there is
    /// nothing to compare against. The skip is recorded in the ledger with
    /// `verdict = "skipped"` and printed as a distinct warning line; it does not
    /// affect the exit code — mirroring `corpus_percentile_max`.
    pub hotspot_anchored_max: Option<f64>,
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
    /// Companion to [`max_red_effort_pct`](Self::max_red_effort_pct): when
    /// `true`, the ceiling is compared against only the *degrading* share of
    /// red-band window churn — churn that landed in red files whose own health
    /// did NOT improve over the window — exempting churn that refactored a red
    /// file toward health. Default `false` leaves the gate comparing the full
    /// red churn share (behaviour unchanged). Has no effect unless
    /// `max_red_effort_pct` is also set; like `fail_on_degraded` it is a
    /// modifier, not a gate, so it never makes the thresholds non-empty on its
    /// own.
    #[serde(default)]
    pub red_effort_exempt_improving: bool,
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
    /// When `true`, a gate recorded `verdict = "skipped"` — its underlying
    /// analysis had nothing to evaluate (no external-findings sidecar, no
    /// calibration artifact, a history shallower than a window, or a change-set
    /// that measured no delta) — is treated as a failure, so a gate that
    /// silently stopped evaluating is no longer indistinguishable at the exit
    /// code from one that evaluated and passed. Default `false` leaves a skipped
    /// gate exit-0 (behaviour unchanged). Honoured by `codelore check`,
    /// `codelore gate`, and `codelore diff`; like `fail_on_degraded` it is a
    /// policy modifier, not a gate, so it never makes the thresholds non-empty
    /// on its own, and the ledger keeps the honest `"skipped"` verdict either
    /// way.
    #[serde(default)]
    pub fail_on_skipped: bool,
}

fn default_fail_on_degraded() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffGates {
    /// Maximum allowed drop in median code-health between base and
    /// head. A drop of more than this magnitude fails the gate.
    ///
    /// The same key is evaluated against a DIFFERENT metric depending on the
    /// surface: `codelore diff` compares `hotspots::HotspotRow::cognitive_health`
    /// (`[60, 100]`, `diff.rs`'s `median_code_health`); `codelore gate` /
    /// `gate_changes` compares `code_health::CodeHealthRow::score` (`[0, 100]`,
    /// `change_set.rs`'s `HealthProjection::baseline_median`/`projected_median`).
    /// This is a documented, deliberate divergence — see `docs/advanced-usage.md`'s
    /// gate-surface comparison table.
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

/// The `[new_code]` section: a two-band, period-scoped quality gate over the
/// active working set.
///
/// - **Born in window** — a file first seen inside the rolling
///   [`window_days`](Self::window_days) window must meet
///   [`born_health_min`](Self::born_health_min) at HEAD. This generalises the
///   `[diff]` [`new_file_health_min`](DiffGates::new_file_health_min) floor from
///   PR scope to period scope.
/// - **Touched in window** — a file touched (but not born) inside the window
///   owes non-negative net health movement over the window when
///   [`touched_no_degradation`](Self::touched_no_degradation) is on (the
///   default). The signal is the *same* per-file net-movement the
///   improving-churn effort exemption computes, here required to be
///   non-degrading rather than strictly improving.
/// - **Untouched** — legacy files nobody edited in the window are exempt; only
///   the absolute `[gates]` apply to them.
///
/// The section is opt-in: its **presence** (any key, or none) enables the gate
/// and makes the thresholds file non-empty. Absence is byte-identical to the
/// behaviour without this feature. The PR-scoped `new_file_health_min` and this
/// period-scoped floor are complementary and both stay: the former answers "is
/// this pull request acceptable now", the latter "is the working set healthy".
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewCodeGates {
    /// Rolling window, in days, defining the active working set. Anchored to the
    /// repo's most recent commit date (not wall-clock), so the born/touched
    /// partition is reproducible on archived repos. Defaults to
    /// [`DEFAULT_WINDOW_DAYS`](crate::constants::DEFAULT_WINDOW_DAYS) — the same
    /// default the effort-exposure view's window uses. The two are nonetheless
    /// independent knobs that can diverge: this gate reads its own
    /// `[new_code] window_days`, while the effort-exposure view reads
    /// `opts.window_days` (the CLI `--window-days`). Sharing a default does not
    /// make them one working set.
    #[serde(default = "default_new_code_window_days")]
    pub window_days: u32,
    /// Floor a file *born* inside the window must meet at HEAD, on the `[0, 100]`
    /// code-health scale. Absent ⇒ the born band is not evaluated (only the
    /// touched band applies).
    pub born_health_min: Option<f64>,
    /// When `true` (the **default** once the section is present), a file
    /// *touched but not born* inside the window fails if its net health movement
    /// over the window is negative. A window that touched a file without moving
    /// any function across a delta-health risk band (a typo fix, a comment edit)
    /// nets zero and passes. Set `false` to keep the born band while dropping the
    /// touched band.
    #[serde(default = "default_touched_no_degradation")]
    pub touched_no_degradation: bool,
}

fn default_new_code_window_days() -> u32 {
    crate::constants::DEFAULT_WINDOW_DAYS
}

fn default_touched_no_degradation() -> bool {
    true
}

impl Default for NewCodeGates {
    /// Matches the shape serde produces for an empty `[new_code]` table:
    /// the default window, no born floor, touched-band on.
    fn default() -> Self {
        Self {
            window_days: default_new_code_window_days(),
            born_health_min: None,
            touched_no_degradation: default_touched_no_degradation(),
        }
    }
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

/// Push a problem when `value` is present and either non-finite or outside the
/// inclusive `[lo, hi]` domain. Shared by every bounded [`Thresholds::validate`]
/// check so the wording of the two failure modes stays identical across keys.
fn finite_in(problems: &mut Vec<String>, key: &str, value: Option<f64>, lo: f64, hi: f64) {
    let Some(v) = value else { return };
    if !v.is_finite() {
        problems.push(format!("{key} = {v} must be a finite number"));
    } else if !(lo..=hi).contains(&v) {
        problems.push(format!(
            "{key} = {v} is outside the accepted range [{lo}, {hi}]"
        ));
    }
}

/// Push a problem when `value` is present and either non-finite or below `min`,
/// with no upper bound — the open-topped ceilings (`cognitive_max`,
/// `hotspot_score_max`, `hotspot_anchored_max`) whose worst value a huge file
/// can legitimately reach.
fn finite_min(problems: &mut Vec<String>, key: &str, value: Option<f64>, min: f64) {
    let Some(v) = value else { return };
    if !v.is_finite() {
        problems.push(format!("{key} = {v} must be a finite number"));
    } else if v < min {
        problems.push(format!("{key} = {v} must be >= {min}"));
    }
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

    /// Parse a thresholds file from disk and validate its values.
    ///
    /// # Errors
    ///
    /// - [`CodeLoreError::RepoIo`] (exit 3) when the file cannot be read.
    /// - [`CodeLoreError::Analysis`] (exit 4) when the TOML fails to parse.
    /// - [`CodeLoreError::InvalidOptions`] (exit 2, the configuration-error
    ///   bucket) when a threshold value is non-finite or out of range — the
    ///   value-level sibling of the `deny_unknown_fields` typo guard.
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
        let thresholds = Self::from_text(&raw).map_err(|e| {
            CodeLoreError::Analysis(format!("parse thresholds {}: {e}", path.display()))
        })?;
        thresholds.validate().map_err(|e| {
            CodeLoreError::InvalidOptions(format!("thresholds {}: {e}", path.display()))
        })?;
        Ok(thresholds)
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
            && self.gates.hotspot_anchored_max.is_none()
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
            // The `[new_code]` section is opt-in by presence: any `[new_code]`
            // table (even with no keys) enables the period gate and makes the
            // thresholds file non-empty, so `check` does not short-circuit.
            && self.new_code.is_none()
        // Note: fail_on_degraded=true is the default and does not make a
        // threshold non-empty by itself — it only affects how degraded
        // verdicts from other gates are handled. red_effort_exempt_improving is
        // excluded for the same reason: it only reshapes the max_red_effort_pct
        // comparison, so a file setting it alone configures no gate.
        // fail_on_skipped is excluded too: it only changes how a skipped
        // verdict maps to the exit code, configuring no gate on its own.
        // Likewise `[calibration]`
        // is deliberately excluded from this expression: it selects a
        // defect-calibration artifact for analyses to consume, it does not
        // configure a gate, so a thresholds file containing only
        // `[calibration]` still leaves `check` vacuously passing.
    }

    /// Validate every numeric threshold's value. A bare `toml::from_str`
    /// accepts TOML's `nan`/`inf` float literals and any out-of-range
    /// magnitude, and every gate comparison is a bare `>`/`<`: a `nan`
    /// floor can never fire, an `inf` ceiling vacuous-passes, and a negative
    /// or overshot bound is silently degenerate. This is the value-level
    /// sibling of the struct's `deny_unknown_fields` typo guard.
    ///
    /// Each field's domain is derived from its own documented semantics:
    /// scores and percentages on `[0, 100]`, ratios on `[0, 1]`, health
    /// *deltas* on `[-100, 100]` (a `[0, 100]` score can move at most a full
    /// span either way), and the three open-topped ceilings (`cognitive_max`,
    /// `hotspot_score_max`, `hotspot_anchored_max`) require only finiteness and
    /// non-negativity.
    /// `u32` counts (`max_dependency_cycles`, `new_hotspot_max`,
    /// `max_findings_in_hot_files`) cannot express a non-finite or negative
    /// value, so the type already guards them.
    ///
    /// Every offending key is collected in one pass so a degenerate config
    /// surfaces all its problems at once rather than one re-run at a time.
    ///
    /// # Errors
    ///
    /// Returns a `String` naming each offending key, its value, and the
    /// accepted domain. `Ok(())` when every configured threshold is in range.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let mut problems = Vec::new();
        let g = &self.gates;
        finite_min(&mut problems, "cognitive_max", g.cognitive_max, 0.0);
        finite_in(
            &mut problems,
            "code_health_min",
            g.code_health_min,
            0.0,
            100.0,
        );
        finite_min(&mut problems, "hotspot_score_max", g.hotspot_score_max, 0.0);
        finite_min(
            &mut problems,
            "hotspot_anchored_max",
            g.hotspot_anchored_max,
            0.0,
        );
        finite_in(
            &mut problems,
            "max_propagation_cost",
            g.max_propagation_cost,
            0.0,
            1.0,
        );
        finite_in(
            &mut problems,
            "max_red_effort_pct",
            g.max_red_effort_pct,
            0.0,
            100.0,
        );
        finite_in(
            &mut problems,
            "code_familiarity_min",
            g.code_familiarity_min,
            0.0,
            100.0,
        );
        finite_in(
            &mut problems,
            "corpus_percentile_max",
            g.corpus_percentile_max,
            0.0,
            1.0,
        );

        let d = &self.diff;
        finite_in(
            &mut problems,
            "delta_code_health_min",
            d.delta_code_health_min,
            -100.0,
            100.0,
        );
        finite_in(
            &mut problems,
            "delta_health_min",
            d.delta_health_min,
            0.0,
            100.0,
        );
        finite_in(
            &mut problems,
            "delta_code_health_min_per_file",
            d.delta_code_health_min_per_file,
            -100.0,
            100.0,
        );
        finite_in(
            &mut problems,
            "new_file_health_min",
            d.new_file_health_min,
            0.0,
            100.0,
        );

        // `[new_code]` bounds: the window must be a sane trailing period and the
        // born floor a valid `[0, 100]` score. `window_days` is a `u32`, so the
        // type already guards non-finite/negative — only the range needs a check.
        if let Some(nc) = &self.new_code {
            if !(7..=365).contains(&nc.window_days) {
                problems.push(format!(
                    "window_days = {} is outside the accepted range [7, 365]",
                    nc.window_days
                ));
            }
            finite_in(
                &mut problems,
                "born_health_min",
                nc.born_health_min,
                0.0,
                100.0,
            );
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
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

#[cfg(all(test, feature = "test-support"))]
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
    fn is_empty_accounts_for_delta_health_keys() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        assert!(!t.is_empty());
        let t = Thresholds::from_text("[diff]\ndeny_degrading_verdict = true\n").unwrap();
        assert!(!t.is_empty());
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

    // ───────── red_effort_exempt_improving modifier ─────────

    #[test]
    fn red_effort_exempt_improving_parses_and_defaults_false() {
        let on = Thresholds::from_text("[gates]\nred_effort_exempt_improving = true\n").unwrap();
        assert!(on.gates.red_effort_exempt_improving);
        // Omitted ⇒ default false (behaviour unchanged).
        let off = Thresholds::from_text("[gates]\nmax_red_effort_pct = 15.0\n").unwrap();
        assert!(!off.gates.red_effort_exempt_improving);
    }

    #[test]
    fn red_effort_exempt_improving_alone_does_not_configure_a_gate() {
        // A modifier, not a gate: like fail_on_degraded it must leave the
        // thresholds vacuously empty when it is the only key present, and it
        // must still validate.
        let t = Thresholds::from_text("[gates]\nred_effort_exempt_improving = true\n").unwrap();
        assert!(t.is_empty(), "the modifier alone configures no gate");
        t.validate().expect("bool modifier is always valid");
    }

    #[test]
    fn red_effort_exempt_improving_unknown_key_rejected() {
        // Typo guard: a near-miss must reject, not silently disable the exemption.
        let raw = "[gates]\nred_effort_exempt_improveing = true\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("red_effort_exempt_improveing"),
            "expected 'unknown field' in error: {err}"
        );
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

    // ───────── fail_on_skipped modifier ─────────

    #[test]
    fn fail_on_skipped_parses_and_defaults_false() {
        let on = Thresholds::from_text("[gates]\nfail_on_skipped = true\n").unwrap();
        assert!(on.gates.fail_on_skipped);
        // Omitted ⇒ default false (behaviour unchanged).
        let off = Thresholds::from_text("[gates]\ncode_health_min = 50.0\n").unwrap();
        assert!(!off.gates.fail_on_skipped);
    }

    #[test]
    fn fail_on_skipped_alone_does_not_configure_a_gate() {
        // A modifier, not a gate: like fail_on_degraded it must leave the
        // thresholds vacuously empty when it is the only key present, and it
        // must still validate.
        let t = Thresholds::from_text("[gates]\nfail_on_skipped = true\n").unwrap();
        assert!(t.is_empty(), "the modifier alone configures no gate");
        t.validate().expect("bool modifier is always valid");
    }

    #[test]
    fn fail_on_skipped_unknown_key_rejected() {
        // Typo guard: a near-miss must reject, not silently disable the policy.
        let raw = "[gates]\nfail_on_skiped = true\n";
        let err = Thresholds::from_text(raw).expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("fail_on_skiped"),
            "expected 'unknown field' in error: {err}"
        );
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

    // ───────── value validation ─────────

    #[test]
    fn validate_accepts_default_and_empty_config() {
        // The default (all-None) and an empty file configure no gates, so
        // there is nothing to validate — both must pass.
        Thresholds::default().validate().expect("default is valid");
        Thresholds::from_text("")
            .unwrap()
            .validate()
            .expect("empty config is valid");
    }

    #[test]
    fn validate_accepts_repo_own_threshold_shape() {
        // Mirror this repository's committed `.codelore-thresholds.toml`; a
        // valid config must parse AND validate exactly as before this guard.
        let raw = "\
[gates]
code_health_min = 35.0
cognitive_max = 150.0
hotspot_score_max = 4.0
max_dependency_cycles = 1
max_propagation_cost = 0.10
max_red_effort_pct = 15.0

[diff]
no_new_cycles = true
";
        Thresholds::from_text(raw)
            .expect("parse")
            .validate()
            .expect("the repo's own thresholds must validate");
    }

    #[test]
    fn validate_accepts_negative_delta_floor() {
        // A negative delta floor is legitimate and documented (`health may
        // drop at most 5 pts`): -5 is in [-100, 100], so it must pass.
        Thresholds::from_text("[diff]\ndelta_code_health_min = -5\n")
            .unwrap()
            .validate()
            .expect("negative delta floor is valid");
    }

    #[test]
    fn validate_rejects_nan() {
        // `nan` parses as a float literal, then can never fire a `<` gate.
        let err = Thresholds::from_text("[gates]\ncode_health_min = nan\n")
            .unwrap()
            .validate()
            .expect_err("nan must be rejected");
        assert!(
            err.contains("code_health_min") && err.contains("finite"),
            "message names the key and the finiteness requirement: {err}"
        );
    }

    #[test]
    fn validate_rejects_inf() {
        // `inf` ceiling vacuous-passes every file — reject it.
        let err = Thresholds::from_text("[gates]\nhotspot_score_max = inf\n")
            .unwrap()
            .validate()
            .expect_err("inf must be rejected");
        assert!(
            err.contains("hotspot_score_max") && err.contains("finite"),
            "message names the key and the finiteness requirement: {err}"
        );
    }

    #[test]
    fn validate_rejects_negative_hotspot_anchored_max() {
        // Open-topped ceiling like hotspot_score_max: finite and non-negative.
        let err = Thresholds::from_text("[gates]\nhotspot_anchored_max = -1.0\n")
            .unwrap()
            .validate()
            .expect_err("negative anchored ceiling must be rejected");
        assert!(
            err.contains("hotspot_anchored_max") && err.contains(">= 0"),
            "message names the key and the non-negativity requirement: {err}"
        );
    }

    #[test]
    fn is_empty_false_when_only_hotspot_anchored_max_set() {
        // A lone anchored ceiling is a real gate — `check` must not short-circuit.
        let t = Thresholds::from_text("[gates]\nhotspot_anchored_max = 9.9\n").unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    fn validate_rejects_negative_floor() {
        // A negative score floor fails every file (all scores >= 0) —
        // degenerate. `code_health_min` is a [0, 100] domain, so -5 is out.
        let err = Thresholds::from_text("[gates]\ncode_health_min = -5.0\n")
            .unwrap()
            .validate()
            .expect_err("negative score floor must be rejected");
        assert!(
            err.contains("code_health_min") && err.contains("[0, 100]"),
            "message names the key and its domain: {err}"
        );
    }

    #[test]
    fn validate_rejects_out_of_range_ratio() {
        // A propagation-cost ceiling above 1.0 is meaningless (the metric is
        // a [0, 1] density) — reject with the domain named.
        let err = Thresholds::from_text("[gates]\nmax_propagation_cost = 2.0\n")
            .unwrap()
            .validate()
            .expect_err("out-of-range ratio must be rejected");
        assert!(
            err.contains("max_propagation_cost") && err.contains("[0, 1]"),
            "message names the key and its [0, 1] domain: {err}"
        );
    }

    #[test]
    fn validate_reports_all_offenders_at_once() {
        // A degenerate config with several bad values surfaces EVERY problem
        // in one error, not one per re-run.
        let raw = "\
[gates]
code_health_min = nan
max_propagation_cost = 5.0
hotspot_score_max = -1.0

[diff]
new_file_health_min = 200.0
";
        let err = Thresholds::from_text(raw)
            .unwrap()
            .validate()
            .expect_err("multiple offenders must be rejected");
        for key in [
            "code_health_min",
            "max_propagation_cost",
            "hotspot_score_max",
            "new_file_health_min",
        ] {
            assert!(
                err.contains(key),
                "every offender named ({key} missing): {err}"
            );
        }
    }

    #[test]
    fn from_path_degenerate_value_is_config_error_exit_2() {
        // End-to-end: a degenerate value on the disk-load path surfaces as a
        // configuration error (exit-2 taxonomy), naming the offending key —
        // the value-level counterpart of the deny_unknown_fields typo guard,
        // which malformed TOML (Analysis, exit 4) does not cover.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(THRESHOLDS_FILENAME);
        std::fs::write(&path, "[gates]\nhotspot_score_max = inf\n").expect("write");
        let err = Thresholds::from_path(&path).expect_err("degenerate value must error");
        assert_eq!(err.exit_code(), 2, "config-error taxonomy (exit 2): {err}");
        assert!(
            err.to_string().contains("hotspot_score_max"),
            "error names the offending key: {err}"
        );
    }

    #[test]
    fn from_path_valid_config_is_byte_identical_ok() {
        // A valid file loads exactly as before the value guard existed.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(THRESHOLDS_FILENAME);
        std::fs::write(&path, "[gates]\ncode_health_min = 60.0\n").expect("write");
        let t = Thresholds::from_path(&path).expect("valid config must load");
        assert_eq!(t.gates.code_health_min, Some(60.0));
    }

    // ───────── [new_code] section ─────────

    #[test]
    fn new_code_absent_is_none_and_empty() {
        // No `[new_code]` table ⇒ the field is None and the file stays empty
        // (byte-identical to before the feature).
        let t = Thresholds::from_text("[gates]\ncode_health_min = 60.0\n").unwrap();
        assert!(t.new_code.is_none());
        let bare = Thresholds::from_text("").unwrap();
        assert!(bare.new_code.is_none());
        assert!(bare.is_empty());
    }

    #[test]
    fn new_code_empty_section_takes_defaults_and_is_non_empty() {
        // Presence of the section (even with no keys) enables the gate and makes
        // the thresholds file non-empty; the defaults are window 90, no born
        // floor, touched band on.
        let t = Thresholds::from_text("[new_code]\n").unwrap();
        let nc = t.new_code.as_ref().expect("section present");
        assert_eq!(nc.window_days, 90);
        assert_eq!(nc.born_health_min, None);
        assert!(nc.touched_no_degradation);
        assert!(
            !t.is_empty(),
            "presence of [new_code] makes the file non-empty"
        );
        t.validate().expect("defaults are in range");
    }

    #[test]
    fn new_code_parses_explicit_values() {
        let t = Thresholds::from_text(
            "[new_code]\nwindow_days = 30\nborn_health_min = 60.0\ntouched_no_degradation = false\n",
        )
        .unwrap();
        let nc = t.new_code.as_ref().expect("section present");
        assert_eq!(nc.window_days, 30);
        assert_eq!(nc.born_health_min, Some(60.0));
        assert!(!nc.touched_no_degradation);
        assert!(!t.is_empty());
    }

    #[test]
    fn new_code_unknown_key_rejected() {
        // deny_unknown_fields must catch a near-miss spelling, mirroring siblings.
        let err = Thresholds::from_text("[new_code]\nwindow_dayz = 90\n")
            .expect_err("typo'd key should reject");
        assert!(
            err.contains("unknown field") || err.contains("window_dayz"),
            "expected 'unknown field' in error: {err}"
        );
    }

    #[test]
    fn new_code_validate_rejects_out_of_range_window() {
        // Window below the floor and above the ceiling both fail with the domain.
        for bad in [
            "[new_code]\nwindow_days = 6\n",
            "[new_code]\nwindow_days = 366\n",
        ] {
            let err = Thresholds::from_text(bad)
                .unwrap()
                .validate()
                .expect_err("out-of-range window must be rejected");
            assert!(
                err.contains("window_days") && err.contains("[7, 365]"),
                "message names the key and its domain: {err}"
            );
        }
        // The inclusive bounds themselves pass.
        Thresholds::from_text("[new_code]\nwindow_days = 7\n")
            .unwrap()
            .validate()
            .expect("7 is in range");
        Thresholds::from_text("[new_code]\nwindow_days = 365\n")
            .unwrap()
            .validate()
            .expect("365 is in range");
    }

    #[test]
    fn new_code_validate_rejects_out_of_range_born_floor() {
        let err = Thresholds::from_text("[new_code]\nborn_health_min = 150.0\n")
            .unwrap()
            .validate()
            .expect_err("born floor above 100 must be rejected");
        assert!(
            err.contains("born_health_min") && err.contains("[0, 100]"),
            "message names the key and its domain: {err}"
        );
        let err = Thresholds::from_text("[new_code]\nborn_health_min = nan\n")
            .unwrap()
            .validate()
            .expect_err("nan born floor must be rejected");
        assert!(
            err.contains("born_health_min") && err.contains("finite"),
            "message names the key and the finiteness requirement: {err}"
        );
    }
}
