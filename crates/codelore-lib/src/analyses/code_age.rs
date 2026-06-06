//! Code age analysis per spec §1.1: entity → months since last modification.
//!
//! Reference date is `opts.age_time_now` if set, else today (UTC).

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeAgeRow {
    pub path: String,
    pub age_months: i32,
}

pub fn run_code_age(db: &FactsDb, opts: &Options) -> Result<Vec<CodeAgeRow>> {
    let limit = opts
        .rows_limit
        .map(|n| format!(" LIMIT {n}"))
        .unwrap_or_default();

    // Reference "now" for age calculation
    let age_time_now = opts
        .age_time_now
        .unwrap_or_else(|| time::OffsetDateTime::now_utc().date());
    let now_str = format!(
        "{:04}-{:02}-{:02}",
        age_time_now.year(),
        u8::from(age_time_now.month()),
        age_time_now.day()
    );

    let sql = format!(
        "SELECT
             changes.path,
             DATE_DIFF('month', MAX(commits.date), DATE '{now}') AS age_months
         FROM changes
         INNER JOIN commits ON changes.rev = commits.rev
         GROUP BY changes.path
         HAVING COUNT(DISTINCT changes.rev) >= {min}
         ORDER BY age_months ASC, changes.path ASC{limit}",
        now = now_str,
        min = opts.min_revs,
        limit = limit,
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-age: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CodeAgeRow {
                path: r.get::<_, String>(0)?,
                age_months: i32::try_from(r.get::<_, i64>(1)?).unwrap_or(i32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query code-age: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-age: {e}")))
}
