//! `delivery-metrics` analysis — repo-level delivery flow distributions.
//!
//! Computes five percentile distributions that describe how code moves from
//! branch to mainline, sliced as percentile-first (p50/p75/p90) summaries
//! (DORA 2025 convention). The analysis requires the `commit_parents` table
//! (schema v4) which records parent revisions and their position; merge
//! commits are identified as those with a `position=1` row.
//!
//! ## Metrics
//!
//! - **`batch_size_files`** — distinct paths touched by the branch-side
//!   commits bundled into each merge unit. A proxy for PR size.
//! - **`batch_size_loc`** — total `loc_added + loc_deleted` across the
//!   branch-side commits. Complements the file count with raw churn size.
//! - **`branch_duration_hours`** — wall-clock hours between the earliest
//!   branch-side commit's author-date and the merge commit's date.
//!   Measures how long branches stay open before integration.
//! - **`rework_pct`** — percentage of added lines that were subsequently
//!   deleted or overwritten within `opts.rework_window_days` on the same
//!   path. Window is anchored to `MAX(date)` across all commits so only
//!   recent activity is examined. Computed via hunk-pair overlap (approximate
//!   — line drift between commits is not tracked). Both sides of the self-join
//!   are pre-filtered to the window (bounds the cross-product); the denominator
//!   is the total lines added in the window, independent of the pair join.
//! - **`lead_proxy_hours`** — per-commit `date_diff('hour', author_date,
//!   committer_date)`, positive values only, over non-merge commits. Uses
//!   the same author/committer semantics as [`crate::analyses::lead_time`]:
//!   `commits.date` is the author date; `commits.committer_date` is when the
//!   commit entered mainline. On squash-merge workflows the delta is small;
//!   on merge-via-merge-commit + review workflows it reflects the in-review
//!   time.
//!
//! ## Squash detection
//!
//! If the number of merge commits is fewer than 3 AND the total number of
//! non-merge commits exceeds 50, the repo is likely using a squash or rebase
//! workflow. The analysis emits a `tracing::warn!` and still produces rows
//! with `n` as-is (branch metrics will be noisy in that case).
//!
//! ## Relationship to existing analyses
//!
//! - [`crate::analyses::lead_time`] — per-commit row; this analysis
//!   summarises the same signal as percentile distributions.
//! - [`crate::analyses::delivery_friction`] — per-FILE composite metric
//!   (churn × lead-time × cognitive). Complementary, not overlapping: this
//!   analysis is repo-level distributions, not per-file scores.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// One row in the `delivery-metrics` output.
///
/// Each row describes one metric as a percentile distribution across all
/// observed units (merge units for batch/branch metrics; commits for
/// `lead_proxy_hours`; for `rework_pct` a single aggregate row where `n` is
/// the number of hunk pairs examined — not the number of changed lines).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeliveryMetricsRow {
    /// Metric name: one of `batch_size_files`, `batch_size_loc`,
    /// `branch_duration_hours`, `rework_pct`, `lead_proxy_hours`.
    pub metric: String,
    /// 50th-percentile value (median).
    pub p50: f64,
    /// 75th-percentile value.
    pub p75: f64,
    /// 90th-percentile value.
    pub p90: f64,
    /// Sample count (merge units, commits, or hunk-pairs depending on metric).
    pub n: u64,
    /// Fixed explanatory caveat string for this metric.
    pub caveat: String,
}

// Branch-side commit CTE.
//
// Branch-side commits for a merge rev M:
//   - reachable from M's parent(position=1) (the branch tip)
//   - NOT reachable from M's parent(position=0) (the mainline parent)
//   - within 90 days of the merge date (date floor) to bound recursion
//   - at most 200 hops deep
//
// Produces (merge_rev, merge_date, branch_commit_rev) for every branch-side
// commit of every merge in the repo.
// lead_proxy_hours: author-date → committer-date gap, positive values only.
// Negative values indicate clock-skew or timezone artefacts and are excluded.
// Merge commits are excluded — their lead-time is architecturally zero.
const LEAD_PROXY_SQL: &str = "
    SELECT
        PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY lt_hours) AS p50,
        PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY lt_hours) AS p75,
        PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY lt_hours) AS p90,
        COUNT(*)                                               AS n
    FROM (
        SELECT
            date_diff('hour', date, committer_date) AS lt_hours
        FROM commits
        WHERE is_merge = FALSE
          AND date IS NOT NULL
          AND committer_date IS NOT NULL
          AND date_diff('hour', date, committer_date) > 0
    ) sub
";

const BRANCH_COMMITS_CTE: &str = "
        WITH RECURSIVE merges AS (
            -- All merge revs: those with a position=1 parent row.
            SELECT DISTINCT
                cp1.rev                              AS merge_rev,
                co.date                              AS merge_date,
                cp1.parent_rev                       AS branch_tip,
                cp0.parent_rev                       AS mainline_parent
            FROM commit_parents cp1
            JOIN commit_parents cp0 ON cp0.rev = cp1.rev AND cp0.position = 0
            JOIN commits co ON co.rev = cp1.rev
            WHERE cp1.position = 1
        ),
        mainline_reachable AS (
            -- First-parent walk from each merge's mainline (position=0) parent.
            -- This spans the true merge base and every shared commit below it,
            -- so anti-joining it out of the branch walk removes the mainline
            -- history the branch walk would otherwise cross into. Bounded by the
            -- same depth and 90-day date floor as the branch walk.
            SELECT
                m.merge_rev,
                m.merge_date,
                m.mainline_parent      AS mainline_rev,
                0                      AS depth
            FROM merges m

            UNION ALL

            SELECT
                mr.merge_rev,
                mr.merge_date,
                cp.parent_rev          AS mainline_rev,
                mr.depth + 1           AS depth
            FROM mainline_reachable mr
            JOIN commit_parents cp ON cp.rev = mr.mainline_rev AND cp.position = 0
            JOIN commits co ON co.rev = cp.parent_rev
            WHERE mr.depth < 200
              AND co.date >= mr.merge_date - INTERVAL '90' DAY
        ),
        branch_walk AS (
            -- Seed: the branch tip itself for each merge.
            SELECT
                m.merge_rev,
                m.merge_date,
                m.branch_tip           AS branch_rev,
                0                      AS depth
            FROM merges m

            UNION ALL

            -- Recursive step: walk each branch commit's first parent
            -- (position=0), bounded by depth and date floor. The walk runs past
            -- the merge base into mainline history on purpose; the
            -- mainline_reachable anti-join in branch_commits trims it back to the
            -- commits unique to the branch. (Stopping at mainline_parent is
            -- wrong once mainline advances after the branch is cut, because then
            -- mainline_parent is no longer on the branch tip's first-parent
            -- chain — the source of the previous overshoot bug.)
            SELECT
                bw.merge_rev,
                bw.merge_date,
                cp.parent_rev          AS branch_rev,
                bw.depth + 1           AS depth
            FROM branch_walk bw
            JOIN commit_parents cp ON cp.rev = bw.branch_rev AND cp.position = 0
            JOIN commits co ON co.rev = cp.parent_rev
            WHERE bw.depth < 200
              AND co.date >= bw.merge_date - INTERVAL '90' DAY
        ),
        branch_commits AS (
            -- Branch-side commits: reachable from the branch tip AND NOT
            -- reachable from the mainline parent. The anti-join drops the merge
            -- base and all shared mainline history, leaving only commits unique
            -- to the branch below the merge base.
            SELECT DISTINCT
                bw.merge_rev,
                bw.merge_date,
                bw.branch_rev          AS branch_commit_rev
            FROM branch_walk bw
            WHERE NOT EXISTS (
                SELECT 1
                FROM mainline_reachable mr
                WHERE mr.merge_rev = bw.merge_rev
                  AND mr.mainline_rev = bw.branch_rev
            )
        )
    ";

/// Run the `delivery-metrics` analysis against the already-ingested fact store.
///
/// Requires the `commit_parents` table (schema v4) and that merges were
/// ingested with `opts.include_merges = true`; without merge rows the branch
/// metrics will be empty.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any `DuckDB` error.
#[tracing::instrument(name = "delivery-metrics", skip_all)]
pub fn run_delivery_metrics(db: &FactsDb, opts: &Options) -> Result<Vec<DeliveryMetricsRow>> {
    check_squash_workflow(db)?;
    let mut result: Vec<DeliveryMetricsRow> = Vec::new();
    let merge_caveat = "merge-topology based; squash workflows undercount";

    let batch_files_sql = format!(
        "{BRANCH_COMMITS_CTE}
        SELECT
            PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY files) AS p50,
            PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY files) AS p75,
            PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY files) AS p90,
            COUNT(*)                                             AS n
        FROM (
            SELECT bc.merge_rev, COUNT(DISTINCT ch.path) AS files
            FROM branch_commits bc
            JOIN changes ch ON ch.rev = bc.branch_commit_rev
            GROUP BY bc.merge_rev
        ) sub
        "
    );
    push_metric(
        &mut result,
        db,
        &batch_files_sql,
        "batch_size_files",
        merge_caveat,
    )?;

    let batch_loc_sql = format!(
        "{BRANCH_COMMITS_CTE}
        SELECT
            PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY loc) AS p50,
            PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY loc) AS p75,
            PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY loc) AS p90,
            COUNT(*)                                           AS n
        FROM (
            SELECT bc.merge_rev, SUM(ch.loc_added + ch.loc_deleted) AS loc
            FROM branch_commits bc
            JOIN changes ch ON ch.rev = bc.branch_commit_rev
            GROUP BY bc.merge_rev
        ) sub
        "
    );
    push_metric(
        &mut result,
        db,
        &batch_loc_sql,
        "batch_size_loc",
        merge_caveat,
    )?;

    let branch_dur_sql = format!(
        "{BRANCH_COMMITS_CTE}
        SELECT
            PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY dur) AS p50,
            PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY dur) AS p75,
            PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY dur) AS p90,
            COUNT(*)                                           AS n
        FROM (
            SELECT
                bc.merge_rev,
                date_diff('hour',
                    MIN(co.date),
                    bc.merge_date
                ) AS dur
            FROM branch_commits bc
            JOIN commits co ON co.rev = bc.branch_commit_rev
            GROUP BY bc.merge_rev, bc.merge_date
            HAVING dur >= 0
        ) sub
        "
    );
    push_metric(
        &mut result,
        db,
        &branch_dur_sql,
        "branch_duration_hours",
        merge_caveat,
    )?;

    if let Some(row) = compute_rework_pct(db, opts.rework_window_days)? {
        result.push(row);
    }

    push_metric(
        &mut result,
        db,
        LEAD_PROXY_SQL,
        "lead_proxy_hours",
        "author→committer date gap; proxy only — does not include waiting time before first review",
    )?;

    Ok(result)
}

/// Query `sql` for percentile rows and, if results exist, push a
/// [`DeliveryMetricsRow`] with the given `metric` label and `caveat` onto
/// `result`. No-op when the query returns no rows (empty sub-population).
fn push_metric(
    result: &mut Vec<DeliveryMetricsRow>,
    db: &FactsDb,
    sql: &str,
    metric: &str,
    caveat: &str,
) -> Result<()> {
    if let Some(row) = query_percentiles(db, sql, metric)? {
        result.push(DeliveryMetricsRow {
            metric: metric.to_string(),
            p50: row.0,
            p75: row.1,
            p90: row.2,
            n: row.3,
            caveat: caveat.to_string(),
        });
    }
    Ok(())
}

/// Emit a warning when the repo looks like it uses squash/rebase merging,
/// which means `commit_parents` has few position=1 rows and branch metrics
/// will be empty or misleading.
fn check_squash_workflow(db: &FactsDb) -> Result<()> {
    let sql = "
        SELECT
            (SELECT COUNT(DISTINCT rev) FROM commit_parents WHERE position = 1) AS merge_count,
            (SELECT COUNT(*) FROM commits WHERE is_merge = FALSE)               AS commit_count
    ";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare squash probe: {e}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| CodeLoreError::Analysis(format!("query squash probe: {e}")))?;
    let row = rows
        .next()
        .map_err(|e| CodeLoreError::Analysis(format!("fetch squash probe row: {e}")))?
        .ok_or_else(|| CodeLoreError::Analysis("squash probe returned no rows".to_string()))?;
    let merge_count: u64 = row
        .get(0)
        .map_err(|e| CodeLoreError::Analysis(format!("get merge_count: {e}")))?;
    let commit_count: u64 = row
        .get(1)
        .map_err(|e| CodeLoreError::Analysis(format!("get commit_count: {e}")))?;
    if merge_count < 3 && commit_count > 50 {
        tracing::warn!(
            merge_count,
            commit_count,
            "few merges found; squash/rebase workflow likely — branch metrics unreliable"
        );
    }
    Ok(())
}

/// Compute the `rework_pct` metric.
///
/// Identifies lines added in one commit that were subsequently deleted or
/// overwritten by a later commit on the same path within `rework_window_days`
/// of the first commit, AND within the trailing `rework_window_days` window
/// anchored to HEAD (`MAX(date)` across all commits).
///
/// # Formula
///
/// ```text
/// rework_pct = 100 × Σ_h1 LEAST( Σ_h2 overlap(h1, h2), new_lines(h1) )
///                    / Σ new_lines(all windowed hunks)
/// ```
///
/// Each added hunk `h1` can be reworked by several later hunks `h2`. Summing
/// the raw per-pair overlaps would count `h1`'s added lines once per reworking
/// partner, so a single 10-line region overwritten by 3 later commits would
/// contribute 30 lines to the numerator against only 10 in the denominator —
/// pushing the ratio above 100%. The inner `LEAST(…, new_lines(h1))` caps each
/// added region's contribution at the lines it actually added, so the numerator
/// can never exceed the denominator and `rework_pct` is bounded to `[0, 100]`.
///
/// The denominator is the total lines added across ALL hunks in the window
/// (not just hunk pairs that happen to have a rework partner); a denominator
/// built only from `h1` sides of matched pairs would inflate the percentage by
/// excluding unpaired added lines.
///
/// Returns `None` when no hunks fall inside the window (empty history or
/// all commits predate the window floor).
#[allow(clippy::too_many_lines)]
fn compute_rework_pct(db: &FactsDb, rework_window_days: u32) -> Result<Option<DeliveryMetricsRow>> {
    // windowed_hunks: hunks whose commit author-date falls within the trailing
    // rework_window_days of the repo's most recent commit. Both sides of the
    // self-join are drawn from this CTE, so the pre-filter genuinely bounds
    // the cross-product (not just the join predicate).
    let now_anchor = crate::analyses::query::clamped_now_anchor("date");
    let sql = format!(
        "
        WITH windowed_hunks AS (
            SELECT
                h.path,
                h.rev,
                h.new_start,
                h.new_lines,
                h.old_start,
                h.old_lines,
                co.date AS commit_date
            FROM hunks h
            JOIN commits co ON co.rev = h.rev
            WHERE co.date IS NOT NULL
              AND co.date >= (SELECT {now_anchor} FROM commits)
                             - INTERVAL '{rework_window_days}' DAY
        ),
        rework_pairs AS (
            -- h1 = the 'added' hunk, h2 = the later hunk that may overwrite it.
            -- Overlap formula: lines of h1's added range that h2 touches.
            -- Carries h1's identity + new_lines so the numerator can cap each
            -- added hunk's total overlap by the lines it actually added.
            SELECT
                h1.path      AS h1_path,
                h1.rev       AS h1_rev,
                h1.old_start AS h1_old_start,
                h1.new_start AS h1_new_start,
                h1.new_lines AS h1_new_lines,
                GREATEST(0,
                    LEAST(h1.new_start + h1.new_lines, h2.old_start + h2.old_lines)
                    - GREATEST(h1.new_start, h2.old_start)
                ) AS overlap
            FROM windowed_hunks h1
            JOIN windowed_hunks h2
              ON h2.path = h1.path
             AND h2.rev  <> h1.rev
             AND date_diff('day', h1.commit_date, h2.commit_date) > 0
             AND date_diff('day', h1.commit_date, h2.commit_date) <= {rework_window_days}
        ),
        capped_rework AS (
            -- Cap each added hunk's total forward overlap by its own new_lines.
            -- A single added region can be overwritten by several later hunks;
            -- summing the raw per-pair overlaps would count its lines once per
            -- reworking partner, letting the numerator exceed the added-line
            -- volume (rework_pct > 100%). The per-hunk LEAST() bounds each
            -- added region's contribution to at most the lines it added, so
            -- SUM(capped_overlap) <= SUM(new_lines) and rework_pct <= 100.
            SELECT LEAST(SUM(overlap), h1_new_lines) AS capped_overlap
            FROM rework_pairs
            GROUP BY h1_path, h1_rev, h1_old_start, h1_new_start, h1_new_lines
        ),
        window_added AS (
            -- Total lines added in the window — the denominator. Independent of
            -- the pair join so unpaired added lines are included; combined with
            -- the per-added-hunk cap above this keeps rework_pct in [0, 100].
            SELECT SUM(new_lines) AS total_added FROM windowed_hunks
        )
        SELECT
            CASE WHEN (SELECT total_added FROM window_added) > 0
                 THEN 100.0 * COALESCE((SELECT SUM(capped_overlap) FROM capped_rework), 0)
                              / (SELECT total_added FROM window_added)
                 ELSE 0.0 END                    AS rework_pct,
            (SELECT COUNT(*)    FROM rework_pairs) AS pair_count,
            (SELECT total_added FROM window_added) AS total_added
        "
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare rework_pct: {e}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| CodeLoreError::Analysis(format!("query rework_pct: {e}")))?;
    let Some(row) = rows
        .next()
        .map_err(|e| CodeLoreError::Analysis(format!("fetch rework_pct row: {e}")))?
    else {
        return Ok(None);
    };
    let pct: f64 = row
        .get(0)
        .map_err(|e| CodeLoreError::Analysis(format!("get rework_pct value: {e}")))?;
    let n: u64 = row
        .get(1)
        .map_err(|e| CodeLoreError::Analysis(format!("get rework_pct n: {e}")))?;
    let total_added: Option<u64> = row
        .get(2)
        .map_err(|e| CodeLoreError::Analysis(format!("get rework_pct total_added: {e}")))?;
    // Return None when the window contains no hunks at all (empty history or
    // all commits predate the window floor). A window with hunks but no rework
    // pairs is still valid and emits pct=0.0.
    if total_added.unwrap_or(0) == 0 {
        return Ok(None);
    }
    Ok(Some(DeliveryMetricsRow {
        metric: "rework_pct".to_string(),
        // rework_pct is a single aggregate; p50=p75=p90=pct.
        p50: pct,
        p75: pct,
        p90: pct,
        n,
        caveat: "approximate — line drift between commits not tracked; window-anchored to HEAD"
            .to_string(),
    }))
}

/// Execute a single-row percentile query and return `(p50, p75, p90, n)`.
/// Returns `None` when no rows matched (n=0 or NULL percentiles).
fn query_percentiles(db: &FactsDb, sql: &str, label: &str) -> Result<Option<(f64, f64, f64, u64)>> {
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare {label}: {e}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| CodeLoreError::Analysis(format!("query {label}: {e}")))?;
    let row = rows
        .next()
        .map_err(|e| CodeLoreError::Analysis(format!("fetch {label} row: {e}")))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let p50: Option<f64> = row
        .get(0)
        .map_err(|e| CodeLoreError::Analysis(format!("get p50 for {label}: {e}")))?;
    let p75: Option<f64> = row
        .get(1)
        .map_err(|e| CodeLoreError::Analysis(format!("get p75 for {label}: {e}")))?;
    let p90: Option<f64> = row
        .get(2)
        .map_err(|e| CodeLoreError::Analysis(format!("get p90 for {label}: {e}")))?;
    let n: u64 = row
        .get(3)
        .map_err(|e| CodeLoreError::Analysis(format!("get n for {label}: {e}")))?;
    // If all percentiles are NULL the subquery was empty.
    match (p50, p75, p90) {
        (Some(p50), Some(p75), Some(p90)) if n > 0 => Ok(Some((p50, p75, p90, n))),
        _ => Ok(None),
    }
}
