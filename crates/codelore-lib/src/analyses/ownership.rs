//! Code ownership analysis per spec §1.1 — Fractal Value (1 − HHI).
//!
//! FV ∈ [0, 1); 0 = single owner, → 1 = perfectly fragmented.
//! Inspired by D'Ambros, Gall, Lanza & Pinzger.
//!
//! Also surfaces the main developer (author with highest revision count) per file.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct OwnershipRow {
    pub path: String,
    pub main_author: String,
    pub total_revs: u32,
    pub fractal_value: f64, // [0, 1)
}

// FV = 1 − Σᵢ (aᵢ / nc)²  (HHI complement)
// where aᵢ = author i's distinct revision count, nc = total distinct revisions.
// Window-function FIRST_VALUE + ROW_NUMBER picks the main author (deterministic
// secondary sort on author ASC).
const SQL: &str = "
    WITH author_revs AS (
        SELECT
            changes.path,
            commits.canonical_author AS author,
            COUNT(DISTINCT changes.rev) AS revs
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        GROUP BY changes.path, commits.canonical_author
    ),
    totals AS (
        SELECT path, SUM(revs) AS total
        FROM author_revs
        GROUP BY path
    ),
    with_rank AS (
        SELECT
            ar.path,
            ar.author,
            ar.revs,
            t.total,
            ROW_NUMBER() OVER (
                PARTITION BY ar.path
                ORDER BY ar.revs DESC, ar.author ASC
            ) AS rn
        FROM author_revs ar
        INNER JOIN totals t ON ar.path = t.path
    ),
    main AS (
        SELECT path, author AS main_author
        FROM with_rank
        WHERE rn = 1
    ),
    hhi AS (
        SELECT
            ar.path,
            t.total,
            1.0 - SUM(
                POWER(CAST(ar.revs AS DOUBLE) / NULLIF(CAST(t.total AS DOUBLE), 0), 2)
            ) AS fractal_value
        FROM author_revs ar
        INNER JOIN totals t ON ar.path = t.path
        GROUP BY ar.path, t.total
        HAVING t.total >= ?
    )
    SELECT
        hhi.path,
        main.main_author,
        hhi.total,
        hhi.fractal_value
    FROM hhi
    INNER JOIN main ON main.path = hhi.path
    ORDER BY fractal_value DESC, hhi.path ASC
    LIMIT ?
";

pub fn run_ownership(db: &FactsDb, opts: &Options) -> Result<Vec<OwnershipRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare ownership: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(OwnershipRow {
                path: r.get::<_, String>(0)?,
                main_author: r.get::<_, String>(1)?,
                total_revs: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                fractal_value: r.get::<_, f64>(3)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query ownership: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect ownership: {e}")))
}
