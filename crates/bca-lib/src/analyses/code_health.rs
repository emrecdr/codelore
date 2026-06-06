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
//! All 4 inputs are wired. Normalization uses the in-repo maximum as the
//! empirical upper bound (min-max over the result set). Score range: [0, 100];
//! higher = healthier.

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};

#[derive(Debug, Clone)]
pub struct CodeHealthRow {
    pub path: String,
    pub name: String,
    pub cognitive: f64,
    pub score: f64, // 0..=100; higher = healthier
}

#[allow(clippy::too_many_lines)]
pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    let limit = opts
        .rows_limit
        .map(|n| format!(" LIMIT {n}"))
        .unwrap_or_default();
    let min_revs = opts.min_revs;

    let sql = format!(
        "WITH file_cognitive AS (
             SELECT path, MAX(cognitive) AS cognitive
             FROM complexity_metrics
             GROUP BY path
         ),
         file_churn AS (
             SELECT path, COALESCE(SUM(loc_added), 0) + COALESCE(SUM(loc_deleted), 0) AS churn
             FROM changes
             GROUP BY path
         ),
         file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs
             FROM changes
             GROUP BY path
             HAVING revs >= {min_revs}
         ),
         author_revs AS (
             SELECT
                 changes.path,
                 commits.canonical_author AS author,
                 COUNT(DISTINCT changes.rev) AS revs
             FROM changes
             INNER JOIN commits ON changes.rev = commits.rev
             GROUP BY changes.path, commits.canonical_author
         ),
         file_fv AS (
             SELECT
                 ar.path,
                 1.0 - SUM(POWER(ar.revs::DOUBLE / NULLIF(t.total, 0), 2)) AS fv
             FROM author_revs ar
             INNER JOIN (SELECT path, SUM(revs) AS total FROM author_revs GROUP BY path) t
                 ON ar.path = t.path
             GROUP BY ar.path
         ),
         file_coupling AS (
             SELECT path, COUNT(*) AS centrality FROM (
                 SELECT a.path AS path
                 FROM changes a
                 INNER JOIN changes b ON a.rev = b.rev AND a.path < b.path
                 GROUP BY a.path, b.path
                 UNION ALL
                 SELECT b.path
                 FROM changes a
                 INNER JOIN changes b ON a.rev = b.rev AND a.path < b.path
                 GROUP BY a.path, b.path
             ) GROUP BY path
         ),
         joined AS (
             SELECT
                 fc.path,
                 fc.cognitive,
                 COALESCE(fch.churn, 0) AS churn,
                 COALESCE(ffv.fv, 0.0) AS fv,
                 COALESCE(fcp.centrality, 0) AS centrality
             FROM file_cognitive fc
             INNER JOIN file_revs fr ON fc.path = fr.path
             LEFT JOIN file_churn fch ON fc.path = fch.path
             LEFT JOIN file_fv ffv ON fc.path = ffv.path
             LEFT JOIN file_coupling fcp ON fc.path = fcp.path
         ),
         normalized AS (
             SELECT
                 path,
                 cognitive,
                 churn,
                 fv,
                 centrality,
                 CASE WHEN MAX(cognitive) OVER () > 0 THEN cognitive / MAX(cognitive) OVER () ELSE 0 END AS n_cx,
                 CASE WHEN MAX(churn) OVER () > 0 THEN churn::DOUBLE / MAX(churn) OVER () ELSE 0 END AS n_cn,
                 fv AS n_au,
                 CASE WHEN MAX(centrality) OVER () > 0 THEN centrality::DOUBLE / MAX(centrality) OVER () ELSE 0 END AS n_cp
             FROM joined
         )
         SELECT
             path,
             '' AS name,
             cognitive,
             GREATEST(0.0, LEAST(100.0,
                 100.0 * (1.0
                     - 0.40 * n_cx
                     - 0.25 * n_cn
                     - 0.15 * n_au
                     - 0.20 * n_cp
                 )
             )) AS score
         FROM normalized
         ORDER BY score ASC, path ASC{limit}"
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
