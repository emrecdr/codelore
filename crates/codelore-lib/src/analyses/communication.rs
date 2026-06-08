//! Communication analysis per spec §1.1 — Conway's law shared-work author pairs.
//!
//! For each pair of authors who co-edit the same files, compute:
//! - shared: distinct paths they both edited
//! - average: mean of their individual total commits
//! - strength: 100 × shared / average (percentage)
//!
//! Self-pairs are excluded.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunicationRow {
    pub author_a: String,
    pub author_b: String,
    pub shared: u32,   // distinct paths both authors touched
    pub average: u32,  // mean of authors' total commits
    pub strength: f64, // 100 * shared / average
}

const SQL: &str = "
    WITH author_files AS (
        SELECT DISTINCT
            changes.path,
            commits.canonical_author AS author
        FROM commits
        INNER JOIN changes ON changes.rev = commits.rev
    ),
    pairs AS (
        SELECT
            a.author AS author_a,
            b.author AS author_b,
            COUNT(DISTINCT a.path) AS shared
        FROM author_files a
        INNER JOIN author_files b ON a.path = b.path AND a.author < b.author
        GROUP BY a.author, b.author
        HAVING shared >= ?
    ),
    totals AS (
        SELECT
            canonical_author AS author,
            COUNT(DISTINCT rev) AS commits
        FROM commits
        GROUP BY canonical_author
    )
    SELECT
        p.author_a,
        p.author_b,
        p.shared,
        (ta.commits + tb.commits) / 2 AS average,
        100.0 * p.shared / NULLIF((ta.commits + tb.commits) / 2.0, 0) AS strength
    FROM pairs p
    INNER JOIN totals ta ON ta.author = p.author_a
    INNER JOIN totals tb ON tb.author = p.author_b
    ORDER BY strength DESC, p.author_a ASC, p.author_b ASC
    LIMIT ?
";

pub fn run_communication(db: &FactsDb, opts: &Options) -> Result<Vec<CommunicationRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        SQL,
        params![opts.min_shared_revs, row_limit],
        "communication",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare communication: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_shared_revs, row_limit], |r| {
            Ok(CommunicationRow {
                author_a: r.get::<_, String>(0)?,
                author_b: r.get::<_, String>(1)?,
                shared: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                average: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
                strength: r.get::<_, f64>(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query communication: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect communication: {e}")))
}
