//! Hotspot ranking analysis per spec §1.1 published formula:
//!
//! ```text
//!   hotspot_score(entity) = percentile_rank(revisions)
//!                         × percentile_rank(cognitive_complexity)
//!                         × (10 − code_health) / 10
//! ```
//!
//! In Plan 3, `code_health` is computed inline using cognitive complexity only;
//! the spec §4.6 inputs for churn / fragmentation / coupling have weight 0 until
//! Plan 4 ships their analyses.

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};

#[derive(Debug, Clone)]
pub struct HotspotRow {
    pub path: String,
    pub name: String,
    pub revisions: u32,
    pub cognitive: f64,
    pub code_health: f64,
    pub hotspot_score: f64,
}

pub fn run_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>> {
    let limit = opts
        .rows_limit
        .map(|n| format!(" LIMIT {n}"))
        .unwrap_or_default();

    // Aggregate per-path:
    //  - revisions: count of distinct commits in changes
    //  - cognitive: MAX of cognitive across all entities in the path's file
    //  - code_health: 100 * (1 - 0.40 * normalize(cognitive))
    //  - hotspot_score: percent_rank(revs) * percent_rank(cog) * (10 - code_health) / 10
    //
    // PERCENT_RANK() is standard SQL:2003 and is supported by DuckDB ≥0.2.
    // The fallback (RANK()-1)/(COUNT(*) OVER ()-1) is equivalent.
    let sql = format!(
        "WITH file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs
             FROM changes
             GROUP BY path
             HAVING revs >= {min}
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
             '' AS name,
             revs,
             cognitive,
             GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS code_health,
             pr_rev * pr_cx * (10.0 - GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx)))) / 10.0 AS score
         FROM ranked
         ORDER BY score DESC, path ASC{limit}",
        min = opts.min_revs,
        limit = limit,
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| BcaError::Analysis(format!("prepare hotspots: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HotspotRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                revisions: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(3)?,
                code_health: r.get::<_, f64>(4)?,
                hotspot_score: r.get::<_, f64>(5)?,
            })
        })
        .map_err(|e| BcaError::Analysis(format!("query hotspots: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| BcaError::Analysis(format!("collect hotspots: {e}")))
}
