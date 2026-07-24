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

use crate::analyses::code_health::{CodeHealthRow, run_code_health};
use crate::analyses::hotspots::{HotspotRow, run_hotspots};
use crate::external::{ExternalStore, PathFindings};
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// One row of `finding-hotspot-overlap` output.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FindingHotspotOverlapRow {
    /// File path (repo-relative).
    pub path: String,
    /// Total findings across all engines.
    pub findings: u32,
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

/// Run the `finding-hotspot-overlap` analysis with pre-computed behavioral rows.
///
/// This is the primary implementation. Accepts already-computed `hotspot_rows`
/// and `health_rows` so callers that already hold those results (e.g.
/// `evaluate_all_gates`) can avoid running the analyses a second time.
///
/// Reads all findings from `store`, joins against the supplied rows in Rust,
/// and emits one row per path that has at least one finding, sorted by
/// priority (act-now first), then findings descending, then path ascending.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] with a user-facing message when `store`
/// contains no findings (required pre-condition: the user must run
/// `codelore ingest-sarif` first).
pub fn run_finding_hotspot_overlap_with(
    store: &ExternalStore,
    hotspot_rows: &[HotspotRow],
    health_rows: &[CodeHealthRow],
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
    let revs: Vec<u32> = hotspot_rows.iter().map(|r| r.revisions).collect();
    let rank_by_idx = compute_percent_ranks(&revs);

    let mut hotspot_map: HashMap<&str, (f64, f64)> = HashMap::new();
    for (i, row) in hotspot_rows.iter().enumerate() {
        hotspot_map.insert(row.path.as_str(), (row.hotspot_score, rank_by_idx[i]));
    }

    // --- code_health side ---
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
            let mut engines = pf.engines;
            engines.sort_unstable();
            let priority = priority_label(pf.count, revs_percentile, &health_band).to_owned();
            FindingHotspotOverlapRow {
                path,
                findings: u32::try_from(pf.count).unwrap_or(u32::MAX),
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

/// Run the `finding-hotspot-overlap` analysis.
///
/// Thin wrapper over [`run_finding_hotspot_overlap_with`] that runs
/// [`run_hotspots`] and [`run_code_health`] internally. Use the `_with`
/// variant when those analyses have already been computed to avoid
/// double-running them.
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
    // Percentiles must rank each finding-path against the FULL hotspot /
    // health population, so the inner analyses run unbounded — feeding a
    // `--rows`-truncated set into `compute_percent_ranks` would divide by the
    // wrong denominator and silently drop any finding-path ranked past the
    // limit to `(0.0, "unknown")`. `--rows` instead caps the final,
    // priority-sorted output.
    let full = opts.with_no_row_limit();
    let hotspot_rows = run_hotspots(db, &full)?;
    let health_rows = run_code_health(db, &full)?;
    let mut rows = run_finding_hotspot_overlap_with(store, &hotspot_rows, &health_rows)?;
    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }
    Ok(rows)
}

fn priority_rank(p: &str) -> u8 {
    match p {
        "act-now" => 0,
        "plan" => 1,
        _ => 2,
    }
}

/// Compute SQL-equivalent `PERCENT_RANK` for each entry in `revs`.
///
/// Returns a vec of the same length where `result[i]` is the `PERCENT_RANK` of
/// `revs[i]` within the full set. Tied values receive the **minimum rank of
/// their group**, exactly matching `PERCENT_RANK() OVER (ORDER BY revs)`:
///
/// ```text
/// revs = [5, 5, 10]  →  [0.0, 0.0, 1.0]   (tied 5s both get rank 0)
/// revs = [1, 2, 3]   →  [0.0, 0.5, 1.0]   (no ties)
/// revs = [7]         →  [0.0]              (single entry)
/// ```
fn compute_percent_ranks(revs: &[u32]) -> Vec<f64> {
    let n = revs.len();
    if n == 0 {
        return Vec::new();
    }
    // Build (original_index, revs) pairs, sort by revs ascending.
    let mut sorted: Vec<(usize, u32)> = revs.iter().copied().enumerate().collect();
    sorted.sort_by_key(|&(_, r)| r);

    let mut ranks = vec![0.0f64; n];
    if n == 1 {
        return ranks; // single entry → 0.0
    }
    // Each group spans sorted[i..j]; because groups are contiguous and i
    // advances to j every iteration, i is the count of already-ranked entries —
    // i.e. the group's minimum rank position, matching PERCENT_RANK semantics.
    let mut i = 0usize;
    while i < n {
        let group_revs = sorted[i].1;
        let mut j = i + 1;
        while j < n && sorted[j].1 == group_revs {
            j += 1;
        }
        #[allow(clippy::cast_precision_loss)]
        // PERCENT_RANK: precision loss negligible for repo-scale counts
        let rank = i as f64 / (n - 1) as f64;
        for &(orig_idx, _) in &sorted[i..j] {
            ranks[orig_idx] = rank;
        }
        i = j;
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::{compute_percent_ranks, priority_label};

    // --- compute_percent_ranks ---

    #[test]
    fn tied_revision_counts_get_same_percentile_rank() {
        // SQL PERCENT_RANK semantics: tied values share the MINIMUM rank of
        // their group. [5, 5, 10] → both 5s get rank 0/(3-1) = 0.0; 10 gets
        // rank 2/(3-1) = 1.0.
        let ranks = compute_percent_ranks(&[5, 5, 10]);
        assert_eq!(ranks.len(), 3);
        assert!((ranks[0] - 0.0).abs() < f64::EPSILON, "first 5 → 0.0");
        assert!((ranks[1] - 0.0).abs() < f64::EPSILON, "second 5 → 0.0");
        assert!((ranks[2] - 1.0).abs() < f64::EPSILON, "10 → 1.0");
    }

    #[test]
    fn no_ties_produces_evenly_spaced_ranks() {
        let ranks = compute_percent_ranks(&[1, 2, 3]);
        assert!((ranks[0] - 0.0).abs() < f64::EPSILON);
        assert!((ranks[1] - 0.5).abs() < f64::EPSILON);
        assert!((ranks[2] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn single_entry_rank_is_zero() {
        let ranks = compute_percent_ranks(&[42]);
        assert_eq!(ranks.len(), 1);
        assert!((ranks[0] - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_input_produces_empty_output() {
        assert!(compute_percent_ranks(&[]).is_empty());
    }

    #[test]
    fn all_tied_produces_all_zeros() {
        // All three entries have the same revision count: every rank is 0.0.
        let ranks = compute_percent_ranks(&[7, 7, 7]);
        assert!(ranks.iter().all(|&r| r == 0.0));
    }

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
