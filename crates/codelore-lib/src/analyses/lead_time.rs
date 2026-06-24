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
/// ## `commits.date` vs `commits.committer_date` semantics
///
/// `commits.date` is the **author** date (when the commit was first
/// authored — `git commit`'s `%aI`); `commits.committer_date` is when
/// it last entered mainline (`%cI`). On linear-history workflows
/// (rebase, squash) these are often equal — the squash commit is born
/// at merge. On merge-via-merge-commit + review workflows the delta is
/// the true in-flight review time.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "lead-time", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_lead_time(db: &FactsDb, opts: &Options) -> Result<Vec<LeadTimeRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    // `date` is the author date; `committer_date` is when the commit
    // last entered mainline. Their delta in seconds is the lead time.
    // `EPOCH` extraction returns DOUBLE; we cast to BIGINT for the
    // typed-row binding.
    let sql = "
        SELECT
            rev,
            canonical_author,
            CAST(CAST(date AS TIMESTAMP) AS TEXT) AS author_date,
            CAST(CAST(committer_date AS TIMESTAMP) AS TEXT) AS committer_date,
            CAST(
                EXTRACT(EPOCH FROM committer_date) - EXTRACT(EPOCH FROM date)
                AS BIGINT
            ) AS lead_time_seconds
        FROM commits
        WHERE is_merge = FALSE
          AND date IS NOT NULL
          AND committer_date IS NOT NULL
        ORDER BY lead_time_seconds DESC, rev ASC
        LIMIT ?
    ";

    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare lead-time: {e}")))?;
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
