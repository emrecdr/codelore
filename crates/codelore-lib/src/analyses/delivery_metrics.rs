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
//!   path. Computed via hunk-pair overlap (approximate — line drift between
//!   commits is not tracked). Bounded self-join: both sides are filtered to
//!   the rework window before the cross-join.
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
/// `lead_proxy_hours`; all hunk pairs within the window for `rework_pct`
/// which produces a single aggregate row with `n` = hunks compared).
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

/// Run the `delivery-metrics` analysis against the already-ingested fact store.
///
/// Requires the `commit_parents` table (schema v4) and that merges were
/// ingested with `opts.include_merges = true`; without merge rows the branch
/// metrics will be empty.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any `DuckDB` error.
// Long function: the body is one flat sequence of 5 independent SQL blocks,
// each building one metric row. Splitting into helpers would obscure the
// parallel structure without reducing complexity.
#[allow(clippy::too_many_lines)]
#[tracing::instrument(name = "delivery-metrics", skip_all)]
pub fn run_delivery_metrics(db: &FactsDb, opts: &Options) -> Result<Vec<DeliveryMetricsRow>> {
    // ── Squash detection ─────────────────────────────────────────────────────
    // Merges = commits that have a commit_parents row at position 1.
    let squash_sql = "
        SELECT
            (SELECT COUNT(DISTINCT rev) FROM commit_parents WHERE position = 1) AS merge_count,
            (SELECT COUNT(*) FROM commits WHERE is_merge = FALSE)               AS commit_count
    ";
    let (merge_count, commit_count): (u64, u64) = {
        let mut stmt = db
            .conn()
            .prepare(squash_sql)
            .map_err(|e| CodeLoreError::Analysis(format!("prepare squash probe: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| CodeLoreError::Analysis(format!("query squash probe: {e}")))?;
        let row = rows
            .next()
            .map_err(|e| CodeLoreError::Analysis(format!("fetch squash probe row: {e}")))?
            .ok_or_else(|| CodeLoreError::Analysis("squash probe returned no rows".to_string()))?;
        let mc: u64 = row
            .get(0)
            .map_err(|e| CodeLoreError::Analysis(format!("get merge_count: {e}")))?;
        let cc: u64 = row
            .get(1)
            .map_err(|e| CodeLoreError::Analysis(format!("get commit_count: {e}")))?;
        (mc, cc)
    };
    if merge_count < 3 && commit_count > 50 {
        tracing::warn!(
            merge_count,
            commit_count,
            "few merges found; squash/rebase workflow likely — branch metrics unreliable"
        );
    }

    let mut result: Vec<DeliveryMetricsRow> = Vec::new();

    // ── Branch-side commit set ────────────────────────────────────────────────
    // Branch-side commits for a merge rev M:
    //   - reachable from M's parent(position=1) (the branch tip)
    //   - NOT reachable from M's parent(position=0) (the mainline parent)
    //   - within 90 days of the merge date (date floor) to bound recursion
    //   - at most 200 hops deep
    //
    // The CTE produces (merge_rev, merge_date, branch_commit_rev) for
    // every branch-side commit of every merge in the repo.
    let branch_commits_cte = "
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
        branch_walk AS (
            -- Seed: the branch tip itself for each merge.
            SELECT
                m.merge_rev,
                m.merge_date,
                m.mainline_parent,
                m.branch_tip           AS branch_rev,
                0                      AS depth
            FROM merges m

            UNION ALL

            -- Recursive step: walk each branch commit's first parent
            -- (position=0), bounded by depth and date floor.
            SELECT
                bw.merge_rev,
                bw.merge_date,
                bw.mainline_parent,
                cp.parent_rev          AS branch_rev,
                bw.depth + 1           AS depth
            FROM branch_walk bw
            JOIN commit_parents cp ON cp.rev = bw.branch_rev AND cp.position = 0
            JOIN commits co ON co.rev = cp.parent_rev
            WHERE bw.depth < 200
              AND co.date >= bw.merge_date - INTERVAL '90' DAY
              -- Stop if we reach the mainline parent (do not cross into main).
              AND cp.parent_rev <> bw.mainline_parent
        ),
        branch_commits AS (
            -- Exclude the mainline parent itself; keep only commits that are
            -- purely on the branch side (not reachable from mainline_parent).
            SELECT DISTINCT
                bw.merge_rev,
                bw.merge_date,
                bw.branch_rev          AS branch_commit_rev
            FROM branch_walk bw
            WHERE bw.branch_rev <> bw.mainline_parent
        )
    ";

    // ── 1. batch_size_files ───────────────────────────────────────────────────
    let batch_files_sql = format!(
        "{branch_commits_cte}
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
    if let Some(row) = query_percentiles(db, &batch_files_sql, "batch_size_files")? {
        result.push(DeliveryMetricsRow {
            metric: "batch_size_files".to_string(),
            p50: row.0,
            p75: row.1,
            p90: row.2,
            n: row.3,
            caveat: "merge-topology based; squash workflows undercount".to_string(),
        });
    }

    // ── 2. batch_size_loc ────────────────────────────────────────────────────
    let batch_loc_sql = format!(
        "{branch_commits_cte}
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
    if let Some(row) = query_percentiles(db, &batch_loc_sql, "batch_size_loc")? {
        result.push(DeliveryMetricsRow {
            metric: "batch_size_loc".to_string(),
            p50: row.0,
            p75: row.1,
            p90: row.2,
            n: row.3,
            caveat: "merge-topology based; squash workflows undercount".to_string(),
        });
    }

    // ── 3. branch_duration_hours ─────────────────────────────────────────────
    let branch_dur_sql = format!(
        "{branch_commits_cte}
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
    if let Some(row) = query_percentiles(db, &branch_dur_sql, "branch_duration_hours")? {
        result.push(DeliveryMetricsRow {
            metric: "branch_duration_hours".to_string(),
            p50: row.0,
            p75: row.1,
            p90: row.2,
            n: row.3,
            caveat: "merge-topology based; squash workflows undercount".to_string(),
        });
    }

    // ── 4. rework_pct ────────────────────────────────────────────────────────
    // Self-join hunks on the same path where date2 − date1 ≤ rework_window_days.
    // Date filter applied BEFORE the cross-join to bound cost.
    // Overlap = GREATEST(0, LEAST(h1.new_start+h1.new_lines, h2.old_start+h2.old_lines)
    //                      - GREATEST(h1.new_start, h2.old_start))
    let rework_window = opts.rework_window_days;
    let rework_sql = format!(
        "
        WITH windowed_hunks AS (
            -- Pull all hunks with their commit author-date into one CTE;
            -- filtering here before the self-join bounds the cross-product.
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
        ),
        rework_pairs AS (
            SELECT
                GREATEST(0,
                    LEAST(h1.new_start + h1.new_lines, h2.old_start + h2.old_lines)
                    - GREATEST(h1.new_start, h2.old_start)
                ) AS overlap,
                h1.new_lines AS loc_added_h1
            FROM windowed_hunks h1
            JOIN windowed_hunks h2
              ON h2.path = h1.path
             AND h2.rev  <> h1.rev
             AND date_diff('day', h1.commit_date, h2.commit_date) > 0
             AND date_diff('day', h1.commit_date, h2.commit_date) <= {rework_window}
        ),
        totals AS (
            SELECT
                SUM(overlap)      AS total_overlap,
                SUM(loc_added_h1) AS total_added,
                COUNT(*)          AS pair_count
            FROM rework_pairs
        )
        SELECT
            CASE WHEN total_added > 0
                 THEN 100.0 * total_overlap / total_added
                 ELSE 0.0 END AS rework_pct,
            pair_count
        FROM totals
        "
    );
    {
        let mut stmt = db
            .conn()
            .prepare(&rework_sql)
            .map_err(|e| CodeLoreError::Analysis(format!("prepare rework_pct: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| CodeLoreError::Analysis(format!("query rework_pct: {e}")))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| CodeLoreError::Analysis(format!("fetch rework_pct row: {e}")))?
        {
            let pct: f64 = row
                .get(0)
                .map_err(|e| CodeLoreError::Analysis(format!("get rework_pct value: {e}")))?;
            let n: u64 = row
                .get(1)
                .map_err(|e| CodeLoreError::Analysis(format!("get rework_pct n: {e}")))?;
            // rework_pct is a single aggregate, so p50=p75=p90=pct.
            result.push(DeliveryMetricsRow {
                metric: "rework_pct".to_string(),
                p50: pct,
                p75: pct,
                p90: pct,
                n,
                caveat: "approximate — line drift between commits not tracked".to_string(),
            });
        }
    }

    // ── 5. lead_proxy_hours ──────────────────────────────────────────────────
    // Reuses the semantics of `analyses/lead_time.rs`:
    //   commits.date          = author date (when authored)
    //   commits.committer_date = when it entered mainline
    // Positive values only (negative = clock skew / timezone artefact).
    // Merge commits excluded — their lead-time is architecturally zero.
    let lead_sql = "
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
    if let Some(row) = query_percentiles(db, lead_sql, "lead_proxy_hours")? {
        result.push(DeliveryMetricsRow {
            metric: "lead_proxy_hours".to_string(),
            p50: row.0,
            p75: row.1,
            p90: row.2,
            n: row.3,
            caveat: "author→committer date gap; proxy only — does not include waiting time before first review".to_string(),
        });
    }

    Ok(result)
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
