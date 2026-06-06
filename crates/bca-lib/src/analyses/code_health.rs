//! Code Health composite analysis per spec §4.6.
//!
//! ```text
//! codehealth(entity) = 100 × (1
//!     - w_cx · normalize(cognitive_complexity)
//!     - w_cn · normalize(churn_rate)
//!     - w_au · normalize(author_fragmentation_FV)
//!     - w_cp · normalize(coupling_centrality_SoC)
//! )
//!
//! defaults: w_cx = 0.40, w_cn = 0.25, w_au = 0.15, w_cp = 0.20
//! ```
//!
//! Plan 3 ships with churn / fragmentation / coupling inputs effectively
//! zero (their analyses don't exist yet); the formula reduces to:
//!
//! ```text
//! codehealth(entity) ≈ 100 × (1 - 0.40 × normalize(cognitive))
//! ```
//!
//! Plan 4 will populate the missing inputs and the full formula will activate.
//!
//! Score range: [0, 100]; higher = healthier.

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};

#[derive(Debug, Clone)]
pub struct CodeHealthRow {
    pub path: String,
    pub name: String,
    pub cognitive: f64,
    pub score: f64, // 0..=100; higher = healthier
}

pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    let limit = opts
        .rows_limit
        .map(|n| format!(" LIMIT {n}"))
        .unwrap_or_default();

    // For Plan 3: only cognitive input is active.
    // normalize() approximates spec §4.6's "repo's empirical 95th percentile" by max.
    // Plan 4 will replace with PERCENTILE_DISC(0.95) for true 95th-percentile normalization.
    let sql = format!(
        "WITH file_complexity AS (
             SELECT path, MAX(cognitive) AS cognitive
             FROM complexity_metrics
             GROUP BY path
         ),
         normalized AS (
             SELECT
                 path,
                 cognitive,
                 CASE
                     WHEN MAX(cognitive) OVER () > 0
                     THEN cognitive / MAX(cognitive) OVER ()
                     ELSE 0
                 END AS norm_cx
             FROM file_complexity
         )
         SELECT
             path,
             '' AS name,
             cognitive,
             GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS score
         FROM normalized
         ORDER BY score ASC, path ASC{limit}",
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| BcaError::Analysis(format!("prepare code-health: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CodeHealthRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                cognitive: r.get::<_, f64>(2)?,
                score: r.get::<_, f64>(3)?,
            })
        })
        .map_err(|e| BcaError::Analysis(format!("query code-health: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| BcaError::Analysis(format!("collect code-health: {e}")))
}
