//! Parquet output via `DuckDB` COPY ... TO ... (FORMAT PARQUET).
//!
//! Parquet is supported for: hotspots, revisions, summary. Other analyses
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
    // Single source of truth: `analyses::hotspots::build_inlined_sql`. Any
    // change to the hotspots formula (revs / cognitive / cognitive-health /
    // score) propagates to Parquet output automatically — no risk of the
    // two paths drifting. Previously this writer carried a verbatim copy
    // of the SQL that had to be hand-synced after every formula change.
    crate::analyses::lineage::materialize_source(db, opts)?;
    let cm_src = crate::analyses::grouped_complexity::source_table(opts);
    let row_limit = opts.rows_limit.map_or(i64::MAX, i64::from);
    let query =
        crate::analyses::hotspots::build_inlined_sql(opts, cm_src, opts.min_revs, row_limit);
    copy_to_parquet(db, &query, path)
}

pub fn write_revisions_parquet(db: &FactsDb, opts: &Options, path: &Path) -> Result<()> {
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let query = crate::analyses::revisions::build_inlined_sql(src, opts.min_revs);
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
