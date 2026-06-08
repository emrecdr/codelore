//! `messages` analysis — code-maat parity. Match commit messages against
//! a user-supplied regex; count one row per (file, matching-commit). High
//! match counts surface files repeatedly touched by commits matching the
//! regex — useful for "where do my bug-fix commits land?" or "which files
//! get the most refactor mentions?".
//!
//! ## Activation
//!
//! Requires `--expression-to-match REGEX` (the `Options.message_regex`
//! field). Calling `run_messages` without a regex returns an error.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessagesRow {
    pub entity: String,
    pub matches: u32,
}

const SQL: &str = "
    SELECT c.path AS entity, COUNT(*)::INTEGER AS matches
    FROM changes c
    JOIN commits m ON m.rev = c.rev
    WHERE regexp_matches(m.message, ?)
    GROUP BY c.path
    ORDER BY matches DESC, entity ASC
    LIMIT ?
";

pub fn run_messages(db: &FactsDb, opts: &Options) -> Result<Vec<MessagesRow>> {
    let expr = opts.message_regex.as_deref().ok_or_else(|| {
        CodeLoreError::Analysis(
            "messages analysis requires --expression-to-match REGEX (commit-message filter)".into(),
        )
    })?;

    // Validate the regex eagerly so we fail fast with a clear error before
    // SQL prepare. DuckDB's regexp_matches uses RE2 flavor — close enough
    // to Rust regex that valid Rust regexes work in DuckDB (modulo
    // backreferences and lookaround which neither supports anyway).
    let _ = regex::Regex::new(expr).map_err(|e| {
        CodeLoreError::Analysis(format!("invalid --expression-to-match regex {expr:?}: {e}"))
    })?;

    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare messages: {e}")))?;
    let rows = stmt
        .query_map(params![expr, row_limit], |r| {
            Ok(MessagesRow {
                entity: r.get::<_, String>(0)?,
                matches: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query messages: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect messages: {e}")))
}
