//! Authors analysis — code-maat parity. One row per canonical author,
//! sorted by commit count desc. Equivalent to code-maat's `-a authors`.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthorsRow {
    /// Canonical author (post-mailmap). Stable across alias changes.
    pub author: String,
    /// Total commits attributed to this canonical author.
    pub commits: u32,
}

pub fn run_authors(db: &FactsDb, opts: &Options) -> Result<Vec<AuthorsRow>> {
    // ORDER BY 2 DESC for highest-commit-count first;
    // ORDER BY 1 ASC tie-break so the test output is deterministic.
    let limit_clause = match opts.rows_limit {
        Some(n) => format!(" LIMIT {n}"),
        None => String::new(),
    };
    let sql = format!(
        "SELECT canonical_author, CAST(COUNT(*) AS UINTEGER) AS commits
         FROM commits
         GROUP BY canonical_author
         ORDER BY commits DESC, canonical_author ASC{limit_clause}"
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare authors: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AuthorsRow {
                author: r.get::<_, String>(0)?,
                commits: r.get::<_, u32>(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query authors: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect authors: {e}")))
}
