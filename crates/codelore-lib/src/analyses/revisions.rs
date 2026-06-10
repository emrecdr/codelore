//! `revisions` analysis — file → revision count.
//! Code-maat parity output: (entity, n-revs).
//! See spec §1.1 ("authors and revisions are addressable standalone").
//!
//! Research basis: see `docs/research-foundations.md` entry "revisions"
//! (Nagappan & Ball, ICSE 2005 — relative churn predicts defect density).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

/// Per-file revision count, gated on `--min-revs` and capped by
/// `--rows`. Bound values: `min_revs`, `row_limit` (`i64::MAX` = unlimited).
pub const SQL_RAW: &str = "
    -- `changes` has PRIMARY KEY (rev, path), so COUNT(rev) == COUNT(*) ==
    -- COUNT(DISTINCT rev) for any GROUP BY path. The plain COUNT skips
    -- DuckDB's distinct-tracking overhead on a column that is already
    -- unique within the group.
    SELECT path, COUNT(rev) AS n_revs
    FROM changes
    GROUP BY path
    HAVING n_revs >= ?
    ORDER BY n_revs DESC, path ASC
    LIMIT ?
";

/// Returns the revisions SQL with `?` placeholders inlined and the
/// source table swapped in. Used by the Parquet writer (`DuckDB COPY`
/// can't accept bind parameters). Sharing the formula with the live
/// `run_revisions` path eliminates silent-drift risk.
#[must_use]
pub fn build_inlined_sql(src: &str, min_revs: u32) -> String {
    // SQL has two `?` placeholders: min_revs (HAVING) then row_limit (LIMIT).
    // Parquet output is unbounded by design (binary export), so the
    // row_limit becomes `i64::MAX`.
    SQL_RAW
        .replace("FROM changes\n", &format!("FROM {src}\n"))
        .replacen('?', &min_revs.to_string(), 1)
        .replace('?', "9223372036854775807")
}

pub fn run_revisions(db: &FactsDb, opts: &Options) -> Result<Vec<(String, u32)>> {
    super::lineage::materialize_if_needed(db, opts)?;
    let sql = super::lineage::rewrite(SQL_RAW, opts);
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    super::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "revisions",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "revisions",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
            ))
        },
    )
}
