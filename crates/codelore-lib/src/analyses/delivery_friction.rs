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
//!   this analysis returns zero across the board. Commits where
//!   `committer_date <= date` (clock-skew or timezone artefacts) are
//!   excluded from the lead-time statistics — matching
//!   `delivery_metrics`'s documented exclusion.
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
    /// timestamps coincide for every commit. Commits where
    /// `committer_date <= date` (clock-skew or timezone artefacts) are
    /// excluded from this statistic — they do not shrink `revisions`,
    /// only the lead-time aggregates.
    pub median_lead_time_days: f64,
    /// 95th-percentile lead time in days. Surfaces the right tail —
    /// the "one commit took two weeks" worst case. Same negative/zero
    /// exclusion as `median_lead_time_days`.
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
        -- Per-file aggregation. The inline subquery computes the per-
        -- commit lead-time ONCE so MEDIAN and QUANTILE_CONT both
        -- aggregate over the same precomputed `lead_secs` column —
        -- avoids the two EXTRACT(EPOCH) calls per row the prior shape
        -- carried. `lead_secs` is NULL (not row-excluded) when
        -- `committer_date <= date`: MEDIAN/QUANTILE_CONT skip NULLs per
        -- standard SQL aggregate semantics, so clock-skew/rebase commits
        -- drop out of the lead-time stats without shrinking `revisions`
        -- or `last_touched` — those must stay the true per-file commit
        -- count/last-touch regardless of any one commit's lead-time sign.
        SELECT
            path,
            COUNT(rev) AS revisions,
            COALESCE(MEDIAN(lead_secs), 0.0) / 86400.0 AS median_lead_time_days,
            COALESCE(QUANTILE_CONT(lead_secs, 0.95), 0.0) / 86400.0 AS p95_lead_time_days,
            MAX(committer_date) AS last_touched
        FROM (
            SELECT
                ch.path,
                c.rev,
                c.committer_date,
                CASE WHEN c.committer_date > c.date
                     THEN EXTRACT(EPOCH FROM c.committer_date)
                          - EXTRACT(EPOCH FROM c.date)
                END AS lead_secs
            FROM changes ch
            INNER JOIN commits c ON c.rev = ch.rev
            WHERE c.is_merge = FALSE
              AND c.date IS NOT NULL
              AND c.committer_date IS NOT NULL
        )
        GROUP BY path
        HAVING revisions >= ?
    ),
    file_complexity AS (
        SELECT path, MAX(cognitive)::DOUBLE AS cognitive
        FROM {cm_src}
        WHERE cognitive IS NOT NULL
        GROUP BY path
    ),
    -- The previous shape had a pass-through `joined` CTE feeding a
    -- `ranked` CTE with the window functions. Collapse: compute the
    -- LEFT JOIN, wip_age_days, and the three PERCENT_RANK windows in
    -- one CTE — one less SQL hop for the planner to materialise.
    ranked AS (
        SELECT
            flt.path,
            flt.revisions,
            flt.median_lead_time_days,
            flt.p95_lead_time_days,
            COALESCE(fc.cognitive, 0.0) AS cognitive,
            EXTRACT(EPOCH FROM (CAST(? AS TIMESTAMP) - flt.last_touched))
                / 86400.0 AS wip_age_days,
            PERCENT_RANK() OVER (ORDER BY flt.revisions) AS pr_rev,
            PERCENT_RANK() OVER (ORDER BY flt.median_lead_time_days) AS pr_lt,
            PERCENT_RANK() OVER (ORDER BY COALESCE(fc.cognitive, 0.0)) AS pr_cx
        FROM file_lead_times flt
        LEFT JOIN file_complexity fc ON fc.path = flt.path
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
#[tracing::instrument(name = "delivery-friction", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_delivery_friction(db: &FactsDb, opts: &Options) -> Result<Vec<DeliveryFrictionRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    // `wip_age_days` anchor. `--age-time-now` (end-of-day of the given
    // calendar date) when set — matching `code-age` / `knowledge-islands`
    // — so the back-test pattern works; otherwise the newest committer
    // date in the store. The default is the max committer date (NOT the
    // wall clock) so `wip_age_days` is deterministic across runs on the
    // same cached store; a wall-clock anchor drifts second-to-second.
    let anchor = anchor_str(db, opts)?;
    // Route `complexity_metrics` read through the same dispatcher the
    // four sibling complexity-reading analyses use. Without this,
    // `--group-file` would silently emit `0.0` cognitive for every
    // grouped entity (the LEFT JOIN against raw-path complexity rows
    // never matches the rewritten group paths) and the `pr_cx`
    // percentile rank would collapse to a constant — same class of
    // bug `grouped_complexity::source_table` was built to prevent for
    // hotspots / code_health / god_classes / stale_code.
    let cm_src = crate::analyses::grouped_complexity::source_table(opts);
    // Rename-aware like every other path-aggregating analysis: without the
    // lineage rewrite, a renamed file splits its revisions / lead-time /
    // WIP-age history at the rename (the old path is dead at HEAD, the new
    // one starts from one revision). Same shape as `stale-code`.
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(&SQL.replace("{cm_src}", cm_src), opts);
    super::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, anchor, row_limit],
        "delivery-friction",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        &sql,
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

/// Resolve the `wip_age_days` anchor. `--age-time-now` (end-of-day of
/// the given calendar date) when set; otherwise the newest committer
/// date in the store, which keeps the result deterministic across runs.
fn anchor_str(db: &FactsDb, opts: &Options) -> Result<String> {
    if let Some(d) = opts.age_time_now {
        return Ok(format!(
            "{:04}-{:02}-{:02} 23:59:59",
            d.year(),
            u8::from(d.month()),
            d.day()
        ));
    }
    // `wip_age_days` is measured against `MAX(committer_date)`, so the
    // default anchor is the newest committer date, capped at the wall clock
    // so a single future-dated commit cannot skew every file's age
    // (`--age-time-now`, handled above, still wins outright). An empty store
    // yields NULL → fall back to the Unix epoch so the timestamp cast in the
    // query still parses.
    db.query_row(
        &format!(
            "SELECT COALESCE(CAST({now_anchor} AS TEXT), '1970-01-01 00:00:00') FROM commits",
            now_anchor = crate::analyses::query::clamped_now_anchor("committer_date")
        ),
        [],
        |r| r.get::<_, String>(0),
    )
}
