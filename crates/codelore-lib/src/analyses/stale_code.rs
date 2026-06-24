//! `stale-code` analysis.
//!
//! Surfaces files that are alive at HEAD but haven't been touched in
//! N+ months AND carry low cognitive complexity — the signature of
//! code that's likely unused / abandoned but hasn't been deleted.
//! Defaults: 12 months untouched + cognitive ≤ 5 (functions /
//! constants / boilerplate). The intersection minimises false
//! positives: critical low-complexity code (config / constants) that
//! was recently TOUCHED stays in the codebase; the surfacing list is
//! files that are BOTH small AND forgotten.
//!
//! Output is sorted by months-since-touch DESC so the worst
//! offenders sit at the top.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StaleCodeRow {
    pub path: String,
    /// Calendar-date string of the most recent change to this file.
    pub last_touched: String,
    pub months_since_touched: u32,
    /// Max cognitive across this file's entities. Files where every
    /// entity is below the cognitive threshold are candidates; the
    /// per-file max reported here is the highest of those.
    pub max_cognitive: f64,
}

/// Default minimum age in months. Matches the "abandoned" heuristic
/// used by code-maat's `code-age` follow-ups.
const DEFAULT_MIN_MONTHS: u32 = 12;

/// Default upper bound on cognitive complexity. Sonar's "trivial"
/// threshold sits at 5 (file-level max).
const DEFAULT_MAX_COGNITIVE: f64 = 5.0;

const SQL: &str = "
    WITH live_paths AS (
        SELECT path, MAX(date) AS last_touched
        FROM changes
        INNER JOIN commits USING (rev)
        GROUP BY path
        HAVING MAX(CASE WHEN change_type = 'deleted' THEN 1 ELSE 0 END) = 0
    ),
    file_complexity AS (
        SELECT path, MAX(cognitive) AS max_cognitive
        FROM {cm_src}
        WHERE cognitive IS NOT NULL
        GROUP BY path
    )
    , months_calc AS (
        SELECT
            lp.path,
            lp.last_touched,
            (
                12 * (EXTRACT(year FROM CAST(? AS TIMESTAMP))
                      - EXTRACT(year FROM lp.last_touched))
              + (EXTRACT(month FROM CAST(? AS TIMESTAMP))
                 - EXTRACT(month FROM lp.last_touched))
              - CASE WHEN EXTRACT(day FROM CAST(? AS TIMESTAMP))
                        < EXTRACT(day FROM lp.last_touched) THEN 1 ELSE 0 END
            )::INTEGER AS months_since
        FROM live_paths lp
    )
    SELECT
        mc.path,
        CAST(CAST(mc.last_touched AS DATE) AS TEXT) AS last_touched,
        mc.months_since,
        COALESCE(fc.max_cognitive, 0)::DOUBLE AS max_cognitive
    FROM months_calc mc
    LEFT JOIN file_complexity fc ON fc.path = mc.path
    WHERE mc.months_since >= ?
      AND COALESCE(fc.max_cognitive, 0) <= ?
    ORDER BY mc.months_since DESC, mc.path ASC
    LIMIT ?
";

/// Run the `stale-code` analysis. Returns rows ranked by months-
/// since-touch (highest first).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "stale-code", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_stale_code(db: &FactsDb, opts: &Options) -> Result<Vec<StaleCodeRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    // Staleness anchor. `--age-time-now` (end-of-day of the given
    // calendar date) when set — matching `code-age` / `knowledge-islands`
    // — so the back-test pattern works; otherwise the newest commit date
    // in the store. The default is the max commit date (NOT the wall
    // clock) so output is deterministic across runs on the same cached
    // store; a wall-clock anchor drifts the months-since-touch arithmetic
    // second-to-second.
    let anchor = anchor_str(db, opts)?;
    let cm_src = crate::analyses::grouped_complexity::source_table(opts);
    let sql = SQL.replace("{cm_src}", cm_src);
    super::query::explain_if_requested(
        db,
        &sql,
        params![
            anchor,
            anchor,
            anchor,
            DEFAULT_MIN_MONTHS,
            DEFAULT_MAX_COGNITIVE,
            row_limit
        ],
        "stale-code",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        &sql,
        params![
            anchor,
            anchor,
            anchor,
            DEFAULT_MIN_MONTHS,
            DEFAULT_MAX_COGNITIVE,
            row_limit
        ],
        "stale-code",
        |r| {
            Ok(StaleCodeRow {
                path: r.get::<_, String>(0)?,
                last_touched: r.get::<_, String>(1)?,
                months_since_touched: r.get::<_, i32>(2).map(|v| u32::try_from(v).unwrap_or(0))?,
                max_cognitive: r.get::<_, f64>(3)?,
            })
        },
    )
}

/// Resolve the staleness anchor. `--age-time-now` (end-of-day of the
/// given calendar date) when set; otherwise the newest commit date in
/// the store, which keeps the result deterministic across runs.
fn anchor_str(db: &FactsDb, opts: &Options) -> Result<String> {
    if let Some(d) = opts.age_time_now {
        return Ok(format!(
            "{:04}-{:02}-{:02} 23:59:59",
            d.year(),
            u8::from(d.month()),
            d.day()
        ));
    }
    // `MAX(date)` is the newest commit; cast to TEXT for a parseable
    // anchor. An empty store yields NULL → fall back to the Unix epoch
    // so the timestamp cast in the query still parses (the result is an
    // empty stale-code set either way).
    db.query_row(
        "SELECT COALESCE(CAST(MAX(date) AS TEXT), '1970-01-01 00:00:00') FROM commits",
        [],
        |r| r.get::<_, String>(0),
    )
}
