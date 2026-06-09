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
    // DEEP-15: Under `--code-maat-compat`, emit code-maat's exact statistic
    // names (hyphenated `number-of-X`) so downstream scripts parsing CSV
    // like `if statistic == "number-of-commits"` keep working. The
    // CodeLore modern default uses concise names (`commits`, `entities`)
    // because the row label is already self-explanatory in the modern
    // surface where the column header reads `metric`.
    let sql = if opts.code_maat_compat {
        "
        SELECT 'number-of-commits' AS metric, COUNT(*) AS value FROM commits
        UNION ALL
        SELECT 'number-of-entities', COUNT(*) FROM entities
        UNION ALL
        SELECT 'number-of-entities-changed', COUNT(*) FROM changes
        UNION ALL
        SELECT 'number-of-authors', COUNT(DISTINCT canonical_author) FROM commits;
    "
    } else {
        "
        SELECT 'commits' AS metric, COUNT(*) AS value FROM commits
        UNION ALL
        SELECT 'changes', COUNT(*) FROM changes
        UNION ALL
        SELECT 'entities', COUNT(*) FROM entities
        UNION ALL
        SELECT 'authors', COUNT(DISTINCT canonical_author) FROM commits;
    "
    };
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
