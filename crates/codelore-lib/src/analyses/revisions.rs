//! `revisions` analysis — file → revision count.
//! Code-maat parity output: (entity, n-revs).
//! See spec §1.1 ("authors and revisions are addressable standalone").

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Per-file revision count, gated on `--min-revs` and capped by
/// `--rows`. Bound values: `min_revs`, `row_limit` (`i64::MAX` = unlimited).
/// `{src}` is substituted with the canonical-lineage view (`changes_lineage`)
/// when the flag is on, or `changes` for code-maat-compat parity.
fn build_sql(src: &str) -> String {
    format!(
        "SELECT path, COUNT(DISTINCT rev) AS n_revs
         FROM {src}
         GROUP BY path
         HAVING n_revs >= ?
         ORDER BY n_revs DESC, path ASC
         LIMIT ?"
    )
}

fn source_table(opts: &Options) -> &'static str {
    if opts.use_canonical_lineage {
        "changes_lineage"
    } else {
        "changes"
    }
}

pub fn run_revisions(db: &FactsDb, opts: &Options) -> Result<Vec<(String, u32)>> {
    if opts.use_canonical_lineage {
        crate::facts::ingest::materialize_changes_lineage(db)?;
    }
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let sql = build_sql(source_table(opts));
    let mut stmt = db
        .conn()
        .prepare(&sql)
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
