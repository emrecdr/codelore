//! `refactoring-targets` analysis.
//!
//! Ranks files by return-on-investment for refactoring: the intersection of
//! low code health and high development activity, divided by inspection
//! effort. `priority = (structural_risk × hotspot_score) / max(loc, floor)` —
//! an effort-aware ranking so a small, dense, churning, unhealthy file
//! outranks a large one with the same raw risk. Reuses the `code-health`
//! composite (which also materialises the per-file biomarker table) and the
//! `hotspots` activity signal; joins them per file.
//!
//! **Join semantic**: results contain only files present in *both* the
//! code-health output (files with parseable complexity surviving `min_revs`)
//! and the hotspots output; because the code-health file set is a strict
//! subset of the hotspots file set, code-health is the binding constraint —
//! a churning file with no parseable complexity (unsupported, vendored, or
//! skipped by the tree-sitter walker) will not appear in the output.
//!
//! Research basis: effort-aware defect ranking (risk per unit inspection
//! effort; Popt / `PofB20`) with an EA-Z-style size floor to avoid tiny-file
//! ranking artifacts.

use std::collections::HashMap;

use crate::analyses::code_health::run_code_health;
use crate::analyses::hotspots::run_hotspots;
use crate::analyses::query::query_map_collect;
use crate::facts::FactsDb;
use crate::{Options, Result};

/// EA-Z-style effort floor: files smaller than this are treated as this many
/// lines when dividing risk by effort, so a 3-line file cannot dominate the
/// ranking on a near-zero denominator.
const EA_Z_FLOOR: u32 = 25;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefactoringTargetRow {
    pub path: String,
    /// `(structural_risk × hotspot_score) / max(loc, EA_Z_FLOOR)`. Higher = refactor sooner.
    pub priority: f64,
    /// `structural_risk × hotspot_score` (health deficit × hotspotness), pre-effort.
    pub combined_risk: f64,
    pub structural_risk: f64,
    pub hotspot_score: f64,
    pub revisions: u32,
    pub loc: u32,
    /// Dominant biomarker smell for this file, or `"none"` if no biomarker is recorded.
    pub dominant_type: String,
    pub band: String,
    /// `ManualUp` baseline rank: 1-based, ascending by `loc` (smallest file = 1),
    /// ties broken by `path`. Assigned over the full set before any truncation.
    pub manual_up_rank: u32,
}

/// Run the `refactoring-targets` analysis. Returns rows ranked by `priority`
/// DESC (worst-ROI-debt first), truncated to `opts.rows_limit`.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "refactoring-targets", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_refactoring_targets(db: &FactsDb, opts: &Options) -> Result<Vec<RefactoringTargetRow>> {
    // Row-limit discipline: the inner analyses must see the FULL file set, or
    // ranking would be computed over a truncated input. Truncate only the
    // final sorted output.
    let full = opts.with_no_row_limit();

    // run_code_health ALSO materialises `code_health_biomarkers_v1` on the
    // connection (used by biomarker enrichment for the dominant biomarker).
    let health = run_code_health(db, &full)?;
    let hotspots = run_hotspots(db, &full)?;

    // Per-file LOC (effort). Raw `complexity_metrics` — the grouped table omits `loc`.
    let loc_by_path: HashMap<String, u32> = query_map_collect(
        db,
        "SELECT path, MAX(loc) AS loc FROM complexity_metrics WHERE loc IS NOT NULL GROUP BY path",
        [],
        "refactoring-targets:loc",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1).map(|v| u32::try_from(v).unwrap_or(0))?,
            ))
        },
    )?
    .into_iter()
    .collect();

    // Index hotspots by path for the join.
    let hs_by_path: HashMap<&str, &crate::analyses::hotspots::HotspotRow> =
        hotspots.iter().map(|h| (h.path.as_str(), h)).collect();

    // Dominant biomarker per file: highest-intensity smell, ties broken by
    // smell name so the pick is deterministic. Reads the temp table that
    // run_code_health materialised above.
    let dominant_by_path: HashMap<String, String> = query_map_collect(
        db,
        "SELECT path, smell FROM ( \
             SELECT path, smell, \
                    ROW_NUMBER() OVER (PARTITION BY path ORDER BY intensity DESC, smell ASC) AS rn \
             FROM code_health_biomarkers_v1 \
         ) WHERE rn = 1",
        [],
        "refactoring-targets:dominant",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?
    .into_iter()
    .collect();

    let mut rows: Vec<RefactoringTargetRow> = health
        .iter()
        .filter_map(|h| {
            // Only files that are BOTH scored for health AND appear as hotspots.
            let hs = hs_by_path.get(h.path.as_str())?;
            let loc = loc_by_path.get(&h.path).copied().unwrap_or(0).max(1);
            let combined_risk = h.structural_risk * hs.hotspot_score;
            let priority = combined_risk / f64::from(loc.max(EA_Z_FLOOR));
            Some(RefactoringTargetRow {
                path: h.path.clone(),
                priority,
                combined_risk,
                structural_risk: h.structural_risk,
                hotspot_score: hs.hotspot_score,
                revisions: hs.revisions,
                loc,
                dominant_type: dominant_by_path
                    .get(&h.path)
                    .cloned()
                    .unwrap_or_else(|| "none".to_owned()),
                band: h.band.clone(),
                manual_up_rank: 0,
            })
        })
        .collect();

    // Deterministic sort: priority DESC, then path ASC as a stable tie-break.
    rows.sort_by(|a, b| {
        b.priority
            .partial_cmp(&a.priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    // ManualUp baseline: rank by ascending size (smallest first). Computed over
    // the full set so the rank is stable regardless of the priority truncation.
    let mut by_size: Vec<usize> = (0..rows.len()).collect();
    by_size.sort_by(|&i, &j| {
        rows[i]
            .loc
            .cmp(&rows[j].loc)
            .then_with(|| rows[i].path.cmp(&rows[j].path))
    });
    for (rank, &idx) in by_size.iter().enumerate() {
        rows[idx].manual_up_rank = u32::try_from(rank + 1).unwrap_or(u32::MAX);
    }

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }
    Ok(rows)
}
