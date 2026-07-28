//! Hotspot ranking analysis. `cognitive_health` is on `[0, 100]` (higher = healthier);
//! `percentile_rank` is on `[0, 1]`; the score formula combines them so unhealthy +
//! frequently-changed + complex files rank highest. Output range is `[0, 10]`:
//!
//! ```text
//!   hotspot_score(entity) = percentile_rank(revisions)
//!                         × percentile_rank(cognitive_complexity)
//!                         × (100 − cognitive_health) / 4
//! ```
//!
//! Why divide by 4 (not 10)?  `cognitive_health` is computed inline as
//! `100 × (1 − 0.40 × normalize(cognitive))`, so its empirical range is
//! `[60, 100]` — the `0.40` weight bounds the deduction. That makes
//! `(100 − cognitive_health) ∈ [0, 40]`; multiplied by two percent ranks (each
//! in `[0, 1]`) the unscaled product caps at `40`. Dividing by 4 lands the
//! score in the documented `[0, 10]` range and matches the `CodeScene`
//! convention that `≈10 ⇒ "on fire"`.
//!
//! Earlier divisor history: the original `(10 − cognitive_health) / 10` produced
//! NEGATIVE scores (`cognitive_health` is `[0, 100]`, not `[0, 10]`). The previous
//! fix `(100 − cognitive_health) / 10` kept the sign positive but capped output at
//! `4.0` instead of `10.0` — so the documented `[0, 10]` scale was never
//! reached and "on fire" was unreachable. The current `/ 4.0` closes that
//! documented-range-vs-math drift.
//!
//! `cognitive_health` is the hotspots analysis's own inline structural proxy,
//! computed from cognitive complexity only — it is deliberately NOT the
//! `code-health` analysis's composite score. The churn / fragmentation /
//! coupling inputs that feed the `code-health` composite are not reused here:
//! `hotspots` is the lightweight "what to look at first" ranking, whose
//! `cognitive_health` proxy is bounded to `[60, 100]`, while `code-health`
//! is the deeper analysis whose composite spans the full `[0, 100]`. The same
//! file can read healthy here and unhealthy there — they measure different
//! things, which is why this field is named `cognitive_health` and not
//! `code_health`.
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
    /// Inline structural proxy on `[60, 100]` (higher = healthier), computed
    /// from cognitive complexity alone as `100 × (1 − 0.40 × normalize(cognitive))`.
    /// This is the hotspots analysis's own signal for ranking — NOT the
    /// `code-health` analysis's 8-smell composite score. A file can read
    /// healthy here and unhealthy in `code-health`; the two are distinct.
    pub cognitive_health: f64,
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
    /// Corpus-anchored hotspot score. The `hotspot_score` formula with its two
    /// repo-relative *cognitive* terms — the complexity percentile rank and the
    /// inline `cognitive_health` proxy's max-normalisation — both replaced by
    /// the file's cognitive-complexity percentile against the calibration
    /// corpus for its language. The revisions percentile stays repo-relative
    /// *by design* (churn is a within-repo concentration signal, not comparable
    /// across repos), so only the complexity coupling is anchored. Unlike
    /// `hotspot_score`, improving or removing another file leaves this value
    /// unchanged — the property that lets an absolute ceiling
    /// (`hotspot_anchored_max`) stay stable under improvement.
    ///
    /// `None` — omitted from every surface, never rendered as `0.00` — when no
    /// calibration artifact is active, or the file's language is absent from the
    /// corpus / pooled below the sample floor. Populated by
    /// [`apply_hotspot_anchor`]; `run_hotspots` alone leaves it `None`, so the
    /// SPA payload and the internal hotspot consumers stay byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspot_score_anchored: Option<f64>,
}

// PERCENT_RANK() is standard SQL:2003 and is supported by DuckDB ≥0.2.
// Aggregate per-path:
//   revisions:   count of distinct commits in changes
//   cognitive:   MAX of cognitive across all entities in the path's file
//   cognitive_health: 100 * (1 − 0.40 * normalize(cognitive))   ∈ [60, 100]
//   hotspot_score: percent_rank(revs) * percent_rank(cog) * (100 − cognitive_health) / 4
//                  ∈ [0, 10] — see module-level docstring for why the divisor
//                  is 4, not 10 (cognitive_health bottoms at 60, not 0).
/// The `file_revs` change source is resolved by routing the assembled SQL
/// through [`crate::analyses::lineage::rewrite`], which rewrites every
/// `FROM changes` / `JOIN changes` to `changes_lineage` / `changes_bucketed`
/// when the corresponding flag is on. `file_ai` is deliberately EXEMPT: its
/// `{ai_src}` placeholder resolves to `changes_lineage` (canonical lineage) or
/// raw `changes`, never `changes_bucketed` — because `ai_pct` joins `commits`
/// on a real `rev` and `changes_bucketed`'s synthetic date-string `rev` would
/// never match, leaving `ai_pct` NULL under `--time-bucket`.
/// `{cm_src}` becomes `complexity_metrics` or `complexity_metrics_grouped`
/// (per `analyses::grouped_complexity`). `{file_mi_cte}` swaps the `file_mi`
/// CTE body between the `entities`-join (raw paths) form and the
/// pre-baked-from-grouped form — the entities join can't be reused
/// post-grouping because `entities.path` is never rewritten by
/// `apply_grouping`, so the rolled-up `mi` must come straight from
/// `complexity_metrics_grouped.mi`.
#[must_use]
pub fn build_sql(opts: &Options, cm_src: &str) -> String {
    let file_mi_cte = if cm_src == "complexity_metrics_grouped" {
        FILE_MI_GROUPED
    } else {
        FILE_MI_RAW
    };
    // `file_ai` must join `commits` on a real `rev` (SHA), so it resolves to
    // the rename-aware but UN-bucketed source — `changes_lineage` under
    // canonical lineage, else raw `changes`. Substituted AFTER `rewrite` so the
    // whole-SQL rewrite (which sends `file_revs` to `changes_bucketed` under
    // `--time-bucket`) leaves the `{ai_src}` placeholder untouched; routing
    // `file_ai` to `changes_bucketed` would null `ai_pct`.
    let ai_src = if opts.use_canonical_lineage {
        "changes_lineage"
    } else {
        "changes"
    };
    let sql = SQL
        .replace("{cm_src}", cm_src)
        .replace("{file_mi_cte}", file_mi_cte);
    crate::analyses::lineage::rewrite(&sql, opts).replace("{ai_src}", ai_src)
}

/// Returns the SAME hotspots SQL with `?` placeholders substituted for
/// inline values — used by the Parquet writer, which routes through
/// `DuckDB COPY ... TO` and can't accept bind parameters. Sharing the
/// formula with [`build_sql`] eliminates the silent-drift risk between
/// the two paths.
#[must_use]
pub fn build_inlined_sql(opts: &Options, cm_src: &str, min_revs: u32, row_limit: i64) -> String {
    // SQL has exactly two `?` placeholders: first is min_revs (HAVING),
    // second is row_limit (LIMIT). Substitute in order.
    build_sql(opts, cm_src)
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
        FROM {ai_src} ch
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
        GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS cognitive_health,
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
    let cm_src = crate::analyses::grouped_complexity::source_table(opts);
    let sql = build_sql(opts, cm_src);
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
                cognitive_health: r.get::<_, f64>(3)?,
                hotspot_score: r.get::<_, f64>(4)?,
                mi: r.get::<_, Option<f64>>(5)?,
                mi_rank: r.get::<_, Option<f64>>(6)?,
                ai_pct: r.get::<_, Option<f64>>(7)?,
                // The corpus anchor is an additive post-pass, not part of the
                // shipped SQL — `apply_hotspot_anchor` fills it. Leaving it
                // `None` here keeps `run_hotspots` (and thus the SPA / internal
                // consumers) byte-identical.
                hotspot_score_anchored: None,
            })
        },
    )
}

/// `pr_rev` (revisions percentile) per path over the FULL population — exactly
/// the `PERCENT_RANK() OVER (ORDER BY revs)` the hotspots `SQL` computes in its
/// `ranked` CTE, before the display `LIMIT`. `joined` is `file_revs` LEFT-JOINed
/// to the complexity / MI / AI CTEs, so it neither adds nor drops a `file_revs`
/// row nor alters `revs`; the percentile therefore depends only on `file_revs`
/// and this stand-alone query reproduces it. Kept OUT of the shared `SQL` (not
/// added as a column) so the Parquet writer, which `COPY`s that SQL verbatim,
/// stays byte-identical. `FROM changes` is routed through [`lineage::rewrite`]
/// like the main `file_revs`, so canonical-lineage / time-bucket rewrites land
/// identically.
///
/// [`lineage::rewrite`]: crate::analyses::lineage::rewrite
const PR_REV_SQL: &str = "
    WITH file_revs AS (
        SELECT path, COUNT(rev) AS revs
        FROM changes
        GROUP BY path
        HAVING revs >= ?
    )
    SELECT path, PERCENT_RANK() OVER (ORDER BY revs) AS pr_rev
    FROM file_revs
";

/// Additive corpus-anchor post-pass: fill `hotspot_score_anchored` on each row
/// whose language the active calibration corpus covers. A pure post-pass — it
/// reads only the corpus and the per-path `pr_rev`, never mutating the shipped
/// fields — so a run without an active artifact leaves every row unchanged.
///
/// The anchored score is the `hotspot_score` formula with both cognitive terms
/// (the `pr_cx` complexity rank and the `norm_cx` max-normalisation inside
/// `cognitive_health`) replaced by `cp`, the file's cognitive-complexity
/// percentile against the corpus — the same lookup the code-health lens uses.
/// `pr_rev` stays repo-relative. With `cp` doing both jobs the closed form is
/// `pr_rev · cp · (100 − 100·(1 − 0.40·cp)) / 4`, i.e. `10 · pr_rev · cp²`, on
/// the same `[0, 10]` scale as `hotspot_score`.
///
/// # Errors
///
/// [`crate::CodeLoreError`] if the `pr_rev` query fails or the `--calibration`
/// artifact cannot be loaded.
pub fn apply_hotspot_anchor(db: &FactsDb, opts: &Options, rows: &mut [HotspotRow]) -> Result<()> {
    let Some(art) = crate::calibration::load_active_artifact(opts)? else {
        return Ok(()); // No artifact active → every row keeps its absent anchor.
    };
    if rows.is_empty() {
        return Ok(());
    }

    // `pr_rev` over the full population, from the same lineage-resolved source
    // the main query uses. `materialize_source` is idempotent, so calling it
    // here keeps `apply_hotspot_anchor` correct when invoked standalone.
    crate::analyses::lineage::materialize_source(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(PR_REV_SQL, opts);
    let pr_rev_by_path: std::collections::HashMap<String, f64> =
        crate::analyses::query::query_map_collect(
            db,
            &sql,
            params![opts.min_revs],
            "hotspot-pr-rev",
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
        )?
        .into_iter()
        .collect();

    for row in rows.iter_mut() {
        let Some(language) = crate::complexity::Tier1Language::from_path(&row.path) else {
            continue; // Not a Tier-1 language → no corpus comparison.
        };
        // `cognitive` is the file's per-function MAX, located against the
        // per-language corpus breakpoints — `None` (no anchor) when the
        // language is uncovered or pooled below the sample floor.
        let Some(cp) =
            crate::calibration::percentile(&art, language.as_str(), "cognitive", row.cognitive)
        else {
            continue;
        };
        let Some(&pr_rev) = pr_rev_by_path.get(&row.path) else {
            continue; // Defensive: every hotspot path is in `file_revs`.
        };
        row.hotspot_score_anchored = Some(anchored_score(pr_rev, cp.p));
    }
    Ok(())
}

/// The corpus-anchored hotspot score for a file with churn percentile `pr_rev`
/// and corpus cognitive percentile `cp` (both in `[0, 1]`).
///
/// The `hotspot_score` SQL formula with `pr_cx` and `norm_cx` — its two
/// repo-relative cognitive terms — both replaced by `cp`, and the
/// `cognitive_health` deduction clamped to `[0, 100]` exactly as the SQL clamps
/// it. Closed form `10 · pr_rev · cp²`, on the same `[0, 10]` scale as
/// `hotspot_score`.
#[must_use]
fn anchored_score(pr_rev: f64, cp: f64) -> f64 {
    let health = (100.0 * (1.0 - 0.40 * cp)).clamp(0.0, 100.0);
    pr_rev * cp * (100.0 - health) / 4.0
}

/// [`run_hotspots`] with the additive corpus anchor applied. The entry the
/// CSV / JSON / markdown surfaces and the `hotspot_anchored_max` gate use; kept
/// distinct from `run_hotspots` so the SPA payload and the internal consumers
/// (refactoring-targets, finding-overlap, change-context, …) stay
/// byte-identical — the anchor is a report / gate reading, not a core field.
///
/// # Errors
///
/// Propagates any error from [`run_hotspots`] or [`apply_hotspot_anchor`].
pub fn run_hotspots_anchored(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>> {
    let mut rows = run_hotspots(db, opts)?;
    apply_hotspot_anchor(db, opts, &mut rows)?;
    Ok(rows)
}

#[cfg(test)]
mod anchor_tests {
    use super::*;
    use crate::calibration::{
        CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MetricQuantiles,
        QUANTILE_POINTS, Stratum,
    };
    use crate::facts::FactsDb;

    /// The anchored score is the SQL formula with `pr_cx` and `norm_cx` both
    /// swapped for the corpus percentile — closed form `10 · pr_rev · cp²`.
    #[test]
    fn anchored_score_matches_closed_form() {
        // Hand-computed: 10 · 0.5 · 0.9² = 4.05.
        assert!((anchored_score(0.5, 0.9) - 4.05).abs() < 1e-12);
        // No churn ⇒ zero, regardless of complexity.
        assert!(anchored_score(0.0, 1.0).abs() < 1e-12);
        // Zero corpus percentile ⇒ zero (health deduction is zero).
        assert!(anchored_score(1.0, 0.0).abs() < 1e-12);
        // Upper bound of the [0, 10] range.
        assert!((anchored_score(1.0, 1.0) - 10.0).abs() < 1e-12);
        // Closed form holds across the range.
        for &(pr, cp) in &[(0.3, 0.7), (0.8, 0.25), (1.0, 0.997)] {
            assert!((anchored_score(pr, cp) - 10.0 * pr * cp * cp).abs() < 1e-12);
        }
    }

    /// A ramp corpus (`q[i] = i / 100`) whose language clears the sample floor:
    /// cognitive `5.0` sits exactly at `q[500]`, so the lookup returns `cp = 0.5`
    /// and the anchored score is the hand-computed `10 · 1.0 · 0.25 = 2.5`.
    fn ramp_artifact(language: &str, sample_functions: u64) -> CalibrationArtifact {
        #[allow(clippy::cast_precision_loss)] // i < 1001, exact in f64
        let quantiles: Vec<f64> = (0..QUANTILE_POINTS).map(|i| i as f64 / 100.0).collect();
        CalibrationArtifact {
            format_version: CALIBRATION_FORMAT_VERSION,
            corpus_vintage: "test-ramp".into(),
            generated_at: "2026-01-01T00:00:00Z".into(),
            repos_included: 1,
            repos_attempted: 1,
            languages: vec![LanguageTable {
                language: language.into(),
                sample_functions,
                strata: vec![Stratum {
                    sloc_min: 0,
                    sloc_max: u64::MAX,
                    metrics: vec![MetricQuantiles {
                        metric: "cognitive".into(),
                        quantiles,
                    }],
                }],
            }],
            repo_metrics: None,
        }
    }

    #[test]
    fn anchored_score_reads_corpus_percentile() {
        let art = ramp_artifact("rust", 1000);
        let cp = crate::calibration::percentile(&art, "rust", "cognitive", 5.0)
            .expect("rust clears the sample floor");
        assert!(
            (cp.p - 0.5).abs() < 1e-9,
            "cognitive 5.0 is the ramp median"
        );
        assert!((anchored_score(1.0, cp.p) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn corpus_percentile_absent_below_sample_floor() {
        // Same breakpoints, but pooled below MIN_LANG_SAMPLE ⇒ no lens.
        let art = ramp_artifact("rust", 42);
        assert!(crate::calibration::percentile(&art, "rust", "cognitive", 5.0).is_none());
    }

    /// Insert a commit, its file touches, and one complexity row per file.
    fn seed(db: &FactsDb) {
        for rev in ["c1", "c2", "c3"] {
            db.conn()
                .execute(
                    "INSERT INTO commits (rev, author_email, author_name, committer_email, \
                     canonical_author, date, committer_date, message, is_merge, parent_count) \
                     VALUES (?, 'a@b.com', 'A', 'a@b.com', 'A', TIMESTAMPTZ '2026-01-01', \
                     TIMESTAMPTZ '2026-01-01', 'm', false, 1)",
                    duckdb::params![rev],
                )
                .expect("insert commit");
        }
        // Revs: worst=3, b=2, c=1 → distinct PERCENT_RANK 1.0 / 0.5 / 0.0.
        db.conn()
            .execute(
                "INSERT INTO changes (rev, path, change_type) VALUES \
                 ('c1','src/worst.rs','modified'),('c2','src/worst.rs','modified'),('c3','src/worst.rs','modified'), \
                 ('c1','src/b.rs','modified'),('c2','src/b.rs','modified'), \
                 ('c1','src/c.rs','modified')",
                [],
            )
            .expect("insert changes");
        // Cognitive: worst=40 (repo max), b=20, c=10.
        db.conn()
            .execute(
                "INSERT INTO complexity_metrics (path, name, rev, cognitive) VALUES \
                 ('src/worst.rs','f','c1',40),('src/b.rs','f','c1',20),('src/c.rs','f','c1',10)",
                [],
            )
            .expect("insert complexity");
    }

    fn opts() -> Options {
        Options {
            repo_path: std::path::PathBuf::from("."),
            min_revs: 1,
            // Raw `changes` — no lineage view to materialise in this synthetic db.
            use_canonical_lineage: false,
            // Embedded world corpus (covers rust well above the floor) is the
            // active artifact; the pin asserts stability, not an absolute value.
            ..Options::default()
        }
    }

    /// THE STABILITY PIN: improving the repo's worst file (dropping its cognitive
    /// complexity, revisions untouched) must leave every OTHER file's anchored
    /// score bit-for-bit unchanged, while the legacy score — coupled through the
    /// repo-max normalisation — moves. This is the property the whole feature
    /// exists to provide.
    #[test]
    fn anchored_score_is_stable_when_worst_file_improves() {
        let db = FactsDb::new_in_memory().expect("db");
        seed(&db);
        let opts = opts();

        let before = run_hotspots_anchored(&db, &opts).expect("run before");
        let b_before = before.iter().find(|r| r.path == "src/b.rs").expect("b.rs");
        let b_anchored_before = b_before
            .hotspot_score_anchored
            .expect("b.rs is rust, covered by the embedded corpus");
        // Deterministic legacy value: pr_rev 0.5 · pr_cx 0.5 · (100−80)/4.
        assert!((b_before.hotspot_score - 1.25).abs() < 1e-9);
        assert!(b_anchored_before > 0.0, "the pin must be non-trivial");

        // Improve the worst file in place: cognitive 40 → 5, revisions unchanged.
        db.conn()
            .execute(
                "UPDATE complexity_metrics SET cognitive = 5 WHERE path = 'src/worst.rs'",
                [],
            )
            .expect("improve worst");

        let after = run_hotspots_anchored(&db, &opts).expect("run after");
        let b_after = after.iter().find(|r| r.path == "src/b.rs").expect("b.rs");

        // Anchored: b.rs cognitive (20) and revisions are untouched, and its
        // corpus percentile does not depend on the worst file → the score is
        // bit-identical (same inputs), so the difference is exactly zero.
        let b_anchored_after = b_after.hotspot_score_anchored.expect("still covered");
        assert!(
            (b_anchored_before - b_anchored_after).abs() < 1e-12,
            "anchored score must not move when an unrelated file improves \
             (before {b_anchored_before}, after {b_anchored_after})"
        );
        // Legacy: the repo max collapsed 40 → 20, so b.rs's norm_cx and pr_cx
        // both rose → pr_rev 0.5 · pr_cx 1.0 · (100−60)/4 = 5.0.
        assert!(
            (b_after.hotspot_score - 5.0).abs() < 1e-9,
            "legacy score MUST move (got {})",
            b_after.hotspot_score
        );
    }

    /// `run_hotspots` (the entry the SPA and the internal consumers use) must
    /// never populate the anchor — only `apply_hotspot_anchor` does — so those
    /// surfaces stay byte-identical whether or not a corpus is active.
    #[test]
    fn run_hotspots_never_populates_the_anchor() {
        let db = FactsDb::new_in_memory().expect("db");
        seed(&db);
        let plain = run_hotspots(&db, &opts()).expect("plain run");
        assert!(
            plain.iter().all(|r| r.hotspot_score_anchored.is_none()),
            "run_hotspots must leave the anchor absent"
        );
    }
}
