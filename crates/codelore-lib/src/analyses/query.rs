//! Shared `prepare → query_map → collect → format!()-wrapped errors`
//! boilerplate. Without this helper every analysis would copy-paste
//! the same 7-line pattern with only the SQL constant, params, and
//! mapper closure varying, and the error-message format would drift
//! across analyses.
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

/// The current UTC instant as a `YYYY-MM-DD HH:MM:SS` string — the exact
/// UTC-naive frame and format commit dates are stored in
/// (`facts::ingest::consumer`'s timestamp formatter), for embedding as a
/// `DuckDB` `TIMESTAMP` literal.
///
/// "Now" is resolved in Rust rather than through SQL `now()` / `timezone()` /
/// `AT TIME ZONE` deliberately: those are `DuckDB`'s ICU-extension functions,
/// they render in the session timezone (so a bare `CAST(now() AS TIMESTAMP)`
/// on a runner behind UTC could clamp a commit made minutes ago), and the
/// value they produce carries an ICU timestamp type whose `- INTERVAL`
/// operator does not bind in every position a subquery embeds it. A plain
/// `TIMESTAMP` literal sidesteps all three, and matches how `code-age` and
/// `knowledge-islands` already resolve their wall-clock anchor.
#[must_use]
pub fn wall_clock_utc_literal() -> String {
    crate::facts::ingest::consumer::format_timestamp(time::OffsetDateTime::now_utc())
}

/// SQL expression for the repository's window anchor — the "now" that every
/// trailing-window and time-decay term is measured against — clamped so a
/// single future-dated commit cannot become "now" for the whole analysis.
///
/// Drops in wherever a data-controlled anchor previously read a bare
/// `MAX(<col>)` over `commits`, emitting `LEAST(MAX(<col>), TIMESTAMP
/// '<utc-now>')`. `col` is the `commits` timestamp column to anchor on:
/// `"date"` (author date — the anchor of nearly every window) or
/// `"committer_date"`. The clamp caps the anchor at the wall clock: a
/// timestamp set to the far future (a bad `GIT_AUTHOR_DATE`, contributor clock
/// skew, or a mis-imported commit) would otherwise collapse active-author
/// windows, underflow the knowledge-decay terms, and shift the new-code
/// born/touched partition — every one of which anchors on `MAX(commits.date)`
/// as "now". The "now" literal comes from [`wall_clock_utc_literal`], so both
/// operands of `LEAST` are plain `TIMESTAMP`s in one UTC frame.
///
/// ## Determinism
///
/// On any repository whose newest commit predates the current instant — every
/// healthy repository — `MAX(<col>)` is the smaller operand, so `LEAST` returns
/// it unchanged: the output is byte-identical to the un-clamped form and does
/// not depend on when the query runs, even though the embedded literal does.
/// Only a repository that actually carries a future-dated commit becomes
/// wall-clock dependent, and there the anchor tracks the real instant of
/// computation; a persisted fact-store cache pins whatever anchor the first
/// run observed until the cache is rebuilt. That wall-clock dependence is the
/// pathological state being defended against, not a regression of the healthy
/// path.
///
/// ## Scope
///
/// This clamps only the *data-controlled* anchor idiom (a bare `MAX(<col>)`
/// standing in for "now"). The `code-age` / `knowledge-islands` family instead
/// anchors on a wall-clock instant (or `--age-time-now`) and filters
/// `<col> <= anchor`, so it never trusts a future date in the first place and
/// is left untouched. Unifying the two idioms is a separate design question,
/// not part of making the data-controlled anchor safe.
///
/// `col` is a fixed internal column name, never user input.
#[must_use]
pub fn clamped_now_anchor(col: &str) -> String {
    format!(
        "LEAST(MAX({col}), TIMESTAMP '{}')",
        wall_clock_utc_literal()
    )
}

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
