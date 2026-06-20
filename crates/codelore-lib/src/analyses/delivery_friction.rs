//! `delivery-friction` analysis — where technical debt actively slows
//! delivery.
//!
//! Combines three signals at the file level:
//!
//! - **Churn** (revisions): files touched more often are higher-risk
//!   per the standard hotspot model.
//! - **Lead time** (median committer-date minus author-date in days):
//!   files where individual commits sit longer between author and
//!   merge — proxy for "review bottleneck" or "PR thrash". Needs the
//!   schema v3 `commits.committer_date` column populated; pre-v3
//!   this analysis returns zero across the board.
//! - **Complexity** (max cognitive): files where the per-function
//!   gnarly-ness is highest are slower to change correctly.
//!
//! The composite `friction_score = pr(revs) × pr(lead_time) × pr(cog)
//! × 100` is in `[0, 100]`. A file scoring high on ALL THREE
//! percentile ranks lights up; one dominant signal alone does not.
//! This is the deliberate contrast with `hotspots` (revs × complexity)
//! and `code-health` (complexity-led composite) — `delivery-friction`
//! is the analysis that answers "where is technical debt actually
//! slowing us down right now?".
//!
//! `wip_age_days` is reported alongside (days since last commit) so
//! callers can tell whether a high-friction file is also stale-but-
//! still-touched (worst case) or hot-but-recently-active (manageable).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

/// One row per file with above-threshold revision count.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeliveryFrictionRow {
    pub path: String,
    pub revisions: u32,
    /// `MAX(cognitive)` across all entities of the file. `0.0` when
    /// `codelore-rca` doesn't support the file's language.
    pub cognitive: f64,
    /// Median `committer_date - date` across the file's commits, in
    /// days. Zero on rebase-only workflows where author and committer
    /// timestamps coincide for every commit.
    pub median_lead_time_days: f64,
    /// 95th-percentile lead time in days. Surfaces the right tail —
    /// the "one commit took two weeks" worst case.
    pub p95_lead_time_days: f64,
    /// Days since the file's last commit. `MAX(committer_date)` minus
    /// the analysis-time anchor. Files high on both friction and
    /// `wip_age_days` are the stale-but-still-touched worst case.
    pub wip_age_days: f64,
    /// Composite friction score in `[0, 100]`. Product of three
    /// percentile ranks (revisions × lead time × cognitive) scaled by
    /// 100; high requires elevation on ALL THREE axes.
    pub friction_score: f64,
}

const SQL: &str = "
    WITH file_lead_times AS (
        SELECT
            ch.path,
            COUNT(c.rev) AS revisions,
            COALESCE(
                MEDIAN(EXTRACT(EPOCH FROM c.committer_date)
                       - EXTRACT(EPOCH FROM c.date)),
                0.0
            ) / 86400.0 AS median_lead_time_days,
            COALESCE(
                QUANTILE_CONT(
                    EXTRACT(EPOCH FROM c.committer_date)
                    - EXTRACT(EPOCH FROM c.date),
                    0.95
                ),
                0.0
            ) / 86400.0 AS p95_lead_time_days,
            MAX(c.committer_date) AS last_touched
        FROM changes ch
        INNER JOIN commits c ON c.rev = ch.rev
        WHERE c.is_merge = FALSE
          AND c.date IS NOT NULL
          AND c.committer_date IS NOT NULL
        GROUP BY ch.path
        HAVING revisions >= ?
    ),
    file_complexity AS (
        SELECT path, MAX(cognitive)::DOUBLE AS cognitive
        FROM complexity_metrics
        WHERE cognitive IS NOT NULL
        GROUP BY path
    ),
    joined AS (
        SELECT
            flt.path,
            flt.revisions,
            flt.median_lead_time_days,
            flt.p95_lead_time_days,
            COALESCE(fc.cognitive, 0.0) AS cognitive,
            EXTRACT(EPOCH FROM (CAST(? AS TIMESTAMP) - flt.last_touched))
                / 86400.0 AS wip_age_days
        FROM file_lead_times flt
        LEFT JOIN file_complexity fc ON fc.path = flt.path
    ),
    ranked AS (
        SELECT
            path,
            revisions,
            cognitive,
            median_lead_time_days,
            p95_lead_time_days,
            wip_age_days,
            PERCENT_RANK() OVER (ORDER BY revisions) AS pr_rev,
            PERCENT_RANK() OVER (ORDER BY median_lead_time_days) AS pr_lt,
            PERCENT_RANK() OVER (ORDER BY cognitive) AS pr_cx
        FROM joined
    )
    SELECT
        path,
        revisions,
        cognitive,
        median_lead_time_days,
        p95_lead_time_days,
        wip_age_days,
        pr_rev * pr_lt * pr_cx * 100.0 AS friction_score
    FROM ranked
    ORDER BY friction_score DESC, path ASC
    LIMIT ?
";

/// Run the `delivery-friction` analysis. Returns rows ranked by
/// composite friction score (highest first).
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` prepare / query /
/// collect errors.
pub fn run_delivery_friction(db: &FactsDb, opts: &Options) -> Result<Vec<DeliveryFrictionRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let n = time::OffsetDateTime::now_utc();
    let anchor = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        n.year(),
        u8::from(n.month()),
        n.day(),
        n.hour(),
        n.minute(),
        n.second(),
    );
    super::query::explain_if_requested(
        db,
        SQL,
        params![opts.min_revs, anchor, row_limit],
        "delivery-friction",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        SQL,
        params![opts.min_revs, anchor, row_limit],
        "delivery-friction",
        |r| {
            Ok(DeliveryFrictionRow {
                path: r.get::<_, String>(0)?,
                revisions: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(2)?,
                median_lead_time_days: r.get::<_, f64>(3)?,
                p95_lead_time_days: r.get::<_, f64>(4)?,
                wip_age_days: r.get::<_, f64>(5)?,
                friction_score: r.get::<_, f64>(6)?,
            })
        },
    )
}
