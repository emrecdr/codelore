//! `effort-exposure` analysis — what fraction of engineering activity
//! (commits, churn) flows into each code-health band (red / yellow / green).
//!
//! Answers the hero KPI question: "Are we spending most of our energy improving
//! healthy code, or fighting fires in the red zone?" A team with ≥50% of
//! commits in the red band is in reactive mode; a team with ≥70% in green is
//! proactively maintaining its healthiest files.
//!
//! ## Algorithm
//!
//! 1. Compute code health for every live file at HEAD via
//!    [`run_code_health_scoped`] with [`HealthScanCtx::head_default()`].
//! 2. Materialise a session-local `eh_bands_v1(path, band, sloc)` temp table
//!    from the health result, joining SLOC from `complexity_metrics`.
//! 3. Over the trailing window (`opts.window_days` days, anchored to the
//!    repo's last commit date — reproducible on old repos), compute per band:
//!    - `files` — distinct files in the band (live at HEAD).
//!    - `loc_share_pct` — percentage of total SLOC in the band.
//!    - `commit_share_pct` — percentage of window commits touching ≥1 file in
//!      the band. One commit touching files in multiple bands is counted once
//!      per band (percentages across bands can therefore sum > 100%).
//!    - `churn_share_pct` — percentage of window LOC churn (added + deleted)
//!      in the band.
//! 4. Wilson 95% CI on `commit_share` (k = commits touching band,
//!    n = total window commits) is appended per row.

use std::collections::HashMap;

use duckdb::params;

use crate::analyses::code_health::{CodeHealthRow, HealthScanCtx, run_code_health_scoped};
use crate::analyses::lineage;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// One row per code-health band in the trailing activity window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EffortExposureRow {
    /// Code-health band: `"red"`, `"yellow"`, or `"green"`.
    pub band: String,
    /// Distinct files live at HEAD that fall in this band.
    pub files: u32,
    /// Percentage of total SLOC (source lines of code) in this band.
    pub loc_share_pct: f64,
    /// Percentage of trailing-window commits that touched ≥1 file in this
    /// band. A commit touching files in multiple bands is counted once per
    /// band it touches, so percentages across bands can sum > 100%.
    pub commit_share_pct: f64,
    /// Percentage of trailing-window churn (lines added + deleted) in this
    /// band's files.
    pub churn_share_pct: f64,
    /// Wilson 95% CI lower bound for `commit_share_pct / 100`.
    pub commit_share_ci_low: f64,
    /// Wilson 95% CI upper bound for `commit_share_pct / 100`.
    pub commit_share_ci_high: f64,
}

const BANDS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE eh_bands_v1 (
        path TEXT NOT NULL,
        band TEXT NOT NULL,
        sloc BIGINT NOT NULL DEFAULT 0
    );
";

/// Wilson score 95% confidence interval for a proportion `k / n`.
///
/// Returns `(low, high)` in `[0.0, 1.0]`. Returns `(0.0, 0.0)` when `n = 0`
/// (undefined proportion). Edge cases `k = 0` and `k = n` are handled
/// correctly by the formula without special-casing.
///
/// Standard Wilson score interval (Wilson 1927); z = 1.96 for 95% coverage.
///
/// Parameters are `u32` (not `u64`) to avoid precision-loss on the f64
/// conversion — all realistic commit counts fit comfortably in 32 bits.
pub(crate) fn wilson_ci(k: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let k = f64::from(k);
    let n = f64::from(n);
    let z = 1.96_f64;
    let z2 = z * z;
    let p_hat = k / n;
    let denom = 1.0 + z2 / n;
    let centre = (p_hat + z2 / (2.0 * n)) / denom;
    let radius = (z / denom) * (p_hat * (1.0 - p_hat) / n + z2 / (4.0 * n * n)).sqrt();
    (
        f64::max(0.0, centre - radius),
        f64::min(1.0, centre + radius),
    )
}

/// Fetch per-file SLOC totals from `complexity_metrics` (HEAD snapshot).
///
/// `SUM` collapses multiple entity rows (functions / methods) per file into one
/// file-level SLOC value, matching the granularity of `eh_bands_v1`.
fn fetch_sloc_map(db: &FactsDb) -> Result<HashMap<String, i64>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT path, COALESCE(SUM(sloc), 0) FROM complexity_metrics GROUP BY path")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare sloc query: {e}")))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query sloc: {e}")))?;
    rows.collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect sloc: {e}")))
}

/// Create (or replace) the `eh_bands_v1` session-local temp table and populate
/// it from the code-health rows, joining SLOC values from `sloc_map`.
fn populate_bands_table(
    db: &FactsDb,
    health: &[CodeHealthRow],
    sloc_map: &HashMap<String, i64>,
) -> Result<()> {
    db.conn()
        .execute(BANDS_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create eh_bands_v1: {e}")))?;
    let mut ins = db
        .conn()
        .prepare("INSERT INTO eh_bands_v1 (path, band, sloc) VALUES (?, ?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare eh_bands_v1 insert: {e}")))?;
    for row in health {
        let sloc = sloc_map.get(&row.path).copied().unwrap_or(0);
        ins.execute(params![row.path, row.band, sloc])
            .map_err(|e| CodeLoreError::Analysis(format!("insert eh_bands_v1 row: {e}")))?;
    }
    Ok(())
}

/// Run the effort-exposure analysis.
///
/// Returns one row per code-health band (only bands that have ≥1 file are
/// included; an entirely green repo returns a single `"green"` row). Bands are
/// ordered red → yellow → green.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failure.
pub fn run_effort_exposure(db: &FactsDb, opts: &Options) -> Result<Vec<EffortExposureRow>> {
    // Step 1: compute per-file code-health bands at HEAD.
    let health = run_code_health_scoped(
        db,
        &opts.with_no_row_limit(),
        &HealthScanCtx::head_default(),
    )?;
    if health.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: fetch per-file SLOC and Step 3: materialise eh_bands_v1.
    // DDL is safe at analysis phase because TEMPORARY tables are session-local.
    let sloc_map = fetch_sloc_map(db)?;
    populate_bands_table(db, &health, &sloc_map)?;

    // Step 4: run the band-level aggregation over the trailing window.
    // `window_days` anchors to the repo's last commit date (not wall-clock)
    // so results are reproducible on archived repos.
    lineage::materialize_if_needed(db, opts)?;
    let src = lineage::source_table(opts);
    let wd = opts.window_days;

    // Aggregation is performed in separate CTEs (band_files, band_commits,
    // band_churn) to avoid the cross-product inflation that arises when
    // joining eh_bands (one row per file) directly against the touch results
    // (one row per rev×path) in the outer SELECT.
    let sql = format!("
        WITH win AS (
            SELECT rev FROM commits
            WHERE date >= (SELECT MAX(date) FROM commits) - INTERVAL '{wd} days'
        ),
        band_files AS (
            SELECT band,
                   COUNT(*)            AS files,
                   COALESCE(SUM(sloc), 0) AS band_sloc
            FROM eh_bands_v1
            GROUP BY band
        ),
        band_commits AS (
            SELECT b.band,
                   COUNT(DISTINCT c.rev) AS n_commits
            FROM {src} c
            INNER JOIN win          USING (rev)
            INNER JOIN eh_bands_v1 b ON b.path = c.path
            GROUP BY b.band
        ),
        band_churn AS (
            SELECT b.band,
                   COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS churn
            FROM {src} c
            INNER JOIN win          USING (rev)
            INNER JOIN eh_bands_v1 b ON b.path = c.path
            GROUP BY b.band
        ),
        total_sloc    AS (SELECT COALESCE(SUM(sloc), 0) AS v FROM eh_bands_v1),
        total_commits AS (SELECT COUNT(*)               AS v FROM win),
        total_churn   AS (
            SELECT COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS v
            FROM {src} c INNER JOIN win USING (rev)
        )
        SELECT
            bf.band,
            bf.files::INTEGER                                                              AS files,
            100.0 * bf.band_sloc           / NULLIF((SELECT v FROM total_sloc),    0)     AS loc_share_pct,
            100.0 * COALESCE(bc.n_commits, 0) / NULLIF((SELECT v FROM total_commits), 0)  AS commit_share_pct,
            100.0 * COALESCE(bch.churn,    0) / NULLIF((SELECT v FROM total_churn),  0)   AS churn_share_pct,
            COALESCE(bc.n_commits, 0)                                                      AS k_commits,
            (SELECT v FROM total_commits)                                                   AS n_commits
        FROM band_files bf
        LEFT JOIN band_commits bc  ON bc.band  = bf.band
        LEFT JOIN band_churn   bch ON bch.band = bf.band
        ORDER BY CASE bf.band WHEN 'red' THEN 1 WHEN 'yellow' THEN 2 ELSE 3 END
    ");

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare effort-exposure: {e}")))?;

    let raw = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,                     // band
                r.get::<_, u32>(1)?,                        // files
                r.get::<_, Option<f64>>(2)?.unwrap_or(0.0), // loc_share_pct
                r.get::<_, Option<f64>>(3)?.unwrap_or(0.0), // commit_share_pct
                r.get::<_, Option<f64>>(4)?.unwrap_or(0.0), // churn_share_pct
                r.get::<_, i64>(5)?,                        // k_commits
                r.get::<_, i64>(6)?,                        // n_commits
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query effort-exposure: {e}")))?;

    let mut out = Vec::new();
    for r in raw {
        let (band, files, loc_share_pct, commit_share_pct, churn_share_pct, k, n) =
            r.map_err(|e| CodeLoreError::Analysis(format!("collect effort-exposure: {e}")))?;
        let (commit_share_ci_low, commit_share_ci_high) =
            wilson_ci(u32::try_from(k).unwrap_or(0), u32::try_from(n).unwrap_or(0));
        out.push(EffortExposureRow {
            band,
            files,
            loc_share_pct,
            commit_share_pct,
            churn_share_pct,
            commit_share_ci_low,
            commit_share_ci_high,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{EffortExposureRow, populate_bands_table, wilson_ci};
    use crate::analyses::code_health::CodeHealthRow;
    use crate::quality_gates::evaluate_effort_exposure_rows;
    use std::collections::HashMap;

    #[test]
    fn wilson_ci_k_zero() {
        let (lo, hi) = wilson_ci(0, 100);
        assert!(lo >= 0.0, "lo must be ≥ 0: {lo}");
        assert!(
            hi > 0.0 && hi < 0.05,
            "hi for k=0/n=100 should be small: {hi}"
        );
    }

    #[test]
    fn wilson_ci_k_equals_n() {
        let (lo, hi) = wilson_ci(100, 100);
        assert!(lo > 0.95 && lo <= 1.0, "lo for k=n should be near 1: {lo}");
        assert!(
            (hi - 1.0).abs() < 1e-9,
            "hi for k=n should be exactly 1: {hi}"
        );
    }

    #[test]
    fn wilson_ci_half() {
        let (lo, hi) = wilson_ci(50, 100);
        // p_hat = 0.5; Wilson CI for 0.5 with n=100, z=1.96 ≈ [0.401, 0.599]
        assert!(lo > 0.39 && lo < 0.50, "lo for k=50/n=100: {lo}");
        assert!(hi > 0.50 && hi < 0.61, "hi for k=50/n=100: {hi}");
        assert!(lo < hi, "interval must be non-empty");
    }

    #[test]
    fn wilson_ci_n_zero_returns_zeros() {
        let (lo, hi) = wilson_ci(0, 0);
        assert_eq!((lo, hi), (0.0, 0.0));
    }

    #[test]
    fn wilson_ci_interval_contains_p_hat() {
        let k = 30_u32;
        let n = 100_u32;
        let (lo, hi) = wilson_ci(k, n);
        let p_hat = f64::from(k) / f64::from(n);
        assert!(
            lo <= p_hat && p_hat <= hi,
            "interval must contain p_hat={p_hat}: [{lo}, {hi}]"
        );
    }

    /// Verifies that `populate_bands_table` correctly stores red-band entries
    /// and that the aggregation SQL produces a `red` row with a positive
    /// `churn_share_pct` when red-band files have window activity.
    ///
    /// Uses a synthetic `CodeHealthRow` set spanning all three bands so the
    /// red-band code path is covered without requiring `run_code_health_scoped`
    /// to produce a red file (`biomarker_repo` only has yellow/green files).
    #[test]
    #[allow(clippy::too_many_lines)]
    fn red_band_aggregation_path_is_covered() {
        use crate::facts::FactsDb;

        let db = FactsDb::new_in_memory().expect("in-memory db");

        // Seed one commit inside the window.
        db.conn()
            .execute(
                "INSERT INTO commits (rev, author_email, author_name, \
                 committer_email, canonical_author, date, committer_date, \
                 message, is_merge, parent_count) \
                 VALUES ('abc1', 'a@b.com', 'A', 'a@b.com', 'A', \
                         TIMESTAMPTZ '2026-01-01', TIMESTAMPTZ '2026-01-01', \
                         'init', false, 1)",
                [],
            )
            .expect("insert commit");

        // Two changes: one in a red file, one in a green file.
        db.conn()
            .execute(
                "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) \
                 VALUES ('abc1', 'src/bad.rs',  'modified', 80, 20), \
                        ('abc1', 'src/good.rs', 'modified', 10,  5)",
                [],
            )
            .expect("insert changes");

        // Seed SLOC in complexity_metrics (one function row per file is enough).
        db.conn()
            .execute(
                "INSERT INTO complexity_metrics \
                 (path, name, rev, sloc) \
                 VALUES ('src/bad.rs',  'main', 'abc1', 500), \
                        ('src/good.rs', 'main', 'abc1', 200), \
                        ('src/ok.rs',   'main', 'abc1', 100)",
                [],
            )
            .expect("insert complexity");

        // Synthetic health rows: red + yellow + green (all three bands).
        let health = vec![
            CodeHealthRow {
                path: "src/bad.rs".into(),
                band: "red".into(),
                cognitive: 42.0,
                score: 20.0,
                structural_risk: 0.9,
                percentile: 0.95,
            },
            CodeHealthRow {
                path: "src/ok.rs".into(),
                band: "yellow".into(),
                cognitive: 10.0,
                score: 55.0,
                structural_risk: 0.4,
                percentile: 0.50,
            },
            CodeHealthRow {
                path: "src/good.rs".into(),
                band: "green".into(),
                cognitive: 2.0,
                score: 85.0,
                structural_risk: 0.1,
                percentile: 0.10,
            },
        ];

        let sloc_map: HashMap<String, i64> = HashMap::from([
            ("src/bad.rs".into(), 500),
            ("src/ok.rs".into(), 100),
            ("src/good.rs".into(), 200),
        ]);

        populate_bands_table(&db, &health, &sloc_map).expect("populate_bands_table");

        // Run the same aggregation SQL that run_effort_exposure uses, with
        // src = "changes" (no lineage) and a window large enough to include
        // the single commit seeded above.
        let sql = "
            WITH win AS (
                SELECT rev FROM commits
                WHERE date >= (SELECT MAX(date) FROM commits) - INTERVAL '3650 days'
            ),
            band_files AS (
                SELECT band, COUNT(*) AS files,
                       COALESCE(SUM(sloc), 0) AS band_sloc
                FROM eh_bands_v1 GROUP BY band
            ),
            band_commits AS (
                SELECT b.band, COUNT(DISTINCT c.rev) AS n_commits
                FROM changes c
                INNER JOIN win          USING (rev)
                INNER JOIN eh_bands_v1 b ON b.path = c.path
                GROUP BY b.band
            ),
            band_churn AS (
                SELECT b.band,
                       COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS churn
                FROM changes c
                INNER JOIN win          USING (rev)
                INNER JOIN eh_bands_v1 b ON b.path = c.path
                GROUP BY b.band
            ),
            total_sloc    AS (SELECT COALESCE(SUM(sloc), 0) AS v FROM eh_bands_v1),
            total_commits AS (SELECT COUNT(*) AS v FROM win),
            total_churn   AS (
                SELECT COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS v
                FROM changes c INNER JOIN win USING (rev)
            )
            SELECT bf.band,
                   100.0 * COALESCE(bc.n_commits, 0)
                         / NULLIF((SELECT v FROM total_commits), 0) AS commit_share_pct,
                   100.0 * COALESCE(bch.churn, 0)
                         / NULLIF((SELECT v FROM total_churn), 0)   AS churn_share_pct
            FROM band_files bf
            LEFT JOIN band_commits bc  ON bc.band  = bf.band
            LEFT JOIN band_churn   bch ON bch.band = bf.band
            ORDER BY CASE bf.band WHEN 'red' THEN 1 WHEN 'yellow' THEN 2 ELSE 3 END
        ";
        let mut stmt = db.conn().prepare(sql).expect("prepare agg sql");
        let rows: Vec<(String, f64, f64)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                ))
            })
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");

        // All three bands must appear (yellow has no changes, but it's in
        // eh_bands_v1 so band_files includes it; band_commits/churn will be 0).
        let bands: Vec<&str> = rows.iter().map(|(b, _, _)| b.as_str()).collect();
        assert!(
            bands.contains(&"red"),
            "red band must be present; got {bands:?}"
        );
        assert!(bands.contains(&"yellow"), "yellow band must be present");
        assert!(bands.contains(&"green"), "green band must be present");

        // Red band has 100 lines of churn (80+20), green has 15 (10+5).
        // Total churn = 115; red share ≈ 86.9%.
        let red = rows.iter().find(|(b, _, _)| b == "red").expect("red row");
        assert!(
            red.2 > 50.0,
            "red churn_share_pct should be majority (≈87%): {}",
            red.2
        );

        // Yellow has no window activity — its churn share must be 0.
        let yellow = rows
            .iter()
            .find(|(b, _, _)| b == "yellow")
            .expect("yellow row");
        assert!(
            yellow.2.abs() < f64::EPSILON,
            "yellow has no churn in window"
        );

        // ── Gate fail-path closure ───────────────────────────────────────────
        // Convert the SQL tuples into the typed rows the gate evaluator expects,
        // then prove the full chain: synthetic red data → aggregation SQL →
        // gate fail verdict.
        let red_churn_share = red.2;
        let ee_rows: Vec<EffortExposureRow> = rows
            .iter()
            .map(|(b, commit_share_pct, churn_share_pct)| EffortExposureRow {
                band: b.clone(),
                files: 1,
                loc_share_pct: 0.0,
                commit_share_pct: *commit_share_pct,
                churn_share_pct: *churn_share_pct,
                commit_share_ci_low: 0.0,
                commit_share_ci_high: 1.0,
            })
            .collect();

        // threshold = 0.0: any positive red churn share must fire exactly one
        // violation naming the correct gate key and carrying the actual value.
        let violations = evaluate_effort_exposure_rows(0.0, &ee_rows);
        assert_eq!(
            violations.len(),
            1,
            "threshold=0.0 must trigger one violation; got {violations:?}"
        );
        assert_eq!(
            violations[0].gate, "max_red_effort_pct",
            "gate name must match the threshold key"
        );
        let reported_actual: f64 = violations[0]
            .actual
            .parse()
            .expect("actual field must be a parseable f64");
        assert!(
            (reported_actual - red_churn_share).abs() < 0.01,
            "reported actual ({reported_actual:.2}) must match red churn share ({red_churn_share:.2})"
        );
    }
}
