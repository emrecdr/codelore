//! Parquet output via `DuckDB` COPY ... TO ... (FORMAT PARQUET).
//!
//! Plan 5 ships Parquet for: hotspots, revisions, summary. Other analyses
//! can be added trivially by following the pattern.

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};
use std::path::Path;

fn copy_to_parquet(db: &FactsDb, query: &str, path: &Path) -> Result<()> {
    // Escape single quotes in path (DuckDB COPY uses SQL string literals)
    let path_str = path.display().to_string().replace('\'', "''");
    let sql = format!("COPY ({query}) TO '{path_str}' (FORMAT PARQUET);");
    db.conn()
        .execute_batch(&sql)
        .map_err(|e| BcaError::Output(format!("parquet: {e}")))?;
    Ok(())
}

pub fn write_hotspots_parquet(db: &FactsDb, opts: &Options, path: &Path) -> Result<()> {
    let min_revs = opts.min_revs;
    let query = format!(
        "WITH file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs
             FROM changes GROUP BY path HAVING revs >= {min_revs}
         ),
         file_complexity AS (
             SELECT path, MAX(cognitive) AS cognitive
             FROM complexity_metrics GROUP BY path
         )
         SELECT fr.path AS entity, fr.revs, fc.cognitive
         FROM file_revs fr
         LEFT JOIN file_complexity fc ON fr.path = fc.path
         ORDER BY fr.revs DESC, fr.path ASC"
    );
    copy_to_parquet(db, &query, path)
}

pub fn write_revisions_parquet(db: &FactsDb, opts: &Options, path: &Path) -> Result<()> {
    let min_revs = opts.min_revs;
    let query = format!(
        "SELECT path AS entity, COUNT(DISTINCT rev) AS n_revs
         FROM changes
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
