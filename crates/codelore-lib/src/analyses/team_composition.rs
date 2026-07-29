//! Team-composition analysis — contribution-span buckets with a behavioral
//! veteran gate and onboarding-velocity metric.
//!
//! ## Tenure buckets
//!
//! Authors are bucketed by their **contribution span** (date of last commit
//! minus date of first commit):
//!
//! | Bucket | Span |
//! |---|---|
//! | `onboarded` | < 90 days |
//! | `experienced` | 90 – 364 days |
//! | `veteran` | ≥ 365 days |
//!
//! These boundaries are contiguous — common industry buckets (e.g. those
//! discussed in the Brooks-law contributor-lifecycle literature: 0–3 months /
//! 6–12 months / 12+ months) leave a 3–6 month gap where authors belong to no
//! tier. Our definition closes that gap by making `experienced` start
//! immediately at 90 days.
//!
//! ## Veteran breadth gate
//!
//! A `veteran` author who has not touched a breadth of files comparable to
//! other core contributors is capped at `experienced`. The gate checks whether
//! the author's distinct lineage-paths touched is ≥ the median across the
//! current **core set** (smallest group reaching 80% of cumulative commits,
//! computed once at anchor). If `veteran_breadth_ok = false` the bucket is
//! silently capped; the field is always reported so callers can surface it.
//!
//! ## Onboarding velocity
//!
//! `onboarding_weeks` is the number of weeks from an author's first commit to
//! the first week in which they enter the weekly 80%-core set. Authors whose
//! first commit falls within the project's first 12 weeks (the founder period,
//! per arXiv 2601.23142) receive `NULL` — their "onboarding" is definitionally
//! the project itself. Authors who never reach the core set also receive `NULL`.
//!
//! ## Summary row
//!
//! A synthetic row with `author = "__summary__"` carries bucket-percentage
//! columns (`onboarded_pct`, `experienced_pct`, `veteran_pct` packed into the
//! `bucket` field as a JSON-like string) for downstream factor-tile and SPA
//! consumption. The `author` field is `"__summary__"` so callers can filter
//! it out; the `bucket` field contains the percentage string.

use crate::analyses::lineage;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Founder-period cutoff in weeks: authors whose first commit falls within
/// the first 12 weeks of the project get `onboarding_weeks = NULL`.
/// Per arXiv 2601.23142 §3.2 (contributor-lifecycle convention).
const FOUNDER_WEEKS: i64 = 12;

/// Per-author team-composition row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeamCompositionRow {
    /// Author canonical name/email, or `"__summary__"` for the summary row.
    pub author: String,
    /// Contribution span in days (`last_commit − first_commit`). 0 for
    /// single-commit authors. `0` for the summary row.
    pub tenure_days: i64,
    /// `"onboarded"` (< 90 d), `"experienced"` (90–364 d), `"veteran"`
    /// (≥ 365 d). For the summary row (where `author == "__summary__"`),
    /// this field holds the percentage string (e.g.
    /// `"onboarded=66.7% experienced=33.3% veteran=0.0%"`).
    pub bucket: String,
    /// `true` when the author's distinct lineage-path count ≥ median of the
    /// current core set. Always `false` for the summary row. A `veteran`
    /// author with `veteran_breadth_ok = false` is capped at `"experienced"`.
    pub veteran_breadth_ok: bool,
    /// `true` when the author has at least one commit in the trailing
    /// `window_days` window.
    pub active: bool,
    /// Total commit count for this author (non-merge only, same as `changes`
    /// table source).
    pub commits: u64,
    /// Distinct lineage-paths touched across all history.
    pub files_touched: u64,
    /// Weeks from first commit to entering the weekly 80%-core set. `None`
    /// for founders (first commit within first 12 weeks of the project) or
    /// authors who never reach the core.
    pub onboarding_weeks: Option<i64>,
}

/// Main SQL for all per-author metrics plus the breadth-based veteran gate.
///
/// Produces one row per distinct non-bot `canonical_author` in `commits`,
/// sorted by `tenure_days DESC, commits DESC, author ASC`. Bot authors
/// (`author_aliases.is_bot`) are excluded so they never inflate tenure
/// buckets, `total_commits`, the 80%-core set, or `core_median_paths`.
const SQL_AUTHOR_METRICS: &str = "
WITH
anchor AS (
    SELECT MAX(date) AS max_d, MIN(date) AS min_d FROM commits
),
-- One row per canonical author (a canonical may own several name+email
-- identities in author_aliases; a direct JOIN would multiply commit counts
-- by that alias count). Bot canonicals are dropped here so they never enter tenure
-- buckets, the core set, or the __summary__ percentages.
canon_authors AS (
    SELECT canonical FROM author_aliases
    GROUP BY canonical
    HAVING NOT BOOL_OR(is_bot)
),
-- Per-author first/last commit and total commit count. Bots excluded via
-- the canon_authors join.
author_stats AS (
    SELECT
        c.canonical_author                   AS author,
        MIN(c.date)                          AS first_commit,
        MAX(c.date)                          AS last_commit,
        COUNT(*)                             AS commits
    FROM commits c
    JOIN canon_authors ca ON ca.canonical = c.canonical_author
    GROUP BY c.canonical_author
),
-- Active = any commit within trailing window.
active_authors AS (
    SELECT DISTINCT canonical_author
    FROM commits, anchor
    WHERE date >= anchor.max_d - INTERVAL '{wd} days'
),
-- Distinct lineage paths per author (rename-aware via source table).
-- {src} is changes (or changes_lineage); join to commits for canonical_author.
author_paths AS (
    SELECT
        co.canonical_author AS author,
        COUNT(DISTINCT ch.path) AS paths_touched
    FROM {src} ch
    JOIN commits co ON co.rev = ch.rev
    GROUP BY co.canonical_author
),
-- Core set at anchor: smallest author group reaching 80% of cumulative commits.
-- Ranked descending by total commits; running cumulative share.
ranked_authors AS (
    SELECT
        author,
        commits,
        SUM(commits) OVER (ORDER BY commits DESC, author ASC
                           ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
            AS cum_commits,
        SUM(commits) OVER () AS total_commits
    FROM author_stats
),
core_set AS (
    SELECT ra.author, ap.paths_touched
    FROM ranked_authors ra
    JOIN author_paths ap ON ap.author = ra.author
    WHERE ra.cum_commits - ra.commits < ra.total_commits * 0.8
),
core_median_paths AS (
    SELECT MEDIAN(paths_touched) AS med FROM core_set
),
-- Raw tenure and bucket (before breadth gate).
author_tenure AS (
    SELECT
        s.author,
        CAST((EXTRACT(EPOCH FROM s.last_commit) - EXTRACT(EPOCH FROM s.first_commit)) / 86400 AS BIGINT)
            AS tenure_days,
        s.first_commit,
        s.last_commit,
        s.commits,
        COALESCE(ap.paths_touched, 0) AS files_touched,
        (a.canonical_author IS NOT NULL) AS active,
        CASE
            WHEN (EXTRACT(EPOCH FROM s.last_commit) - EXTRACT(EPOCH FROM s.first_commit)) / 86400 >= 365
                THEN 'veteran_raw'
            WHEN (EXTRACT(EPOCH FROM s.last_commit) - EXTRACT(EPOCH FROM s.first_commit)) / 86400 >= 90
                THEN 'experienced'
            ELSE 'onboarded'
        END AS bucket_raw,
        COALESCE(ap.paths_touched, 0) >= COALESCE((SELECT med FROM core_median_paths), 0)
            AS veteran_breadth_ok
    FROM author_stats s
    LEFT JOIN active_authors a ON a.canonical_author = s.author
    LEFT JOIN author_paths   ap ON ap.author = s.author
),
-- Apply breadth gate to cap veteran → experienced when breadth_ok = false.
author_bucketed AS (
    SELECT
        author,
        tenure_days,
        first_commit,
        CASE
            WHEN bucket_raw = 'veteran_raw' AND NOT veteran_breadth_ok THEN 'experienced'
            WHEN bucket_raw = 'veteran_raw'                             THEN 'veteran'
            ELSE bucket_raw
        END AS bucket,
        veteran_breadth_ok,
        active,
        commits,
        files_touched
    FROM author_tenure
)
SELECT
    ab.author,
    ab.tenure_days,
    ab.bucket,
    ab.veteran_breadth_ok,
    ab.active,
    ab.commits,
    ab.files_touched
FROM author_bucketed ab
ORDER BY ab.tenure_days DESC, ab.commits DESC, ab.author ASC
";

/// Onboarding-velocity SQL.
///
/// For each author whose first commit is AFTER the project's first 12 weeks,
/// finds the first calendar week in which they enter the weekly 80%-core set.
/// Returns `(author, onboarding_weeks)`.
///
/// Algorithm per arXiv 2601.23142:
/// 1. Compute per-author cumulative commit counts up to each week.
/// 2. Per week, rank authors by their cumulative count; compute running sum.
/// 3. An author is "in core" for a given week when their cumulative total is
///    ≤ 80% of the week's grand cumulative total (Pareto 80% coverage).
/// 4. Take each author's earliest such week; subtract their first-commit week.
///
/// Bot authors (`author_aliases.is_bot`) are excluded from every CTE that
/// feeds the weekly grand total, so bot commits never shift the 80%-core
/// threshold real authors are measured against.
const SQL_ONBOARDING: &str = "
WITH
anchor AS (
    SELECT MIN(date) AS project_start FROM commits
),
-- Project start week (truncated to Monday).
project_week AS (
    SELECT date_trunc('week', project_start) AS pw FROM anchor
),
-- Founder cutoff: first commit must be AFTER project_start + 12 weeks.
founder_cutoff AS (
    SELECT pw + INTERVAL '{fw} weeks' AS cutoff FROM project_week
),
-- Bot canonicals excluded (see SQL_AUTHOR_METRICS::canon_authors): a bot's
-- commits must not inflate the weekly grand_total used to compute the
-- 80%-core threshold, or they'd shift real authors' onboarding_weeks.
canon_authors AS (
    SELECT canonical FROM author_aliases
    GROUP BY canonical
    HAVING NOT BOOL_OR(is_bot)
),
-- Per-author weekly commit counts. Bots excluded via the canon_authors join.
weekly_counts AS (
    SELECT
        c.canonical_author            AS author,
        date_trunc('week', c.date)    AS week,
        COUNT(*)                      AS week_commits
    FROM commits c
    JOIN canon_authors ca ON ca.canonical = c.canonical_author
    GROUP BY c.canonical_author, date_trunc('week', c.date)
),
-- Cumulative commits per author up to (and including) each week they
-- appear in. We need ALL weeks the project has commits, not just weeks
-- the author was active, to correctly track when they cross 80%.
all_weeks AS (
    SELECT DISTINCT date_trunc('week', date) AS week FROM commits
),
-- Cross-join authors × all_weeks, fill in 0 for weeks with no activity.
author_week_grid AS (
    SELECT
        a.author,
        w.week,
        COALESCE(wc.week_commits, 0) AS week_commits
    FROM (SELECT DISTINCT author FROM weekly_counts) a
    CROSS JOIN all_weeks w
    LEFT JOIN weekly_counts wc ON wc.author = a.author AND wc.week = w.week
),
-- Cumulative commits per author up to each week.
author_cumulative AS (
    SELECT
        author,
        week,
        SUM(week_commits) OVER (PARTITION BY author ORDER BY week
                                ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
            AS cum_commits
    FROM author_week_grid
),
-- Per-week grand cumulative total (sum across ALL authors).
week_totals AS (
    SELECT week, SUM(cum_commits) AS grand_total
    FROM author_cumulative
    GROUP BY week
),
-- Per week, rank authors by cum_commits DESC; running sum of cum_commits
-- partitioned by week.
week_ranked AS (
    SELECT
        ac.author,
        ac.week,
        ac.cum_commits,
        wt.grand_total,
        SUM(ac.cum_commits) OVER (
            PARTITION BY ac.week
            ORDER BY ac.cum_commits DESC, ac.author ASC
            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
        ) AS running_core_total
    FROM author_cumulative ac
    JOIN week_totals wt ON wt.week = ac.week
    WHERE ac.cum_commits > 0
),
-- An author is in the core for a week when their cumulative total is part of
-- the 80% mass (running_core_total - cum_commits < 80% of grand_total,
-- i.e. they are needed to reach 80%).
core_membership AS (
    SELECT
        author,
        week,
        (running_core_total - cum_commits) < grand_total * 0.8 AS in_core
    FROM week_ranked
),
-- First commit per author. Bots excluded via the canon_authors join.
author_first AS (
    SELECT c.canonical_author AS author, MIN(c.date) AS first_commit
    FROM commits c
    JOIN canon_authors ca ON ca.canonical = c.canonical_author
    GROUP BY c.canonical_author
),
-- First week each author enters core (NULL if never).
first_core_week AS (
    SELECT
        cm.author,
        MIN(cm.week) AS entry_week
    FROM core_membership cm
    WHERE cm.in_core = TRUE
    GROUP BY cm.author
),
-- Combine: compute onboarding_weeks only for non-founders.
result AS (
    SELECT
        af.author,
        date_trunc('week', af.first_commit)   AS first_week,
        fc.entry_week,
        CAST(
            (EXTRACT(EPOCH FROM fc.entry_week) - EXTRACT(EPOCH FROM date_trunc('week', af.first_commit)))
            / 604800 AS BIGINT
        ) AS onboarding_weeks,
        af.first_commit >= (SELECT cutoff FROM founder_cutoff) AS non_founder
    FROM author_first af
    LEFT JOIN first_core_week fc ON fc.author = af.author
)
SELECT
    author,
    CASE WHEN non_founder AND entry_week IS NOT NULL
         THEN onboarding_weeks
         ELSE NULL
    END AS onboarding_weeks
FROM result
ORDER BY author
";

/// Run the team-composition analysis.
///
/// Returns one row per author plus a `__summary__` row. Rows are sorted by
/// `tenure_days DESC, commits DESC, author ASC`; the summary row is appended
/// last.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on SQL or row-mapping failures.
pub fn run_team_composition(db: &FactsDb, opts: &Options) -> Result<Vec<TeamCompositionRow>> {
    lineage::materialize_if_needed(db, opts)?;

    let wd = opts.window_days;
    let src = lineage::source_table(opts);

    let author_sql = SQL_AUTHOR_METRICS
        .replace("{wd}", &wd.to_string())
        .replace("{src}", src);

    let onboarding_sql = SQL_ONBOARDING.replace("{fw}", &FOUNDER_WEEKS.to_string());

    let conn = db.conn();

    // --- Author metrics ---
    let mut stmt = conn
        .prepare(&author_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare team-composition metrics: {e}")))?;

    let mut rows: Vec<TeamCompositionRow> = stmt
        .query_map([], |r| {
            Ok(TeamCompositionRow {
                author: r.get(0)?,
                tenure_days: r.get(1)?,
                bucket: r.get(2)?,
                veteran_breadth_ok: r.get(3)?,
                active: r.get(4)?,
                commits: r.get::<_, u64>(5)?,
                files_touched: r.get::<_, u64>(6)?,
                onboarding_weeks: None,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query team-composition metrics: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect team-composition rows: {e}")))?;

    // --- Onboarding velocity ---
    let mut ob_stmt = conn.prepare(&onboarding_sql).map_err(|e| {
        CodeLoreError::Analysis(format!("prepare team-composition onboarding: {e}"))
    })?;

    let onboarding_map: std::collections::HashMap<String, Option<i64>> = ob_stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query onboarding: {e}")))?
        .collect::<std::result::Result<std::collections::HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect onboarding: {e}")))?;

    for row in &mut rows {
        if let Some(ob) = onboarding_map.get(&row.author) {
            row.onboarding_weeks = *ob;
        }
    }

    // --- Summary row ---
    let total = rows.len();
    if total > 0 {
        #[allow(clippy::cast_precision_loss)]
        let n = total as f64;
        let onboarded = rows.iter().filter(|r| r.bucket == "onboarded").count();
        let experienced = rows.iter().filter(|r| r.bucket == "experienced").count();
        let veteran = rows.iter().filter(|r| r.bucket == "veteran").count();
        #[allow(clippy::cast_precision_loss)]
        let bucket = format!(
            "onboarded={:.1}% experienced={:.1}% veteran={:.1}%",
            onboarded as f64 / n * 100.0,
            experienced as f64 / n * 100.0,
            veteran as f64 / n * 100.0,
        );
        rows.push(TeamCompositionRow {
            author: "__summary__".into(),
            tenure_days: 0,
            bucket,
            veteran_breadth_ok: false,
            active: false,
            commits: 0,
            files_touched: 0,
            onboarding_weeks: None,
        });
    }

    Ok(rows)
}
