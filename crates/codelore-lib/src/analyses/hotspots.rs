//! Hotspot ranking analysis. `code_health` is on `[0, 100]` (higher = healthier);
//! `percentile_rank` is on `[0, 1]`; the score formula combines them so unhealthy +
//! frequently-changed + complex files rank highest. Output range is `[0, 10]`:
//!
//! ```text
//!   hotspot_score(entity) = percentile_rank(revisions)
//!                         × percentile_rank(cognitive_complexity)
//!                         × (100 − code_health) / 10
//! ```
//!
//! Note: an earlier version used `(10 − code_health) / 10` which produced
//! negative scores because `code_health` is on the `[0, 100]` scale, not
//! `[0, 10]`. The published formula now divides by 10 (not 100) so the
//! resulting `[0, 10]` range matches what consumers (CSV, Markdown, SARIF
//! emitters) display.
//!
//! `code_health` itself is computed inline using cognitive complexity only;
//! the churn / fragmentation / coupling inputs from `code_health` analysis
//! aren't reused here — `hotspots` is the lightweight "what to look at first"
//! ranking, `code-health` is the deeper analysis.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HotspotRow {
    pub path: String,
    pub revisions: u32,
    pub cognitive: f64,
    pub code_health: f64,
    pub hotspot_score: f64,
}

// PERCENT_RANK() is standard SQL:2003 and is supported by DuckDB ≥0.2.
// Aggregate per-path:
//   revisions:   count of distinct commits in changes
//   cognitive:   MAX of cognitive across all entities in the path's file
//   code_health: 100 * (1 − 0.40 * normalize(cognitive))   ∈ [0, 100]
//   hotspot_score: percent_rank(revs) * percent_rank(cog) * (100 − code_health) / 10
//                  ∈ [0, 10] — divide by 10 not 100 so emitted values stay
//                  on the same scale as code_health (0–10 hotspots, ≈10 = on fire).
pub const SQL: &str = "
    WITH file_revs AS (
        SELECT path, COUNT(DISTINCT rev) AS revs
        FROM changes
        GROUP BY path
        HAVING revs >= ?
    ),
    file_complexity AS (
        SELECT path, MAX(cognitive) AS cognitive
        FROM complexity_metrics
        GROUP BY path
    ),
    joined AS (
        SELECT
            fr.path,
            fr.revs,
            COALESCE(fc.cognitive, 0) AS cognitive
        FROM file_revs fr
        LEFT JOIN file_complexity fc ON fc.path = fr.path
    ),
    ranked AS (
        SELECT
            path,
            revs,
            cognitive,
            PERCENT_RANK() OVER (ORDER BY revs) AS pr_rev,
            PERCENT_RANK() OVER (ORDER BY cognitive) AS pr_cx,
            CASE
                WHEN MAX(cognitive) OVER () > 0
                THEN cognitive / MAX(cognitive) OVER ()
                ELSE 0
            END AS norm_cx
        FROM joined
    )
    SELECT
        path,
        revs,
        cognitive,
        GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS code_health,
        pr_rev * pr_cx * (100.0 - GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx)))) / 10.0 AS score
    FROM ranked
    ORDER BY score DESC, path ASC
    LIMIT ?
";

pub fn run_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    if opts.explain {
        let plan = db.explain_sql(SQL, params![opts.min_revs, row_limit])?;
        eprintln!("--- EXPLAIN: hotspots ---\n{plan}---");
    }
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare hotspots: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(HotspotRow {
                path: r.get::<_, String>(0)?,
                revisions: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(2)?,
                code_health: r.get::<_, f64>(3)?,
                hotspot_score: r.get::<_, f64>(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query hotspots: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect hotspots: {e}")))
}
