//! `hotspot-velocity` analysis — which files are *heating up*.
//!
//! Hotspots rank all-time churn; velocity asks the forward-looking
//! question: is a file's change rate **accelerating**? A file that
//! suddenly starts churning is becoming a hotspot before its all-time
//! count says so — an early-warning signal.
//!
//! For each file the analysis compares two windows ending at the latest
//! commit in the data:
//!
//! - **recent** — the last [`RECENT_DAYS`] days,
//! - **baseline** — the [`BASELINE_DAYS`] days *before* that.
//!
//! Both are normalised to revisions-per-week (the windows have different
//! lengths) and `acceleration = recent_per_week − baseline_per_week`.
//! Positive = heating up, negative = cooling down. Subtracting rates
//! (rather than a ratio) keeps brand-new files — baseline 0, recent high
//! — at the top where they belong, instead of dividing by zero.
//!
//! ## Anchoring
//!
//! "Now" is `MAX(commits.date)`, NOT wall-clock today, so the result is
//! reproducible and survives back-testing (the same anchor lesson
//! `code-age` / `stale-code` learned). A repo whose last commit was a
//! year ago still reports its final-year velocity, not all-zeros.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

/// Length of the "recent" window in days.
pub const RECENT_DAYS: u32 = 30;
/// Length of the "baseline" window (immediately preceding recent) in days.
pub const BASELINE_DAYS: u32 = 90;

/// One hotspot-velocity finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HotspotVelocityRow {
    /// The file.
    pub path: String,
    /// Revisions in the recent window.
    pub revs_recent: u32,
    /// Revisions in the baseline window.
    pub revs_baseline: u32,
    /// Recent revisions per week.
    pub recent_per_week: f64,
    /// Baseline revisions per week.
    pub baseline_per_week: f64,
    /// `recent_per_week − baseline_per_week`. Positive = heating up,
    /// negative = cooling down. The ranking signal.
    pub acceleration: f64,
}

// Two windows anchored at MAX(commits.date): recent = last RECENT_DAYS,
// baseline = the BASELINE_DAYS before that. Rates are per-week so the
// unequal-length windows compare fairly. Only files touched in the recent
// window are reported (a file that went fully cold is stale-code's job);
// the `>= ?` floor drops one-off noise.
const SQL: &str = "
    WITH win AS (
        SELECT
            MAX(date) AS now_ts,
            MAX(date) - INTERVAL '30 days'  AS recent_start,
            MAX(date) - INTERVAL '120 days' AS baseline_start
        FROM commits
    ),
    recent AS (
        SELECT ch.path, COUNT(ch.rev) AS revs_recent
        FROM changes ch
        INNER JOIN commits c ON c.rev = ch.rev
        CROSS JOIN win w
        WHERE c.date > w.recent_start AND c.date <= w.now_ts
        GROUP BY ch.path
    ),
    baseline AS (
        SELECT ch.path, COUNT(ch.rev) AS revs_baseline
        FROM changes ch
        INNER JOIN commits c ON c.rev = ch.rev
        CROSS JOIN win w
        WHERE c.date > w.baseline_start AND c.date <= w.recent_start
        GROUP BY ch.path
    )
    SELECT
        r.path,
        r.revs_recent,
        COALESCE(b.revs_baseline, 0) AS revs_baseline,
        r.revs_recent * 7.0 / 30.0 AS recent_per_week,
        COALESCE(b.revs_baseline, 0) * 7.0 / 90.0 AS baseline_per_week,
        (r.revs_recent * 7.0 / 30.0)
            - (COALESCE(b.revs_baseline, 0) * 7.0 / 90.0) AS acceleration
    FROM recent r
    LEFT JOIN baseline b ON r.path = b.path
    WHERE (r.revs_recent + COALESCE(b.revs_baseline, 0)) >= ?
    ORDER BY acceleration DESC, revs_recent DESC, path ASC
    LIMIT ?
";

/// Run the `hotspot-velocity` analysis. Returns files ranked by change
/// acceleration (heating up first).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors.
#[tracing::instrument(name = "hotspot-velocity", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_hotspot_velocity(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotVelocityRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        SQL,
        params![opts.min_revs, row_limit],
        "hotspot-velocity",
        opts,
    )?;
    crate::analyses::query::query_map_collect(
        db,
        SQL,
        params![opts.min_revs, row_limit],
        "hotspot-velocity",
        |r| {
            Ok(HotspotVelocityRow {
                path: r.get::<_, String>(0)?,
                revs_recent: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
                revs_baseline: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                recent_per_week: r.get::<_, f64>(3)?,
                baseline_per_week: r.get::<_, f64>(4)?,
                acceleration: r.get::<_, f64>(5)?,
            })
        },
    )
}
