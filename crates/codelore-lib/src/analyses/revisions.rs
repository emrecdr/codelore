//! `revisions` analysis — file → revision count.
//! Code-maat parity output: (entity, n-revs).
//! See spec §1.1 ("authors and revisions are addressable standalone").

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Per-file revision count, gated on `--min-revs` and capped by
/// `--rows`. Bound values: `min_revs`, `row_limit` (`i64::MAX` = unlimited).
const SQL: &str = "
    SELECT path, COUNT(DISTINCT rev) AS n_revs
    FROM changes
    GROUP BY path
    HAVING n_revs >= ?
    ORDER BY n_revs DESC, path ASC
    LIMIT ?
";

pub fn run_revisions(db: &FactsDb, opts: &Options) -> Result<Vec<(String, u32)>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare revisions: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query revisions: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect revisions: {e}")))
}
