//! Quality-gates: parse `.codelore-thresholds.toml` at the repo root
//! and evaluate a fact store against the declared gates. Used by
//! `codelore check` to power CI quality-gate enforcement.
//!
//! ## Config schema
//!
//! ```toml
//! [gates]
//! cognitive_max = 30        # any file exceeding fails
//! code_health_min = 60      # any file below fails
//! hotspot_score_max = 8.0   # any file above fails
//! disallow_clone_type_1 = true
//!
//! [diff]
//! delta_code_health_min = -5  # health may drop at most 5 pts in a PR
//! new_hotspot_max = 0         # zero new hotspots allowed
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Thresholds {
    #[serde(default)]
    pub gates: Gates,
    #[serde(default)]
    pub diff: DiffGates,
}

#[derive(Debug, Clone, Default, Deserialize)]
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
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiffGates {
    /// Maximum allowed drop in median code-health between base and
    /// head. A drop of more than this magnitude fails the gate.
    pub delta_code_health_min: Option<f64>,
    /// Maximum number of NEW hotspots a PR may introduce.
    pub new_hotspot_max: Option<u32>,
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
            CodeLoreError::Analysis(format!("read thresholds {}: {e}", path.display()))
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
            && self.diff.delta_code_health_min.is_none()
            && self.diff.new_hotspot_max.is_none()
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
        if let Some(min) = g.code_health_min
            && row.code_health < min
        {
            out.push(GateViolation {
                gate: "code_health_min".into(),
                path: row.path.clone(),
                actual: format!("{:.1}", row.code_health),
                threshold: format!("{min:.1}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_default() {
        let t = Thresholds::from_text("").unwrap();
        assert!(t.is_empty());
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

    #[test]
    fn code_health_min_flags_offending_file() {
        let mut t = Thresholds::default();
        t.gates.code_health_min = Some(70.0);
        let rows = vec![make_row("a.rs", 10.0, 50.0, 1.0)];
        let v = evaluate_full_tree(&t, &rows);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "code_health_min");
    }

    #[test]
    fn empty_thresholds_never_violates() {
        let t = Thresholds::default();
        let rows = vec![make_row("a.rs", 9999.0, 0.0, 99.0)];
        let v = evaluate_full_tree(&t, &rows);
        assert!(v.is_empty());
    }
}
