//! Hotspot ranking analysis. `code_health` is on `[0, 100]` (higher = healthier);
//! `percentile_rank` is on `[0, 1]`; the score formula combines them so unhealthy +
//! frequently-changed + complex files rank highest. Output range is `[0, 10]`:
//!
//! ```text
//!   hotspot_score(entity) = percentile_rank(revisions)
//!                         × percentile_rank(cognitive_complexity)
//!                         × (100 − code_health) / 4
//! ```
//!
//! Why divide by 4 (not 10)?  `code_health` is computed inline as
//! `100 × (1 − 0.40 × normalize(cognitive))`, so its empirical range is
//! `[60, 100]` — the `0.40` weight bounds the deduction. That makes
//! `(100 − code_health) ∈ [0, 40]`; multiplied by two percent ranks (each
//! in `[0, 1]`) the unscaled product caps at `40`. Dividing by 4 lands the
//! score in the documented `[0, 10]` range and matches the `CodeScene`
//! convention that `≈10 ⇒ "on fire"`.
//!
//! Earlier divisor history: the original `(10 − code_health) / 10` produced
//! NEGATIVE scores (`code_health` is `[0, 100]`, not `[0, 10]`). The previous
//! fix `(100 − code_health) / 10` kept the sign positive but capped output at
//! `4.0` instead of `10.0` — so the documented `[0, 10]` scale was never
//! reached and "on fire" was unreachable. The current `/ 4.0` closes that
//! documented-range-vs-math drift.
//!
//! `code_health` itself is computed inline using cognitive complexity only;
//! the churn / fragmentation / coupling inputs from `code_health` analysis
//! aren't reused here — `hotspots` is the lightweight "what to look at first"
//! ranking, `code-health` is the deeper analysis.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HotspotRow {
    pub path: String,
    pub revisions: u32,
    pub cognitive: f64,
    pub code_health: f64,
    pub hotspot_score: f64,
}

// PERCENT_RANK() is standard SQL:2003 and is supported by DuckDB ≥0.2.
// Aggregate per-path:
//   revisions:   count of distinct commits in changes
//   cognitive:   MAX of cognitive across all entities in the path's file
//   code_health: 100 * (1 − 0.40 * normalize(cognitive))   ∈ [60, 100]
//   hotspot_score: percent_rank(revs) * percent_rank(cog) * (100 − code_health) / 4
//                  ∈ [0, 10] — see module-level docstring for why the divisor
//                  is 4, not 10 (code_health bottoms at 60, not 0).
/// `{src}` becomes `changes` (legacy) or `changes_lineage` (canonical
/// rename-aware). Kept as a `format!()` template so `--use-canonical-lineage`
/// flips the table without rewriting the rest of the SQL.
#[must_use]
pub fn build_sql(src: &str) -> String {
    SQL.replace("FROM changes\n", &format!("FROM {src}\n"))
}

/// Returns the SAME hotspots SQL with `?` placeholders substituted for
/// inline values — used by the Parquet writer, which routes through
/// `DuckDB COPY ... TO` and can't accept bind parameters. Sharing the
/// formula with [`build_sql`] eliminates the silent-drift risk between
/// the two paths.
#[must_use]
pub fn build_inlined_sql(src: &str, min_revs: u32, row_limit: i64) -> String {
    // SQL has exactly two `?` placeholders: first is min_revs (HAVING),
    // second is row_limit (LIMIT). Substitute in order.
    build_sql(src)
        .replacen('?', &min_revs.to_string(), 1)
        .replace('?', &row_limit.to_string())
}

pub const SQL: &str = "
    WITH file_revs AS (
        SELECT path, COUNT(DISTINCT rev) AS revs
        FROM changes
        GROUP BY path
        HAVING revs >= ?
    ),
    file_complexity AS (
        SELECT path, MAX(cognitive) AS cognitive
        FROM complexity_metrics
        GROUP BY path
    ),
    joined AS (
        SELECT
            fr.path,
            fr.revs,
            COALESCE(fc.cognitive, 0) AS cognitive
        FROM file_revs fr
        LEFT JOIN file_complexity fc ON fc.path = fr.path
    ),
    ranked AS (
        SELECT
            path,
            revs,
            cognitive,
            PERCENT_RANK() OVER (ORDER BY revs) AS pr_rev,
            PERCENT_RANK() OVER (ORDER BY cognitive) AS pr_cx,
            CASE
                WHEN MAX(cognitive) OVER () > 0
                THEN cognitive / MAX(cognitive) OVER ()
                ELSE 0
            END AS norm_cx
        FROM joined
    )
    SELECT
        path,
        revs,
        cognitive,
        GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS code_health,
        pr_rev * pr_cx * (100.0 - GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx)))) / 4.0 AS score
    FROM ranked
    ORDER BY score DESC, path ASC
    LIMIT ?
";

pub fn run_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    // Unified dispatch: honours both --time-bucket and --use-canonical-lineage,
    // including the composition where both flags are on (bucketing of the
    // lineage-resolved view).
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let sql = build_sql(src);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "hotspots",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare hotspots: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(HotspotRow {
                path: r.get::<_, String>(0)?,
                revisions: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(2)?,
                code_health: r.get::<_, f64>(3)?,
                hotspot_score: r.get::<_, f64>(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query hotspots: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect hotspots: {e}")))
}
