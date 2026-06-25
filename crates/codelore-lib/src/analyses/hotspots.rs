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
//!
//! Research basis: see `docs/research-foundations.md` entry "hotspots"
//! (Tornhill, *Software Design X-Rays*, 2018; `McCabe`, *IEEE TSE* 1976
//! — cyclomatic complexity; Campbell / `SonarSource` 2018 — cognitive
//! complexity formalisation; Coleman et al., *IEEE Computer* 1994 +
//! `SEI` 1997 variant — Maintainability Index).
//!
//! `mi` (Maintainability Index, SEI variant) is computed by the vendored
//! `codelore-rca` fork of Mozilla `rust-code-analysis` at ingest. We pull
//! the **file-level** value by joining `entities` and filtering to
//! `kind = 'unit'` — that's the rust-code-analysis convention for the
//! root space (file/module level), whose cumulative Halstead /
//! Cyclomatic / SLOC inputs produce the file's MI per Coleman 1994.
//! Per-function MIs are also in `complexity_metrics` but averaging them
//! is mathematically unsound (MI is non-linear in its inputs).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HotspotRow {
    pub path: String,
    pub revisions: u32,
    pub cognitive: f64,
    pub code_health: f64,
    pub hotspot_score: f64,
    /// File-level Maintainability Index (SEI variant). `None` when the
    /// file has no `kind='unit'` complexity entry — typically because
    /// the language isn't supported by `codelore-rca` or the file was
    /// skipped for size / clone reasons at ingest.
    pub mi: Option<f64>,
    /// Percentile rank of this file's MI within the analyzed repo, in
    /// `[0, 1]`. `None` when `mi` is unknown. Used by the [`crate::analyses::mi::MiBand`]
    /// classifier to derive a repo-relative Low / Moderate / High label —
    /// see that module's docs for why the bands are percentile-based
    /// rather than absolute Coleman/SEI thresholds.
    pub mi_rank: Option<f64>,
    /// Percentage of commits touching this file that carry an AI-attribution
    /// signal (`ai-assisted` or `ai-authored` per `identity::bots`). Range
    /// `[0, 100]`. `None` when no commits touched the file (shouldn't
    /// happen in practice since hotspots filters to `revs >= min_revs`, but
    /// defensive against schema drift). Unlike MI, this is a true
    /// percentage so absolute interpretation is meaningful across repos.
    pub ai_pct: Option<f64>,
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
/// rename-aware). `{cm_src}` becomes `complexity_metrics` or
/// `complexity_metrics_grouped` (per `analyses::grouped_complexity`).
/// `{file_mi_cte}` swaps the `file_mi` CTE body between the
/// `entities`-join (raw paths) form and the pre-baked-from-grouped
/// form — the entities join can't be reused post-grouping because
/// `entities.path` is never rewritten by `apply_grouping`, so the
/// rolled-up `mi` must come straight from
/// `complexity_metrics_grouped.mi`.
#[must_use]
pub fn build_sql(src: &str, cm_src: &str) -> String {
    let file_mi_cte = if cm_src == "complexity_metrics_grouped" {
        FILE_MI_GROUPED
    } else {
        FILE_MI_RAW
    };
    SQL.replace("FROM changes\n", &format!("FROM {src}\n"))
        .replace("{cm_src}", cm_src)
        .replace("{file_mi_cte}", file_mi_cte)
}

/// Returns the SAME hotspots SQL with `?` placeholders substituted for
/// inline values — used by the Parquet writer, which routes through
/// `DuckDB COPY ... TO` and can't accept bind parameters. Sharing the
/// formula with [`build_sql`] eliminates the silent-drift risk between
/// the two paths.
#[must_use]
pub fn build_inlined_sql(src: &str, cm_src: &str, min_revs: u32, row_limit: i64) -> String {
    // SQL has exactly two `?` placeholders: first is min_revs (HAVING),
    // second is row_limit (LIMIT). Substitute in order.
    build_sql(src, cm_src)
        .replacen('?', &min_revs.to_string(), 1)
        .replace('?', &row_limit.to_string())
}

/// `file_mi` body for the raw (ungrouped) path — joins `entities` to
/// filter `kind='unit'` since `complexity_metrics` doesn't store kind.
/// The `e.rev_last_seen = cm.rev` predicate is the lockstep invariant:
/// `append_entity_row` and `append_metric_row` both receive the same
/// `head_rev`, so the alive entity at the metric's rev is the only
/// row that should match. Lex-comparing SHA strings as if chronological
/// is unreliable (lex order on SHA hex is essentially random vs commit
/// order). `MAX(cm.mi)` collapses to the single matched row by
/// construction.
const FILE_MI_RAW: &str = "file_mi AS (
        SELECT
            cm.path,
            MAX(cm.mi) AS mi
        FROM complexity_metrics cm
        INNER JOIN entities e
            ON e.path = cm.path
            AND e.name = cm.name
            AND e.rev_last_seen = cm.rev
        WHERE e.kind = 'unit' AND cm.mi IS NOT NULL
        GROUP BY cm.path
    )";

/// `file_mi` body for the grouped path — `complexity_metrics_grouped.mi`
/// is already the per-group `MAX` of `kind='unit'` MI rows (the
/// `apply_grouping` materialiser applied the same `entities`-join +
/// `kind='unit'` filter before collapsing to one row per group). Read
/// it directly — re-joining `entities` here would silently produce
/// NULL MI for every grouped entity because `entities.path` is
/// never rewritten by `apply_grouping`.
const FILE_MI_GROUPED: &str = "file_mi AS (
        SELECT path, mi
        FROM complexity_metrics_grouped
        WHERE mi IS NOT NULL
    )";

pub const SQL: &str = "
    WITH file_revs AS (
        -- (rev, path) is the changes PK, so COUNT(rev) == COUNT(DISTINCT rev)
        -- per path. Plain COUNT avoids DuckDB's distinct-tracking overhead.
        SELECT path, COUNT(rev) AS revs
        FROM changes
        GROUP BY path
        HAVING revs >= ?
    ),
    file_complexity AS (
        SELECT path, MAX(cognitive) AS cognitive
        FROM {cm_src}
        GROUP BY path
    ),
    -- File-level MI body is swapped at build time depending on whether
    -- grouping is active — see `FILE_MI_RAW` / `FILE_MI_GROUPED`. The
    -- raw form joins `entities` to filter `kind='unit'`; the grouped
    -- form reads pre-baked `mi` from `complexity_metrics_grouped`.
    {file_mi_cte},
    -- Per-file AI-attribution percentage: share of commits touching this
    -- file that carry an AI-attribution signal (ai-assisted or
    -- ai-authored). NULLIF guards against div-by-zero on the empty join
    -- (defensive; file_revs already filters revs >= min_revs).
    -- Categories come from identity::bots — schema is stable.
    file_ai AS (
        SELECT
            ch.path,
            COUNT(CASE WHEN co.ai_attribution IN ('ai-assisted', 'ai-authored')
                       THEN 1 END) * 100.0
                / NULLIF(COUNT(ch.rev), 0) AS ai_pct
        FROM changes ch
        INNER JOIN commits co ON co.rev = ch.rev
        GROUP BY ch.path
    ),
    -- Repo-relative MI percentile rank, computed ONLY over files that
    -- actually have a `mi` value — files without a `kind='unit'` entry
    -- (unsupported language / skipped file) MUST NOT skew the
    -- distribution. PERCENT_RANK over file_mi emits values in [0, 1].
    -- See analyses/mi.rs for why bands are repo-relative rather than
    -- absolute Coleman/SEI thresholds (literature thresholds would
    -- classify ~100% of any large codebase as 'low maintainability').
    file_mi_ranked AS (
        SELECT
            path,
            mi,
            PERCENT_RANK() OVER (ORDER BY mi) AS mi_rank
        FROM file_mi
    ),
    joined AS (
        SELECT
            fr.path,
            fr.revs,
            COALESCE(fc.cognitive, 0) AS cognitive,
            fmr.mi AS mi,
            fmr.mi_rank AS mi_rank,
            fa.ai_pct AS ai_pct
        FROM file_revs fr
        LEFT JOIN file_complexity fc ON fc.path = fr.path
        LEFT JOIN file_mi_ranked fmr ON fmr.path = fr.path
        LEFT JOIN file_ai fa ON fa.path = fr.path
    ),
    ranked AS (
        SELECT
            path,
            revs,
            cognitive,
            mi,
            mi_rank,
            ai_pct,
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
        pr_rev * pr_cx * (100.0 - GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx)))) / 4.0 AS score,
        mi,
        mi_rank,
        ai_pct
    FROM ranked
    ORDER BY score DESC, path ASC
    LIMIT ?
";

#[tracing::instrument(name = "hotspots", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    // Unified dispatch: honours both --time-bucket and --use-canonical-lineage,
    // including the composition where both flags are on (bucketing of the
    // lineage-resolved view).
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let cm_src = crate::analyses::grouped_complexity::source_table(opts);
    let sql = build_sql(src, cm_src);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "hotspots",
        opts,
    )?;
    crate::analyses::query::query_map_collect(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "hotspots",
        |r| {
            Ok(HotspotRow {
                path: r.get::<_, String>(0)?,
                revisions: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(2)?,
                code_health: r.get::<_, f64>(3)?,
                hotspot_score: r.get::<_, f64>(4)?,
                mi: r.get::<_, Option<f64>>(5)?,
                mi_rank: r.get::<_, Option<f64>>(6)?,
                ai_pct: r.get::<_, Option<f64>>(7)?,
            })
        },
    )
}
