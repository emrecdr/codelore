//! `lead-time` analysis.
//!
//! Computes the time elapsed between a commit's author-date (when
//! the change was authored) and committer-date (when it was merged
//! to mainline). Captures the "how long does code sit in review
//! before shipping?" question — a foundational DORA metric.
//!
//! ## Why per-commit, not per-PR?
//!
//! `CodeLore` is git-only by design — no GitHub PR metadata in scope
//! (see [`project_git_only_scope`](../../.devt/memory/lessons/project_git_only_scope.md)).
//! Git's commit object carries BOTH author-date and committer-date;
//! their delta is the in-flight time. On merge-via-squash workflows
//! this delta is small (the squash commit is born at merge); on
//! merge-via-rebase or merge-via-merge-commit workflows it's the
//! true review-time. Either way it's a defensible proxy.
//!
//! ## Output
//!
//! One row per commit, ordered by lead-time DESC. Useful for
//! identifying stragglers + computing org-wide percentiles.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LeadTimeRow {
    pub rev: String,
    pub canonical_author: String,
    pub author_date: String,
    pub committer_date: String,
    /// Seconds between author-date and committer-date. Negative
    /// values can occur when the timestamps disagree (clock skew or
    /// timezone artefacts); they're clamped to 0 in `lead_time_days`
    /// but preserved here for forensic inspection.
    pub lead_time_seconds: i64,
    /// Convenience field: `max(0, lead_time_seconds) / 86400.0`.
    pub lead_time_days: f64,
}

/// Run the `lead-time` analysis. Returns commits ordered by
/// lead-time DESC. Merge commits are excluded — their lead-time is
/// architecturally zero on most workflows.
///
/// ## Note on `commits.date` semantics
///
/// `CodeLore`'s `commits.date` column carries the **committer** date
/// (committer email's `date` field). The current schema doesn't
/// preserve the separate `author_date`, so this analysis emits
/// `0` lead-time across the board today; once an `author_date`
/// column is added to the commits table, the query becomes
/// `DATE_DIFF('second', author_date, date)` and the analysis
/// returns real values.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` errors.
pub fn run_lead_time(db: &FactsDb, opts: &Options) -> Result<Vec<LeadTimeRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    // Current schema carries only `commits.date` (committer date).
    // Without a separate `author_date` column we emit zero-lead-time
    // rows; the analysis surface exists so downstream analyses can
    // build against it. Once `author_date` is added the query
    // becomes `DATE_DIFF('second', author_date, date)`.
    let sql = "
        SELECT
            rev,
            canonical_author,
            CAST(CAST(date AS TIMESTAMP) AS TEXT) AS author_date,
            CAST(CAST(date AS TIMESTAMP) AS TEXT) AS committer_date,
            CAST(0 AS BIGINT) AS lead_time_seconds
        FROM commits
        WHERE is_merge = FALSE
          AND date IS NOT NULL
        ORDER BY date DESC
        LIMIT ?
    ";

    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare lead-time: {e}")))?;
    tracing::warn!(
        "lead-time: schema carries only committer date; all rows report 0-second lead time until a future schema bump adds author_date. Use `codelore explain lead-time` for the planned semantic."
    );
    let rows = stmt
        .query_map(params![row_limit], |r| {
            let lead_time_seconds: i64 = r.get(4)?;
            #[allow(clippy::cast_precision_loss)]
            let lead_time_days = (lead_time_seconds.max(0) as f64) / 86_400.0;
            Ok(LeadTimeRow {
                rev: r.get(0)?,
                canonical_author: r.get(1)?,
                author_date: r.get(2)?,
                committer_date: r.get(3)?,
                lead_time_seconds,
                lead_time_days,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query lead-time: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect lead-time: {e}")))
}
