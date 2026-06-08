//! Churn analyses per spec §1.1:
//! - abs-churn: by date (added/deleted/commits)
//! - author-churn: by `canonical_author` (added/deleted/commits)
//! - entity-churn: by path (added/deleted/commits)

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

const ABS_CHURN_SQL: &str = "
    SELECT
        CAST(commits.date AS TEXT) AS date,
        COALESCE(SUM(changes.loc_added), 0) AS added,
        COALESCE(SUM(changes.loc_deleted), 0) AS deleted,
        COUNT(DISTINCT commits.rev) AS commits
    FROM commits
    INNER JOIN changes ON changes.rev = commits.rev
    GROUP BY commits.date
    ORDER BY commits.date ASC, added DESC, deleted DESC
    LIMIT ?
";

const AUTHOR_CHURN_SQL: &str = "
    SELECT
        commits.canonical_author AS author,
        COALESCE(SUM(changes.loc_added), 0) AS added,
        COALESCE(SUM(changes.loc_deleted), 0) AS deleted,
        COUNT(DISTINCT commits.rev) AS commits
    FROM commits
    INNER JOIN changes ON changes.rev = commits.rev
    GROUP BY commits.canonical_author
    ORDER BY added DESC, commits DESC, author ASC
    LIMIT ?
";

const ENTITY_CHURN_SQL: &str = "
    SELECT
        changes.path,
        COALESCE(SUM(changes.loc_added), 0) AS added,
        COALESCE(SUM(changes.loc_deleted), 0) AS deleted,
        COUNT(DISTINCT changes.rev) AS commits
    FROM changes
    GROUP BY changes.path
    HAVING commits >= ?
    ORDER BY added DESC, commits DESC, path ASC
    LIMIT ?
";

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
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let mut stmt = db
        .conn()
        .prepare(ABS_CHURN_SQL)
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
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let mut stmt = db
        .conn()
        .prepare(AUTHOR_CHURN_SQL)
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
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let mut stmt = db
        .conn()
        .prepare(ENTITY_CHURN_SQL)
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
