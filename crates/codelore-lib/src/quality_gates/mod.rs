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

pub mod config;
pub mod evaluators;
pub mod evidence;
pub mod ledger;
pub mod ratchet;

pub use config::{
    CalibrationConfig, DiffGates, Gates, NewCodeGates, THRESHOLDS_FILENAME, Thresholds,
    resolve_defect_calibration,
};
pub use evaluators::{
    ArchMeasured, GateViolation, change_set_gate_verdict, diff_gate_verdict,
    evaluate_architecture_gate, evaluate_architecture_gate_measured, evaluate_clone_gate,
    evaluate_code_health_gate, evaluate_corpus_percentile_rows, evaluate_diff_gate,
    evaluate_effort_exposure_gate, evaluate_effort_exposure_rows,
    evaluate_effort_exposure_rows_exempt, evaluate_familiarity_gate, evaluate_familiarity_rows,
    evaluate_finding_overlap_rows, evaluate_full_tree, evaluate_gate_thresholds,
    evaluate_hotspot_anchored_rows, evaluate_new_code_rows, head_has_scorable_source,
};
