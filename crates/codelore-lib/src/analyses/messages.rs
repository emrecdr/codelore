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
//!
//! Research basis: see `docs/research-foundations.md` entry "messages"
//! (Mockus & Votta, ICSM 2000 — identifying reasons for software
//! changes from commit messages; Hindle et al., MSR 2011 — automated
//! topic naming).

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

#[tracing::instrument(name = "messages", skip_all, fields(min_revs = opts.min_revs))]
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
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![expr, row_limit],
        "messages",
        opts,
    )?;
    crate::analyses::query::query_map_collect(db, &sql, params![expr, row_limit], "messages", |r| {
        Ok(MessagesRow {
            entity: r.get::<_, String>(0)?,
            matches: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
        })
    })
}
