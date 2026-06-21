//! Code age analysis — entity → time since last modification.
//!
//! Reference date is `opts.age_time_now` if set, else today (UTC).
//!
//! ## What's emitted (modern default)
//!
//! For each file (or canonical-lineage entity under `--use-canonical-lineage`),
//! the analysis emits:
//!
//! - `path` — the entity identifier
//! - `age_months` — whole calendar months between the latest qualifying
//!   commit and the anchor date (interval-month semantics: Mar 15 →
//!   Apr 1 = 0 months, not 1 — see PAR-4 SQL doc below)
//! - `age_days` — whole days between the latest qualifying commit and
//!   the anchor — finer-grained precision than code-maat's months-only
//!   output, useful for sort tie-breaking and recency triage
//! - `last_modified` — calendar date of the latest qualifying commit
//!   (context column — helps the operator see WHEN rather than just
//!   HOW LONG AGO)
//!
//! Code-maat emits only `entity, age-months`. We add the extra columns
//! because they cost nothing at query time and answer follow-up
//! questions ("how recently?", "is this a stale stale or a fresh stale?")
//! without re-running the analysis.
//!
//! ## Anchor-date filter (PAR-2 fix)
//!
//! `--age-time-now` lets the operator anchor the "now" used by the age
//! calculation. To make the back-test pattern (`--age-time-now <past>`)
//! return historically-faithful results, the SQL filters out commits
//! whose date is AFTER the anchor — same semantics as code-maat's
//! `changes-within-time-span`. Without this filter the back-test
//! returned NEGATIVE ages for files modified between the anchor and
//! today, which is meaningless output.
//!
//! Research basis: see `docs/research-foundations.md` entry "code-age"
//! (inspired by Dan North's "short software half-life" talk;
//! quantitative analysis in Tornhill, *Your Code as a Crime Scene*,
//! 2015).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeAgeRow {
    pub path: String,
    pub age_months: i32,
    pub age_days: i32,
    pub last_modified: String,
}

// `WHERE commits.date <= anchor`: inclusive of commits made AT the
// anchor moment. The natural mental model for `--age-time-now 2026-06-01`
// is "as of June 1st, including that day's commits", and with the
// default anchor (`now`) we want to include all commits up to and
// including the current second. Code-maat used a strict `<` operator
// because their day-precision dates never encountered the equality case
// in real repos; codelore stores TIMESTAMP and so must be explicit.
// PAR-4: `age_months` uses interval-month semantics (whole calendar
// months elapsed between MAX(commit) and anchor), NOT `DATE_DIFF`'s
// month-boundary-crossing count. Concretely:
//
//   - Mar 15 → Apr 1  → DATE_DIFF = 1 month (boundary crossed)
//                     → interval  = 0 months (not yet a full month)
//   - Mar 15 → Apr 16 → DATE_DIFF = 1, interval = 1 (full month + 1 day)
//   - Mar 31 → Apr 30 → DATE_DIFF = 1, interval = 0 (one day short)
//
// `joda-time`'s `(tc/interval start end)` followed by `tc/in-months` is
// the reference semantic (that's what code-maat uses). The closed-form
// computation is: 12*(yr-yr) + (mo-mo), minus 1 if the day-of-month
// hasn't been reached yet.
//
// Implemented inline in SQL so the analysis stays as a single
// parameterised query (no post-processing in Rust). The
// `EXTRACT(year/month/day FROM ...)` calls work on both `DATE` and
// `TIMESTAMP` types, so we don't need to cast the anchor or the
// `MAX(...)` aggregate to a specific shape.
// F16 fix: code-age now filters to files that are LIVE AS OF THE ANCHOR
// MOMENT (not just live at HEAD — back-test pattern needs the historical
// view). The `live_paths_at_anchor` CTE takes the same anchor parameter
// as the existing PAR-2 filter and selects paths whose latest change
// at-or-before anchor is not a deletion. This drops 2-year-old deleted
// files from current-anchor reports AND correctly resurrects files in
// back-test mode that were deleted later.
const SQL: &str = "
    WITH live_paths_at_anchor AS (
        SELECT path FROM (
            SELECT c.path,
                   arg_max(
                       c.change_type,
                       ROW(commits.date, -commits.rowid)
                   ) AS change_type
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
            WHERE commits.date <= CAST(? AS TIMESTAMP)
            GROUP BY c.path
        ) WHERE change_type != 'deleted'
    ),
    per_path AS (
        SELECT
            changes.path,
            MAX(commits.date) AS last_at,
            -- (rev, path) is the changes PK so rev is unique within each
            -- `GROUP BY changes.path` group. Plain COUNT skips DuckDB's
            -- distinct-tracking overhead.
            COUNT(changes.rev) AS n_revs
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        INNER JOIN live_paths_at_anchor USING (path)
        WHERE commits.date <= CAST(? AS TIMESTAMP)
        GROUP BY changes.path
    )
    SELECT
        path,
        (
            12 * (EXTRACT(year FROM CAST(? AS TIMESTAMP))
                  - EXTRACT(year FROM last_at))
          + (EXTRACT(month FROM CAST(? AS TIMESTAMP))
             - EXTRACT(month FROM last_at))
          - CASE WHEN EXTRACT(day FROM CAST(? AS TIMESTAMP))
                    < EXTRACT(day FROM last_at) THEN 1 ELSE 0 END
        )::INTEGER AS age_months,
        DATE_DIFF('day', last_at, CAST(? AS TIMESTAMP)) AS age_days,
        CAST(CAST(last_at AS DATE) AS TEXT) AS last_modified
    FROM per_path
    WHERE n_revs >= ?
    ORDER BY age_months ASC, age_days ASC, path ASC
    LIMIT ?
";

#[tracing::instrument(name = "code-age", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_code_age(db: &FactsDb, opts: &Options) -> Result<Vec<CodeAgeRow>> {
    // Reference anchor for the "now" of age calculation, with two
    // separate semantics depending on whether the user passed
    // `--age-time-now`:
    //
    // - **No flag (default):** anchor is the current instant
    //   (`OffsetDateTime::now_utc()`), to-the-second precision. Matches
    //   code-maat's default `(tc/now)`.
    // - **Flag set to a calendar date:** anchor is END-OF-DAY of that
    //   date (`23:59:59`). The natural user mental model for
    //   `--age-time-now 2026-06-01` is "as of June 1st, including
    //   that day's commits", not "as of midnight at the start of
    //   June 1st, excluding the day".
    //
    // The `WHERE commits.date < anchor` filter then correctly drops
    // post-anchor commits in both cases (back-test pattern returns
    // historically-faithful results, default pattern drops only
    // genuinely future-dated commits like badly-set
    // `GIT_AUTHOR_DATE`).
    let now_str = if let Some(d) = opts.age_time_now {
        format!(
            "{:04}-{:02}-{:02} 23:59:59",
            d.year(),
            u8::from(d.month()),
            d.day()
        )
    } else {
        let n = time::OffsetDateTime::now_utc();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            n.year(),
            u8::from(n.month()),
            n.day(),
            n.hour(),
            n.minute(),
            n.second(),
        )
    };
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![
            now_str,
            now_str,
            now_str,
            now_str,
            now_str,
            now_str,
            opts.min_revs,
            row_limit
        ],
        "code-age",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-age: {e}")))?;
    let rows = stmt
        .query_map(
            params![
                now_str,
                now_str,
                now_str,
                now_str,
                now_str,
                now_str,
                opts.min_revs,
                row_limit
            ],
            |r| {
                Ok(CodeAgeRow {
                    path: r.get::<_, String>(0)?,
                    age_months: i32::try_from(r.get::<_, i64>(1)?).unwrap_or(i32::MAX),
                    age_days: i32::try_from(r.get::<_, i64>(2)?).unwrap_or(i32::MAX),
                    last_modified: r.get::<_, String>(3)?,
                })
            },
        )
        .map_err(|e| CodeLoreError::Analysis(format!("query code-age: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-age: {e}")))
}
