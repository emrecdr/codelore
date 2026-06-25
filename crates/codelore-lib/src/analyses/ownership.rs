//! Code ownership analysis per spec §1.1 — Fractal Value (1 − HHI).
//!
//! FV ∈ [0, 1); 0 = single owner, → 1 = perfectly fragmented.
//! Inspired by D'Ambros, Gall, Lanza & Pinzger.
//!
//! Also surfaces the main developer (author with highest revision count) per file.
//!
//! Research basis: see `docs/research-foundations.md` entry "ownership"
//! (Mockus & Herbsleb, ICSE 2002 — expertise concentration measurement;
//! Hirschman 1980 — Herfindahl–Hirschman concentration index borrowed
//! from industrial organisation).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct OwnershipRow {
    pub path: String,
    pub main_author: String,
    pub total_revs: u32,
    pub fractal_value: f64, // [0, 1)
}

// FV = 1 − Σᵢ (aᵢ / nc)²  (HHI complement)
// where aᵢ = author i's distinct revision count, nc = total distinct revisions.
// `first(author ORDER BY revs DESC, author ASC)` picks the main author per
// path in one aggregate — replaces the older ROW_NUMBER+self-join pattern.
const SQL: &str = "
    WITH author_revs AS (
        SELECT
            changes.path,
            commits.canonical_author AS author,
            -- (rev, path) is the changes PK so rev is unique within each
            -- (path, author) group. Plain COUNT skips DuckDB's
            -- distinct-tracking overhead.
            COUNT(changes.rev) AS revs
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        GROUP BY changes.path, commits.canonical_author
    ),
    totals AS (
        SELECT path, SUM(revs) AS total
        FROM author_revs
        GROUP BY path
    ),
    hhi AS (
        SELECT
            ar.path,
            t.total,
            first(ar.author ORDER BY ar.revs DESC, ar.author ASC) AS main_author,
            1.0 - SUM(
                POWER(CAST(ar.revs AS DOUBLE) / NULLIF(CAST(t.total AS DOUBLE), 0), 2)
            ) AS fractal_value
        FROM author_revs ar
        INNER JOIN totals t ON ar.path = t.path
        GROUP BY ar.path, t.total
        HAVING t.total >= ?
    )
    SELECT
        path,
        main_author,
        total,
        fractal_value
    FROM hhi
    ORDER BY fractal_value DESC, path ASC
    LIMIT ?
";

#[tracing::instrument(name = "ownership", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_ownership(db: &FactsDb, opts: &Options) -> Result<Vec<OwnershipRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "ownership",
        opts,
    )?;
    crate::analyses::query::query_map_collect(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "ownership",
        |r| {
            Ok(OwnershipRow {
                path: r.get::<_, String>(0)?,
                main_author: r.get::<_, String>(1)?,
                total_revs: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                fractal_value: r.get::<_, f64>(3)?,
            })
        },
    )
}
