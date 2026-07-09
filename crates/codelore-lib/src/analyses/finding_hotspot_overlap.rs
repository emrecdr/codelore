//! `finding-hotspot-overlap` — external scanner findings fused with behavioral
//! hotspot and code-health signal.
//!
//! For each file path that appears in the external findings sidecar, this
//! analysis emits one row carrying:
//!
//! - `findings` — total finding count across all engines
//! - `engines` — comma-joined, sorted list of engine names
//! - `worst_level` — most severe level (`"error"` > `"warning"` > `"note"`)
//! - `hotspot_score` — from [`hotspots::run_hotspots`]; 0.0 when absent
//! - `revs_percentile` — `PERCENT_RANK` of this file's revision count within the
//!   hotspot result set; 0.0 when absent from hotspots
//! - `health_band` — `"red"` / `"yellow"` / `"green"` from
//!   [`code_health::run_code_health`]; `"unknown"` when absent
//! - `priority` — classifier produced by [`priority_label`]
//!
//! ## Join strategy
//!
//! The external sidecar and the main `FactsDb` each own a separate `!Send +
//! !Sync` `DuckDB` connection; they MUST NOT be `ATTACH`ed together. All joining
//! is done in Rust via `HashMap` lookups after materialising each side
//! independently.
//!
//! ## Absent paths
//!
//! A path that has findings but is not in the hotspot result set (new file,
//! below `min_revs`, unsupported language) still appears in the output with
//! `hotspot_score = 0.0`, `revs_percentile = 0.0`. This is the honest
//! LEFT-join contract documented to callers.

use std::collections::HashMap;

use crate::analyses::code_health::run_code_health;
use crate::analyses::hotspots::run_hotspots;
use crate::external::{ExternalStore, PathFindings};
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// One row of `finding-hotspot-overlap` output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FindingHotspotOverlapRow {
    /// File path (repo-relative).
    pub path: String,
    /// Total findings across all engines.
    pub findings: usize,
    /// Comma-joined sorted engine names.
    pub engines: String,
    /// Most severe level: `"error"` / `"warning"` / `"note"`.
    pub worst_level: String,
    /// Hotspot score from hotspots analysis; 0.0 when path not in hotspots.
    pub hotspot_score: f64,
    /// `PERCENT_RANK` of revision count within the hotspot result set; 0.0 when
    /// path is absent from hotspots.
    pub revs_percentile: f64,
    /// Code-health band: `"red"` / `"yellow"` / `"green"` / `"unknown"`.
    pub health_band: String,
    /// Priority label: `"act-now"` / `"plan"` / `"note"`.
    pub priority: String,
}

/// Classify priority given the three fused signals.
///
/// Rules (evaluated in order; first match wins):
/// - `"act-now"` — `findings > 0` AND `revs_percentile >= 0.9` AND
///   `health_band == "red"`
/// - `"plan"` — `revs_percentile >= 0.7` OR `health_band == "red"`
/// - `"note"` — everything else
#[must_use]
pub fn priority_label(findings: usize, revs_percentile: f64, health_band: &str) -> &'static str {
    if findings > 0 && revs_percentile >= 0.9 && health_band == "red" {
        "act-now"
    } else if revs_percentile >= 0.7 || health_band == "red" {
        "plan"
    } else {
        "note"
    }
}

/// Run the `finding-hotspot-overlap` analysis.
///
/// Reads all findings from `store`, runs [`run_hotspots`] and
/// [`run_code_health`], then joins in Rust. Emits one row per path that has
/// at least one finding, sorted by priority (act-now first), then findings
/// descending, then path ascending.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] with a user-facing message when `store`
/// contains no findings (required pre-condition: the user must run
/// `codelore ingest-sarif` first).
pub fn run_finding_hotspot_overlap(
    db: &FactsDb,
    opts: &Options,
    store: &ExternalStore,
) -> Result<Vec<FindingHotspotOverlapRow>> {
    // --- read from store ---
    let by_path: HashMap<String, PathFindings> = store.findings_by_path().map_err(|e| {
        CodeLoreError::Analysis(format!("finding-hotspot-overlap: read store: {e}"))
    })?;

    if by_path.is_empty() {
        return Err(CodeLoreError::Analysis(
            "finding-hotspot-overlap requires prior `codelore ingest-sarif` \
             (no external findings found)"
                .to_string(),
        ));
    }

    // --- hotspots side ---
    let hotspot_rows = run_hotspots(db, opts)?;

    // Build a map path → (hotspot_score, revs) for O(1) lookup.
    // revs_percentile is PERCENT_RANK of revision count within this result
    // set: 0.0 for fewest revisions, 1.0 for the most. Computed over the
    // hotspot rows (already filtered by min_revs) — exact equivalent of
    // the SQL `PERCENT_RANK() OVER (ORDER BY revs)` used internally.
    let n = hotspot_rows.len();
    let mut sorted_by_revs: Vec<(usize, u32)> = hotspot_rows
        .iter()
        .enumerate()
        .map(|(i, r)| (i, r.revisions))
        .collect();
    sorted_by_revs.sort_by_key(|&(_, r)| r);

    // rank[i] = PERCENT_RANK position for hotspot_rows[i] within the sorted order.
    let mut rank_by_idx = vec![0.0f64; n];
    if n > 1 {
        #[allow(clippy::cast_precision_loss)]
        // PERCENT_RANK: precision loss negligible for repo-scale revision counts
        for (rank_pos, &(orig_idx, _)) in sorted_by_revs.iter().enumerate() {
            rank_by_idx[orig_idx] = rank_pos as f64 / (n - 1) as f64;
        }
    }
    // n == 1: single file, PERCENT_RANK = 0.0 (only rank in its own set)

    let mut hotspot_map: HashMap<&str, (f64, f64)> = HashMap::new();
    for (i, row) in hotspot_rows.iter().enumerate() {
        hotspot_map.insert(row.path.as_str(), (row.hotspot_score, rank_by_idx[i]));
    }

    // --- code_health side ---
    let health_rows = run_code_health(db, opts)?;
    let health_band_map: HashMap<&str, &str> = health_rows
        .iter()
        .map(|r| (r.path.as_str(), r.band.as_str()))
        .collect();

    // --- join ---
    let mut rows: Vec<FindingHotspotOverlapRow> = by_path
        .into_iter()
        .map(|(path, pf)| {
            let (hotspot_score, revs_percentile) = hotspot_map
                .get(path.as_str())
                .copied()
                .unwrap_or((0.0, 0.0));
            let health_band = health_band_map
                .get(path.as_str())
                .copied()
                .unwrap_or("unknown")
                .to_owned();
            let mut engines = pf.engines.clone();
            engines.sort_unstable();
            let priority = priority_label(pf.count, revs_percentile, &health_band).to_owned();
            FindingHotspotOverlapRow {
                path,
                findings: pf.count,
                engines: engines.join(","),
                worst_level: pf.worst_level,
                hotspot_score,
                revs_percentile,
                health_band,
                priority,
            }
        })
        .collect();

    // Sort: priority (act-now first → plan → note), then findings desc, then path asc.
    rows.sort_by(|a, b| {
        priority_rank(&a.priority)
            .cmp(&priority_rank(&b.priority))
            .then(b.findings.cmp(&a.findings))
            .then(a.path.cmp(&b.path))
    });

    Ok(rows)
}

fn priority_rank(p: &str) -> u8 {
    match p {
        "act-now" => 0,
        "plan" => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::priority_label;

    #[test]
    fn act_now_requires_all_three_conditions() {
        // All three: findings > 0, revs_percentile >= 0.9, band red
        assert_eq!(priority_label(3, 0.9, "red"), "act-now");
        assert_eq!(priority_label(1, 1.0, "red"), "act-now");
        // Boundary: exactly 0.9
        assert_eq!(priority_label(1, 0.9, "red"), "act-now");
    }

    #[test]
    fn act_now_fails_when_findings_zero() {
        // findings == 0 → cannot be act-now even with perfect percentile + red
        // (impossible in practice but defensive)
        assert_ne!(priority_label(0, 0.95, "red"), "act-now");
    }

    #[test]
    fn act_now_fails_when_percentile_below_threshold() {
        assert_ne!(priority_label(2, 0.89, "red"), "act-now");
    }

    #[test]
    fn act_now_fails_when_band_not_red() {
        assert_ne!(priority_label(2, 0.95, "yellow"), "act-now");
        assert_ne!(priority_label(2, 0.95, "green"), "act-now");
        assert_ne!(priority_label(2, 0.95, "unknown"), "act-now");
    }

    #[test]
    fn plan_via_high_percentile() {
        // revs_percentile >= 0.7 → plan (regardless of band)
        assert_eq!(priority_label(1, 0.7, "green"), "plan");
        assert_eq!(priority_label(1, 0.7, "unknown"), "plan");
        assert_eq!(priority_label(1, 0.8, "yellow"), "plan");
    }

    #[test]
    fn plan_via_red_band() {
        // red band alone → plan
        assert_eq!(priority_label(1, 0.0, "red"), "plan");
        assert_eq!(priority_label(1, 0.5, "red"), "plan");
    }

    #[test]
    fn plan_boundary_exactly_0_7() {
        assert_eq!(priority_label(1, 0.7, "green"), "plan");
        // Below the boundary → note
        assert_eq!(priority_label(1, 0.699_999, "green"), "note");
    }

    #[test]
    fn note_is_default() {
        assert_eq!(priority_label(1, 0.0, "green"), "note");
        assert_eq!(priority_label(1, 0.5, "yellow"), "note");
        assert_eq!(priority_label(1, 0.5, "unknown"), "note");
    }
}
