//! Parquet output via `DuckDB` COPY ... TO ... (FORMAT PARQUET).
//!
//! Plan 5 ships Parquet for: hotspots, revisions, summary. Other analyses
//! can be added trivially by following the pattern.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};
use std::path::Path;

fn copy_to_parquet(db: &FactsDb, query: &str, path: &Path) -> Result<()> {
    // Escape single quotes in path (DuckDB COPY uses SQL string literals)
    let path_str = path.display().to_string().replace('\'', "''");
    let sql = format!("COPY ({query}) TO '{path_str}' (FORMAT PARQUET);");
    db.conn()
        .execute_batch(&sql)
        .map_err(|e| CodeLoreError::Output(format!("parquet: {e}")))?;
    Ok(())
}

pub fn write_hotspots_parquet(db: &FactsDb, opts: &Options, path: &Path) -> Result<()> {
    // Mirror analyses::hotspots::SQL so Parquet output matches CSV/JSON/Markdown
    // exactly (entity, revs, cognitive, code_health, score). Earlier this writer
    // emitted only `entity, revs, cognitive` and silently dropped the two
    // computed columns users actually care about. Same formula as
    // `run_hotspots`: score is `percent_rank(revs) * percent_rank(cog) *
    // (100 − code_health) / 10`, range `[0, 10]`.
    // Route through `changes_lineage` so Parquet output stays in sync with
    // the CSV/JSON/Markdown emitters when canonical lineage is on.
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let min_revs = opts.min_revs;
    let row_limit = opts.rows_limit.map_or(i64::MAX, i64::from);
    let query = format!(
        "WITH file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs
             FROM {src} GROUP BY path HAVING revs >= {min_revs}
         ),
         file_complexity AS (
             SELECT path, MAX(cognitive) AS cognitive
             FROM complexity_metrics GROUP BY path
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
                 PERCENT_RANK() OVER (ORDER BY revs)      AS pr_rev,
                 PERCENT_RANK() OVER (ORDER BY cognitive) AS pr_cx,
                 CASE
                     WHEN MAX(cognitive) OVER () > 0
                     THEN cognitive / MAX(cognitive) OVER ()
                     ELSE 0
                 END AS norm_cx
             FROM joined
         )
         SELECT
             path AS entity,
             revs,
             cognitive,
             GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS code_health,
             pr_rev * pr_cx * (100.0 - GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx)))) / 10.0 AS score
         FROM ranked
         ORDER BY score DESC, path ASC
         LIMIT {row_limit}"
    );
    copy_to_parquet(db, &query, path)
}

pub fn write_revisions_parquet(db: &FactsDb, opts: &Options, path: &Path) -> Result<()> {
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let min_revs = opts.min_revs;
    let query = format!(
        "SELECT path AS entity, COUNT(DISTINCT rev) AS n_revs
         FROM {src}
         GROUP BY path
         HAVING n_revs >= {min_revs}
         ORDER BY n_revs DESC, path ASC"
    );
    copy_to_parquet(db, &query, path)
}

pub fn write_summary_parquet(db: &FactsDb, _opts: &Options, path: &Path) -> Result<()> {
    let query = "
        SELECT 'commits' AS metric, COUNT(*) AS value FROM commits
        UNION ALL SELECT 'changes', COUNT(*) FROM changes
        UNION ALL SELECT 'entities', COUNT(*) FROM entities
        UNION ALL SELECT 'authors', COUNT(DISTINCT canonical_author) FROM commits
    "
    .to_string();
    copy_to_parquet(db, &query, path)
}
