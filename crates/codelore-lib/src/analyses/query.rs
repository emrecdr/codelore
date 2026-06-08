//! Shared `prepare → query_map → collect → format!()-wrapped errors`
//! boilerplate. Before this, 13 analyses copy-pasted the same 7-line
//! pattern with only the SQL constant, params, and mapper closure
//! varying. Drift risk on the error-message format was a real concern
//! flagged by the v0.1.1 cleanup review.
//!
//! Usage:
//!
//! ```ignore
//! use crate::analyses::query::query_map_collect;
//! let rows: Vec<MyRow> = query_map_collect(
//!     db, &sql, duckdb::params![opts.min_revs, row_limit], "my-analysis",
//!     |r| Ok(MyRow { x: r.get(0)?, y: r.get(1)? }),
//! )?;
//! ```

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Prepare + `query_map` + collect, with uniform `CodeLoreError::Analysis`
/// error context at each step. `label` is interpolated into the error
/// messages so debug output identifies which analysis failed.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on prepare, query, or row-mapping
/// failure; the error message is `"<step> <label>: <underlying>"`.
pub fn query_map_collect<T, P, F>(
    db: &FactsDb,
    sql: &str,
    params: P,
    label: &str,
    mut mapper: F,
) -> Result<Vec<T>>
where
    P: duckdb::Params,
    F: FnMut(&duckdb::Row<'_>) -> duckdb::Result<T>,
{
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare {label}: {e}")))?;
    let rows = stmt
        .query_map(params, |r| mapper(r))
        .map_err(|e| CodeLoreError::Analysis(format!("query {label}: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect {label}: {e}")))
}

/// Emit the `DuckDB` EXPLAIN plan for `sql` + `params` to stderr if
/// `opts.explain` is on. No-op otherwise. Shared so every analysis can
/// add `--explain` support in one line instead of copying the
/// `if opts.explain { db.explain_sql(...)?; eprintln!(...); }` block.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] only if `--explain` is on AND
/// `db.explain_sql` fails. Off path is infallible.
pub fn explain_if_requested<P: duckdb::Params>(
    db: &FactsDb,
    sql: &str,
    params: P,
    label: &str,
    opts: &Options,
) -> Result<()> {
    if !opts.explain {
        return Ok(());
    }
    let plan = db.explain_sql(sql, params)?;
    eprintln!("--- EXPLAIN: {label} ---\n{plan}---");
    Ok(())
}
