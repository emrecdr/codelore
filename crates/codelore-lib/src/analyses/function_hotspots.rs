//! `function-hotspots` analysis — repo-wide function-level hotspot ranking.
//!
//! Every hotspot-family analysis (`hotspots`, `hotspot-velocity`, `code-health`)
//! is FILE-granularity: a 2000-line file with one genuinely hot function looks
//! identical to one with uniform low-grade churn. This analysis ranks
//! HEAD-live **functions** by the same `revs × cognitive`-style score
//! `hotspots` uses, so the two are comparable in spirit.
//!
//! **Data sources — no tree-sitter reparse.** HEAD function/method spans come
//! from `entities` (`path, name, kind, start_line, end_line`), already
//! populated at ingest; `entities.rev_introduced`/`rev_last_seen` are
//! **degenerate** (always `head_rev` — a single HEAD-only appender), so this
//! answers "hot now," not "was hot historically." Per-function complexity at
//! HEAD comes from `complexity_metrics`. Revision history comes from `hunks`
//! (`rev, path, new_start, new_lines`).
//!
//! **Hunk↔span overlap.** The overlap predicate is the exact one
//! [`crate::analyses::function_xray`] already applies for a single
//! `--target` file, transliterated to SQL so it can run as a repo-wide join
//! instead of a per-target Rust loop: a hunk touching
//! `[new_start, new_start + new_lines)` overlaps a function span
//! `[start_line, end_line]` when `new_start <= end_line AND new_start +
//! new_lines > start_line`; a pure deletion (`new_lines = 0`) attributes to
//! the span whose range contains the anchor line
//! (`start_line <= new_start <= end_line`). See
//! `function_xray::hunk_overlaps` for the canonical Rust form and its unit
//! tests covering every edge case.
//!
//! **Rename limitation (same as `function-xray`).** The join is
//! `hunks.path = entities.path` — the current HEAD-relative path. `hunks` is
//! keyed on the literal path recorded at commit time, and rewriting it
//! through the rename-aware lineage CTE
//! ([`crate::analyses::lineage::rewrite`]) would only affect `changes`/
//! `changes_lineage` references, not `hunks` itself, so opting in would do
//! nothing here — a file's pre-rename history is not attributed, matching
//! `function-xray`'s documented caveat exactly.
//!
//! **Approximate attribution.** Like `function-xray`, hunk line numbers are
//! historical (recorded at commit time) while the span is the function's
//! CURRENT (HEAD) location — line numbers drift as intervening commits
//! insert/remove lines above a function, so attribution is an approximation,
//! not an exact replay. This mirrors `function-xray`'s documented limitation.
//!
//! Score formula — identical shape to [`crate::analyses::hotspots`]:
//!
//! ```text
//!   function_hotspot_score(f) = percentile_rank(revs)
//!                              × percentile_rank(cognitive)
//!                              × (100 − cognitive_health) / 4
//! ```
//!
//! where `cognitive_health = 100 × (1 − 0.40 × normalize(cognitive))`,
//! `normalize` divides by the max cognitive complexity across all HEAD-live
//! functions. Output range `[0, 10]`, same scale as `hotspots.hotspot_score`
//! — see `hotspots.rs`'s module doc for the full derivation of the `/ 4`
//! divisor and the `[60, 100]` range of `cognitive_health`.
//!
//! Research basis: Gall et al. ICSM 2003 (`HistoryFinder`, function-level
//! churn) + Tornhill 2018 (Software Design X-Rays, the file-level hotspot
//! score this mirrors).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

/// One HEAD-live function/method, ranked by the `hotspots`-style score.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionHotspotRow {
    pub path: String,
    /// Function or method name, as recorded in `entities` — carries the
    /// `{name}@{start_line}-{end_line}` disambiguation suffix
    /// `facts::ingest::consumer::dedup_entities` applies at ingest (same
    /// convention `function-xray`'s `function` field uses).
    pub function: String,
    /// Distinct revisions where at least one hunk overlapped this
    /// function's HEAD span. Gated by `--min-revs` (a `HAVING` floor, like
    /// `hotspots`).
    pub revs: u32,
    /// Cognitive complexity at HEAD (from `complexity_metrics`). `0.0` when
    /// no complexity row matched (should not happen for a real function —
    /// `entities` and `complexity_metrics` are written in lockstep at
    /// ingest — but the join stays defensive).
    pub cognitive: f64,
    /// Inline structural proxy on `[60, 100]` (higher = healthier) — the
    /// same formula `hotspots.cognitive_health` uses, computed over the
    /// HEAD-live function population instead of the file population.
    pub cognitive_health: f64,
    /// `percentile_rank(revs) × percentile_rank(cognitive) × (100 −
    /// cognitive_health) / 4`, range `[0, 10]` — see the module doc.
    pub function_hotspot_score: f64,
}

// Mirrors `hotspots.rs::SQL`'s CTE shape, at function granularity. See the
// module doc for the hunk-overlap predicate (transliterated from
// `function_xray::hunk_overlaps`) and the score formula.
const SQL: &str = "
    WITH fn_entities AS (
        SELECT path, name AS function, start_line, end_line
        FROM entities
        WHERE kind IN ('function', 'method')
    ),
    fn_hunk_hits AS (
        SELECT e.path, e.function, h.rev
        FROM fn_entities e
        JOIN hunks h ON h.path = e.path
            AND (
                (h.new_lines = 0 AND h.new_start >= e.start_line AND h.new_start <= e.end_line)
                OR (h.new_lines > 0 AND h.new_start <= e.end_line AND h.new_start + h.new_lines > e.start_line)
            )
    ),
    fn_revs AS (
        SELECT path, function, COUNT(DISTINCT rev) AS revs
        FROM fn_hunk_hits
        GROUP BY path, function
        HAVING COUNT(DISTINCT rev) >= ?
    ),
    fn_complexity AS (
        SELECT cm.path, cm.name AS function, cm.cognitive
        FROM complexity_metrics cm
        JOIN entities e ON e.path = cm.path AND e.name = cm.name
        WHERE e.kind IN ('function', 'method')
    ),
    joined AS (
        SELECT
            fr.path,
            fr.function,
            fr.revs,
            COALESCE(fc.cognitive, 0) AS cognitive
        FROM fn_revs fr
        LEFT JOIN fn_complexity fc ON fc.path = fr.path AND fc.function = fr.function
    ),
    ranked AS (
        SELECT
            path,
            function,
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
        function,
        revs,
        cognitive,
        GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS cognitive_health,
        pr_rev * pr_cx * (100.0 - GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx)))) / 4.0 AS function_hotspot_score
    FROM ranked
    ORDER BY function_hotspot_score DESC, path ASC, function ASC
    LIMIT ?
";

#[tracing::instrument(name = "function-hotspots", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_function_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<FunctionHotspotRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        SQL,
        params![opts.min_revs, row_limit],
        "function-hotspots",
        opts,
    )?;
    crate::analyses::query::query_map_collect(
        db,
        SQL,
        params![opts.min_revs, row_limit],
        "function-hotspots",
        |r| {
            Ok(FunctionHotspotRow {
                path: r.get::<_, String>(0)?,
                function: r.get::<_, String>(1)?,
                revs: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(3)?,
                cognitive_health: r.get::<_, f64>(4)?,
                function_hotspot_score: r.get::<_, f64>(5)?,
            })
        },
    )
}
