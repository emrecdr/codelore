//! `top-committers` analysis — per-author commit leaderboard.
//!
//! ## What this signal tells you
//!
//! For each canonical author (post-mailmap), this analysis emits a
//! global summary of their contribution: total commits, total lines
//! added/deleted, and the first and last commit dates. It's the answer
//! to "who's the biggest contributor by raw commit count" — useful for
//! release-notes generation, contributor-recognition snapshots,
//! velocity dashboards, and onboarding (who should I ask about this
//! codebase as a whole?).
//!
//! ## Relationship to the `authors` analysis
//!
//! This is a DIFFERENT analysis than `authors`. `authors` answers a
//! per-file question ("how many distinct contributors touched this
//! module?", Bird et al. 2011 defect-risk indicator). `top-committers`
//! answers a global per-author question ("who has the most commits
//! repo-wide?", contributor-leaderboard signal).
//!
//! In code-maat, the per-file question is `-a authors`. There's no
//! built-in per-author leaderboard — users approximate it by running
//! `-a author-churn` and sorting. `CodeLore` exposes both as first-class
//! named analyses so the operator picks the question, not the
//! workaround.
//!
//! ## Columns (richer than the approximate code-maat workaround)
//!
//! - `author` — canonical email (post-mailmap, post-bot-filter).
//! - `commits` — distinct commits attributed to this author.
//! - `loc_added` — total `LoC` added across all commits.
//! - `loc_deleted` — total `LoC` deleted across all commits.
//! - `first_commit` — calendar date of this author's earliest commit.
//! - `last_commit` — calendar date of this author's latest commit.
//! - `is_bot` — flag from `author_aliases.is_bot`. Surfaces
//!   Dependabot / renovate / similar bots so dashboards can suppress
//!   or visually distinguish them without joining a second table.
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "top-committers".

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TopCommittersRow {
    pub author: String,
    pub commits: u32,
    pub loc_added: u64,
    pub loc_deleted: u64,
    pub first_commit: String,
    pub last_commit: String,
    pub is_bot: bool,
}

const SQL_TEMPLATE: &str = "
    WITH {human_aliases},
    per_author AS (
        SELECT
            commits.canonical_author AS author,
            COUNT(DISTINCT commits.rev) AS commits,
            COALESCE(SUM(c.loc_added), 0)::BIGINT AS loc_added,
            COALESCE(SUM(c.loc_deleted), 0)::BIGINT AS loc_deleted,
            MIN(commits.date) AS first_at,
            MAX(commits.date) AS last_at
        FROM commits
        LEFT JOIN changes c ON c.rev = commits.rev
        GROUP BY commits.canonical_author
    )
    SELECT
        pa.author,
        CAST(pa.commits AS UINTEGER) AS commits,
        pa.loc_added,
        pa.loc_deleted,
        CAST(CAST(pa.first_at AS DATE) AS TEXT) AS first_commit,
        CAST(CAST(pa.last_at AS DATE) AS TEXT) AS last_commit,
        (aa.canonical IS NULL) AS is_bot
    FROM per_author pa
    -- Pair-granular: a canonical is bot ONLY when it has NO human alias at
    -- all (author_aliases is keyed on (raw_name, raw_email); a canonical
    -- with at least one human alias stays eligible through that alias, even
    -- if it ALSO owns a bot-classified alias). DISTINCT dedupes the lookup
    -- to one row per canonical so the join can't multiply per_author rows.
    LEFT JOIN (SELECT DISTINCT canonical FROM human_aliases) aa ON aa.canonical = pa.author
    ORDER BY commits DESC, author ASC
    LIMIT ?
";

#[tracing::instrument(name = "top-committers", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_top_committers(db: &FactsDb, opts: &Options) -> Result<Vec<TopCommittersRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let sql = SQL_TEMPLATE.replace("{human_aliases}", super::query::HUMAN_ALIASES_CTE);
    super::query::explain_if_requested(db, &sql, params![row_limit], "top-committers", opts)?;
    super::query::query_map_collect(db, &sql, params![row_limit], "top-committers", |r| {
        Ok(TopCommittersRow {
            author: r.get::<_, String>(0)?,
            commits: r.get::<_, u32>(1)?,
            loc_added: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(u64::MAX),
            loc_deleted: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(u64::MAX),
            first_commit: r.get::<_, String>(4)?,
            last_commit: r.get::<_, String>(5)?,
            is_bot: r.get::<_, bool>(6)?,
        })
    })
}
