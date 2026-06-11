//! Churn analyses per spec §1.1:
//! - abs-churn: by date (added/deleted/commits)
//! - author-churn: by `canonical_author` (added/deleted/commits)
//! - entity-churn: by path (added/deleted/commits)
//!
//! Research basis: see `docs/research-foundations.md` entry "churn"
//! (Nagappan & Ball, ICSE 2005 — relative code churn predicts system
//! defect density; foundational input to nearly every downstream
//! behavioural signal).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

fn source_table(opts: &Options) -> &'static str {
    if opts.use_canonical_lineage {
        "changes_lineage"
    } else {
        "changes"
    }
}

fn materialize(db: &FactsDb, opts: &Options) -> Result<()> {
    if opts.use_canonical_lineage {
        crate::facts::ingest::materialize_changes_lineage(db)?;
    }
    Ok(())
}

fn build_abs_churn_sql(src: &str) -> String {
    // `commits.date` is `TIMESTAMP` in schema v2 — explicitly truncate to
    // `DATE` for the per-day aggregation, otherwise two commits one second
    // apart would each get their own bucket and the output stops being a
    // daily trend.
    format!(
        "SELECT
            CAST(CAST(commits.date AS DATE) AS TEXT) AS date,
            COALESCE(SUM(c.loc_added), 0) AS added,
            COALESCE(SUM(c.loc_deleted), 0) AS deleted,
            COUNT(DISTINCT commits.rev) AS commits
        FROM commits
        INNER JOIN {src} c ON c.rev = commits.rev
        GROUP BY CAST(commits.date AS DATE)
        ORDER BY CAST(commits.date AS DATE) ASC, added DESC, deleted DESC
        LIMIT ?"
    )
}

fn build_author_churn_sql(src: &str) -> String {
    format!(
        "SELECT
            commits.canonical_author AS author,
            COALESCE(SUM(c.loc_added), 0) AS added,
            COALESCE(SUM(c.loc_deleted), 0) AS deleted,
            COUNT(DISTINCT commits.rev) AS commits
        FROM commits
        INNER JOIN {src} c ON c.rev = commits.rev
        GROUP BY commits.canonical_author
        ORDER BY added DESC, commits DESC, author ASC
        LIMIT ?"
    )
}

// F16 fix: entity-churn now filters to files currently live at HEAD.
// Without this, a file deleted years ago still shows up with full
// historical churn numbers — useful for retroactive forensics but
// confusing for triage dashboards which are the dominant use case.
//
// The `live_paths` CTE uses the same `arg_max(change_type, ROW(date, -rowid))`
// hash-aggregation as `query_live_paths` — selects paths whose most-recent
// change is not `'deleted'` in a single streaming pass (O(K) memory, K =
// distinct paths). The ROW(date, -rowid) ordering reproduces the original
// `ORDER BY date DESC, rowid ASC` tiebreak via DuckDB struct lex-compare.
//
// Unlike code_age, entity-churn has no anchor parameter — it's a
// "current state" report — so the CTE filter is "live at HEAD now."
// Users who want historical (deleted-included) views can pair
// `entity-churn` with `--no-canonical-lineage` semantically; the
// modern default surfaces only live files.
fn build_entity_churn_sql(src: &str) -> String {
    format!(
        "WITH live_paths AS (
            SELECT path FROM (
                SELECT c.path,
                       arg_max(
                           c.change_type,
                           ROW(commits.date, -commits.rowid)
                       ) AS change_type
                FROM {src} c
                INNER JOIN commits ON commits.rev = c.rev
                GROUP BY c.path
            ) WHERE change_type != 'deleted'
        )
        SELECT
            c.path,
            COALESCE(SUM(c.loc_added), 0) AS added,
            COALESCE(SUM(c.loc_deleted), 0) AS deleted,
            -- (rev, path) is the changes PK so rev is unique within each
            -- `GROUP BY c.path` group. Plain COUNT skips DuckDB's
            -- distinct-tracking overhead.
            COUNT(c.rev) AS commits
        FROM {src} c
        INNER JOIN live_paths USING (path)
        GROUP BY c.path
        HAVING commits >= ?
        ORDER BY added DESC, commits DESC, path ASC
        LIMIT ?"
    )
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AbsChurnRow {
    pub date: String,
    pub added: i64,
    pub deleted: i64,
    pub commits: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthorChurnRow {
    pub author: String,
    pub added: i64,
    pub deleted: i64,
    pub commits: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EntityChurnRow {
    pub path: String,
    pub added: i64,
    pub deleted: i64,
    pub commits: u32,
}

pub fn run_abs_churn(db: &FactsDb, opts: &Options) -> Result<Vec<AbsChurnRow>> {
    materialize(db, opts)?;
    let sql = build_abs_churn_sql(source_table(opts));
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(db, &sql, params![row_limit], "abs-churn", opts)?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare abs-churn: {e}")))?;
    let rows = stmt
        .query_map(params![row_limit], |r| {
            Ok(AbsChurnRow {
                date: r.get::<_, String>(0)?,
                added: r.get::<_, i64>(1)?,
                deleted: r.get::<_, i64>(2)?,
                commits: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query abs-churn: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect abs-churn: {e}")))
}

pub fn run_author_churn(db: &FactsDb, opts: &Options) -> Result<Vec<AuthorChurnRow>> {
    materialize(db, opts)?;
    let sql = build_author_churn_sql(source_table(opts));
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![row_limit],
        "author-churn",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare author-churn: {e}")))?;
    let rows = stmt
        .query_map(params![row_limit], |r| {
            Ok(AuthorChurnRow {
                author: r.get::<_, String>(0)?,
                added: r.get::<_, i64>(1)?,
                deleted: r.get::<_, i64>(2)?,
                commits: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query author-churn: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect author-churn: {e}")))
}

pub fn run_entity_churn(db: &FactsDb, opts: &Options) -> Result<Vec<EntityChurnRow>> {
    materialize(db, opts)?;
    let sql = build_entity_churn_sql(source_table(opts));
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "entity-churn",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare entity-churn: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(EntityChurnRow {
                path: r.get::<_, String>(0)?,
                added: r.get::<_, i64>(1)?,
                deleted: r.get::<_, i64>(2)?,
                commits: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query entity-churn: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect entity-churn: {e}")))
}
