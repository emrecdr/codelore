//! Summary analysis per spec §1.1 — 4-row repo overview.
//!
//! Research basis: see `docs/research-foundations.md` entry "summary"
//! (diagnostic analysis; no single foundational citation — used for
//! ingest sanity-checking, repo-comparison overviews, and CI dashboard
//! "is the data healthy?" panels).

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SummaryRow {
    pub metric: String,
    pub value: i64,
}

pub fn run_summary(db: &FactsDb, opts: &Options) -> Result<Vec<SummaryRow>> {
    let sql = "
        SELECT 'commits' AS metric, COUNT(*) AS value FROM commits
        UNION ALL
        SELECT 'changes', COUNT(*) FROM changes
        UNION ALL
        SELECT 'entities', COUNT(*) FROM entities
        UNION ALL
        SELECT 'authors', COUNT(DISTINCT canonical_author) FROM commits;
    ";
    crate::analyses::query::explain_if_requested(db, sql, [], "summary", opts)?;
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare summary: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SummaryRow {
                metric: r.get::<_, String>(0)?,
                value: r.get::<_, i64>(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query summary: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect summary: {e}")))
}
