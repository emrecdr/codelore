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
//!
//!    `commit_share_pct` and `churn_share_pct` are shares of *banded* activity:
//!    both denominators (`total_commits`, `total_churn`) join `eh_bands_v1`, so
//!    commits and churn on unscored paths — lockfiles, generated code, docs,
//!    anything without a code-health row — are excluded from denominator as
//!    well as numerator. Otherwise those paths would inflate the denominator
//!    alone, understating every share and making the `max_red_effort_pct` gate
//!    monotonically more permissive on repositories with heavy non-code churn.
//!    `loc_share_pct` is already banded (SLOC comes from `eh_bands_v1`).
//! 4. Wilson 95% CI on `commit_share` (k = commits touching band,
//!    n = banded window commits) is appended per row.

use std::collections::{HashMap, HashSet};

use duckdb::params;

use crate::analyses::code_health::{CodeHealthRow, HealthScanCtx, run_code_health_scoped};
use crate::analyses::delta_health::{Outcome, classify, outcome_for, run_function_metrics};
use crate::analyses::lineage;
use crate::facts::FactsDb;
use crate::repo::Repo;
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
    /// Percentage of trailing-window commits — counting only commits that
    /// touched a code-health-banded file — that touched ≥1 file in this band.
    /// The denominator is banded (scorable) commits, not every window commit:
    /// a commit touching only unscored paths (lockfiles, generated code, docs)
    /// is in neither numerator nor denominator, so the share is of scorable
    /// engineering activity. A commit touching files in multiple bands is
    /// counted once per band it touches, so percentages across bands can sum
    /// > 100%.
    pub commit_share_pct: f64,
    /// Percentage of trailing-window churn (lines added + deleted) in this
    /// band's files, as a share of churn on code-health-banded files only —
    /// not raw repo churn. Churn on unscored paths (lockfiles, generated code,
    /// docs) is excluded from the denominator, matching the band-restricted
    /// numerator; counting it would understate every band's share and let the
    /// `max_red_effort_pct` gate drift more permissive as non-code churn grows.
    pub churn_share_pct: f64,
    /// Wilson 95% CI lower bound for `commit_share_pct / 100`.
    pub commit_share_ci_low: f64,
    /// Wilson 95% CI upper bound for `commit_share_pct / 100`.
    pub commit_share_ci_high: f64,
    /// Share of *total* window churn (same denominator as
    /// [`churn_share_pct`](Self::churn_share_pct)) that landed in this band's
    /// files whose own health IMPROVED over the window. `None` unless the
    /// decomposition was computed ([`run_effort_exposure_decomposed`], which
    /// needs repository access); populated only for the `red` band, where the
    /// improving-churn gate exemption reads it. Additive: absent from the
    /// serialized output when `None`, so a run without the decomposition is
    /// byte-identical to before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub churn_share_improving_pct: Option<f64>,
    /// Complement of [`churn_share_improving_pct`](Self::churn_share_improving_pct):
    /// the share of total window churn in this band's files whose health did NOT
    /// improve over the window. For the `red` band the two sum to
    /// [`churn_share_pct`](Self::churn_share_pct). `None` under the same
    /// conditions as its sibling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub churn_share_degrading_pct: Option<f64>,
}

const BANDS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE eh_bands_v1 (
        path TEXT NOT NULL,
        band TEXT NOT NULL,
        sloc BIGINT NOT NULL DEFAULT 0
    );
";

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
    run_effort_exposure_with_health(db, opts, &health)
}

/// [`run_effort_exposure`] with the code-health rows supplied by the caller.
///
/// `health` must be the HEAD-scope, unlimited-row health result
/// ([`run_code_health_scoped`] with [`HealthScanCtx::head_default()`] and
/// `with_no_row_limit()`); callers that already hold those rows — the
/// quality-gate path computes them for `code_health_min` — avoid re-running
/// the heaviest analysis in the pipeline.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failure.
pub fn run_effort_exposure_with_health(
    db: &FactsDb,
    opts: &Options,
    health: &[CodeHealthRow],
) -> Result<Vec<EffortExposureRow>> {
    if health.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: fetch per-file SLOC and Step 3: materialise eh_bands_v1.
    // DDL is safe at analysis phase because TEMPORARY tables are session-local.
    let sloc_map = fetch_sloc_map(db)?;
    populate_bands_table(db, health, &sloc_map)?;

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
        -- total_commits/total_churn join eh_bands_v1 so their denominators cover
        -- the same banded population as the numerators (see this module's docs).
        total_commits AS (
            SELECT COUNT(DISTINCT c.rev) AS v
            FROM {src} c
            INNER JOIN win          USING (rev)
            INNER JOIN eh_bands_v1 b ON b.path = c.path
        ),
        total_churn   AS (
            SELECT COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS v
            FROM {src} c
            INNER JOIN win          USING (rev)
            INNER JOIN eh_bands_v1 b ON b.path = c.path
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
            crate::stats::wilson_ci(u32::try_from(k).unwrap_or(0), u32::try_from(n).unwrap_or(0));
        out.push(EffortExposureRow {
            band,
            files,
            loc_share_pct,
            commit_share_pct,
            churn_share_pct,
            commit_share_ci_low,
            commit_share_ci_high,
            // The improving/degrading split is an opt-in enrichment computed by
            // run_effort_exposure_decomposed (which needs repo access); the base
            // path leaves it absent, keeping existing output byte-identical.
            churn_share_improving_pct: None,
            churn_share_degrading_pct: None,
        });
    }
    Ok(out)
}

/// Run effort-exposure and enrich the `red` band row with its improving vs
/// degrading window-churn decomposition — the signal the
/// `red_effort_exempt_improving` gate exemption reads.
///
/// `health` must be the HEAD-scope, unlimited-row code-health rows (as for
/// [`run_effort_exposure_with_health`]); the gate path already holds them.
///
/// ## What "improving" means (and its limits)
///
/// A red file is classed **improving** when the *net* absolute-risk movement of
/// its functions between the window-start revision and HEAD is favourable —
/// good weight strictly exceeds bad weight, using the same fixed LOC /
/// cyclomatic risk bands as `delta-health` ([`classify`] / [`outcome_for`]).
/// Every other red file (net-degrading, net-neutral, or with no analyzable
/// function change) is class **degrading**, the conservative default: the
/// exemption fires only on demonstrable improvement.
///
/// The window-start baseline is the last commit strictly before the trailing
/// window opened; a red file with no such baseline (added within the window, or
/// renamed into its HEAD path during it — pairing keys on the HEAD path, so a
/// rename reads as all-new) has all its functions treated as *added* and is
/// judged by their absolute risk alone. A file both refactored AND degraded
/// within one window is classified by the NET of those movements, so a large
/// regression can mask a smaller concurrent cleanup (and vice-versa). The split
/// is a directional gate signal, not an exact per-commit attribution.
///
/// The two shares reconcile: for the red band,
/// `churn_share_improving_pct + churn_share_degrading_pct == churn_share_pct`
/// (both over the same total-window-churn denominator).
///
/// ## Cost
///
/// One extra pass beyond the base analysis: the window-start source of each red
/// file that saw window churn is parsed for complexity (via
/// [`crate::repo::Repo::read_blob_at`] + tree-sitter), scoped to those red files
/// only — never a second full-tree health scan. Repos with few red files pay
/// almost nothing.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failure.
pub fn run_effort_exposure_decomposed<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    health: &[CodeHealthRow],
) -> Result<Vec<EffortExposureRow>> {
    // Base rows also materialize eh_bands_v1 as a side effect, which the
    // per-red-file churn query below reuses.
    let mut rows = run_effort_exposure_with_health(db, opts, health)?;
    let Some(red_idx) = rows.iter().position(|r| r.band == "red") else {
        return Ok(rows); // No red files ⇒ no exemption-relevant split.
    };
    let red_paths: HashSet<String> = health
        .iter()
        .filter(|r| r.band == "red")
        .map(|r| r.path.clone())
        .collect();
    let (improving_pct, degrading_pct) = decompose_red_churn(db, repo, opts, &red_paths)?;
    rows[red_idx].churn_share_improving_pct = Some(improving_pct);
    rows[red_idx].churn_share_degrading_pct = Some(degrading_pct);
    Ok(rows)
}

/// [`run_effort_exposure_decomposed`] computing the HEAD code-health scan
/// itself — the standalone `codelore analyze effort-exposure` entry point,
/// where a repository handle is available and the decomposition columns should
/// carry real values.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failure.
pub fn run_effort_exposure_decomposed_scan<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
) -> Result<Vec<EffortExposureRow>> {
    let health = run_code_health_scoped(
        db,
        &opts.with_no_row_limit(),
        &HealthScanCtx::head_default(),
    )?;
    run_effort_exposure_decomposed(db, repo, opts, &health)
}

/// Total window churn (added + deleted LOC) across every band's files — the
/// shared decomposition denominator. Restricted to `eh_bands_v1` (code-health-
/// banded files) so it stays byte-identical to the `total_churn` CTE in
/// [`run_effort_exposure_with_health`]; the reconciliation
/// `improving_pct + degrading_pct == churn_share_pct` holds only when both
/// denominators cover the same banded population.
fn window_total_churn(db: &FactsDb, src: &str, wd: u32) -> Result<f64> {
    let sql = format!(
        "SELECT COALESCE(SUM(c.loc_added + c.loc_deleted), 0)::DOUBLE
         FROM {src} c
         INNER JOIN eh_bands_v1 b ON b.path = c.path
         WHERE c.rev IN (
             SELECT rev FROM commits
             WHERE date >= (SELECT MAX(date) FROM commits) - INTERVAL '{wd} days'
         )"
    );
    db.conn()
        .query_row(&sql, [], |r| r.get::<_, f64>(0))
        .map_err(|e| CodeLoreError::Analysis(format!("query window total churn: {e}")))
}

/// Per-red-file window churn, keyed by path. Joins the trailing window to
/// `eh_bands_v1` (materialized by [`run_effort_exposure_with_health`]) filtered
/// to the `red` band, so the summed value equals the red-band `churn` the base
/// analysis reports.
fn red_window_churn(db: &FactsDb, src: &str, wd: u32) -> Result<HashMap<String, f64>> {
    let sql = format!(
        "SELECT c.path, COALESCE(SUM(c.loc_added + c.loc_deleted), 0)::DOUBLE AS churn
         FROM {src} c
         INNER JOIN eh_bands_v1 b ON b.path = c.path AND b.band = 'red'
         WHERE c.rev IN (
             SELECT rev FROM commits
             WHERE date >= (SELECT MAX(date) FROM commits) - INTERVAL '{wd} days'
         )
         GROUP BY c.path"
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare red window churn: {e}")))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query red window churn: {e}")))?;
    rows.collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect red window churn: {e}")))
}

/// The revision representing a file's state entering the trailing window: the
/// newest commit strictly *before* the window opened. `None` when the window
/// spans all history (no pre-window commit) — every file is then judged as
/// newly-added, which is also the "history shallower than the window" signal the
/// `[new_code]` gate skips on.
///
/// Shared with the `[new_code]` gate so both views anchor the window start on
/// the same rev; the caller passes the window length (the effort view uses
/// `opts.window_days`, the new-code gate its own `[new_code] window_days`).
pub fn window_start_rev(db: &FactsDb, wd: u32) -> Result<Option<String>> {
    let sql = format!(
        "SELECT rev FROM commits
         WHERE date < (SELECT MAX(date) FROM commits) - INTERVAL '{wd} days'
         ORDER BY date DESC, rev DESC
         LIMIT 1"
    );
    match db.conn().query_row(&sql, [], |r| r.get::<_, String>(0)) {
        Ok(rev) => Ok(Some(rev)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(CodeLoreError::Analysis(format!(
            "query window-start rev: {e}"
        ))),
    }
}

/// One function's absolute risk drivers at a revision: worst-case per bare name.
type FnMetrics = HashMap<String, (u32, f64)>;

/// HEAD per-function `(loc, cyclomatic)` for the given path set, indexed by path
/// then bare function name. Reuses `delta-health`'s [`run_function_metrics`],
/// which already collapses each `(path, bare-name)` to its worst-case metrics
/// and keeps only real functions / methods. Shared by the red-band effort
/// decomposition and the `[new_code]` touched-band net-movement scan.
fn head_function_metrics(
    db: &FactsDb,
    paths: &HashSet<String, impl std::hash::BuildHasher>,
) -> Result<HashMap<String, FnMetrics>> {
    let mut out: HashMap<String, FnMetrics> = HashMap::new();
    for row in run_function_metrics(db)? {
        if paths.contains(&row.path) {
            out.entry(row.path)
                .or_default()
                .insert(row.name, (row.loc, row.cyclomatic));
        }
    }
    Ok(out)
}

/// Per-file net `delta-health` movement over the trailing window for an
/// arbitrary set of live-at-HEAD `paths`, sharing the improving-churn
/// exemption's window-start machinery: HEAD function metrics from the fact
/// store ([`head_function_metrics`]), the window-start baseline parsed per file
/// ([`base_function_metrics`]), and the [`net_movement`] classification.
///
/// `window_start` is the caller-resolved window-start rev ([`window_start_rev`]);
/// `None` treats every file as all-new (each function judged by its HEAD risk
/// alone). This is the signal the `[new_code]` touched band reads — it performs
/// no second full-tree health scan, only the same scoped per-file at-rev parse
/// the effort decomposition already uses, here over the touched-not-born set
/// instead of the red set. A path with no analyzable functions at either rev
/// (docs, config, unsupported languages) nets zero.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failure
/// fetching the HEAD function metrics.
pub fn window_net_movement<R: Repo>(
    db: &FactsDb,
    repo: &R,
    paths: &HashSet<String, impl std::hash::BuildHasher>,
    window_start: Option<&str>,
) -> Result<HashMap<String, f64>> {
    let head_fns = head_function_metrics(db, paths)?;
    let empty: FnMetrics = HashMap::new();
    let mut out = HashMap::with_capacity(paths.len());
    for path in paths {
        let head = head_fns.get(path).unwrap_or(&empty);
        let base =
            window_start.map_or_else(FnMetrics::new, |rev| base_function_metrics(repo, rev, path));
        out.insert(path.clone(), net_movement(head, &base));
    }
    Ok(out)
}

/// Parse `path` at `rev` for its per-function `(loc, cyclomatic)` — the
/// window-start baseline. Scoped to the single red file; a per-file blob-read,
/// size-cap, or parse failure yields an empty baseline (every function reads as
/// added) rather than aborting the analysis, matching the at-rev scan's
/// resilience contract.
fn base_function_metrics<R: Repo>(repo: &R, rev: &str, path: &str) -> FnMetrics {
    use crate::complexity::{Tier1Language, compute_for_file};

    let mut out: FnMetrics = HashMap::new();
    let Some(lang) = Tier1Language::from_path(path) else {
        return out;
    };
    let source = match repo.read_blob_at(rev, path) {
        Ok(Some(b)) => b,
        Ok(None) => return out,
        Err(e) => {
            tracing::warn!("effort-exposure: blob read failed for {path} at {rev}: {e}");
            return out;
        }
    };
    if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
        return out;
    }
    let entities = match compute_for_file(std::path::Path::new(path), source, lang) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("effort-exposure: parse error {path} at {rev}: {e}");
            return out;
        }
    };
    for e in entities {
        if e.kind == "function" || e.kind == "method" {
            // Same-named functions in one file collapse to worst-case, mirroring
            // run_function_metrics' MAX rollup on the HEAD side.
            let entry = out.entry(e.name).or_insert((0, 0.0));
            entry.0 = entry.0.max(e.loc);
            entry.1 = entry.1.max(e.cyclomatic);
        }
    }
    out
}

/// Net `delta-health` movement of a file between its window-start `base` and
/// `head` function metrics: good LOC-weight minus bad LOC-weight, classified
/// with the same fixed risk bands ([`classify`] / [`outcome_for`]) the whole
/// effort decomposition uses. Positive ⇒ net improvement, negative ⇒ net
/// degradation, exactly zero ⇒ no function crossed a risk band (untouched
/// functions and sub-band edits — the typo-fix case — contribute nothing, so
/// the banded classification is itself the noise filter).
///
/// This is the single per-file signal two gates share: the improving-churn
/// effort exemption asks whether it is strictly positive ([`file_improved`]);
/// the `[new_code]` touched band asks whether it is non-negative. Neither
/// applies the delta-health red-file weight multiplier — movement stays in raw
/// LOC weight, matching the effort decomposition's reported numbers.
pub(crate) fn net_movement(head: &FnMetrics, base: &FnMetrics) -> f64 {
    let mut good_w = 0.0_f64;
    let mut bad_w = 0.0_f64;
    let names: HashSet<&String> = head.keys().chain(base.keys()).collect();
    for name in names {
        let b = base.get(name);
        let h = head.get(name);
        // Identical rows are untouched functions — no movement.
        if let (Some(b), Some(h)) = (b, h)
            && b == h
        {
            continue;
        }
        let before = b.map(|&(loc, cyc)| classify(loc, cyc, false));
        let after = h.map(|&(loc, cyc)| classify(loc, cyc, false));
        let weight = f64::from(h.or(b).map_or(0, |&(loc, _)| loc));
        match outcome_for(before, after) {
            Outcome::Good => good_w += weight,
            Outcome::Bad => bad_w += weight,
            Outcome::Neutral => {}
        }
    }
    good_w - bad_w
}

/// Whether a file's functions net-improved between `base` (window-start) and
/// `head` — [`net_movement`] strictly positive (good weight dominates bad). The
/// conservative test that leaves neutral and net-degrading files degrading.
fn file_improved(head: &FnMetrics, base: &FnMetrics) -> bool {
    net_movement(head, base) > 0.0
}

/// Split the red band's window churn into `(improving_pct, degrading_pct)` of
/// total window churn. `red_paths` is the set of red-band files at HEAD.
fn decompose_red_churn<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    red_paths: &HashSet<String>,
) -> Result<(f64, f64)> {
    let src = lineage::source_table(opts);
    let wd = opts.window_days;

    let total_churn = window_total_churn(db, src, wd)?;
    if total_churn <= 0.0 {
        return Ok((0.0, 0.0)); // No window churn ⇒ no shares to split.
    }
    let per_file = red_window_churn(db, src, wd)?;
    if per_file.is_empty() {
        return Ok((0.0, 0.0)); // Red files exist but saw no window churn.
    }

    let window_start = window_start_rev(db, wd)?;
    let head_fns = head_function_metrics(db, red_paths)?;
    let empty_fns: FnMetrics = HashMap::new();

    let mut improving_churn = 0.0_f64;
    let mut red_churn = 0.0_f64;
    for (path, churn) in &per_file {
        red_churn += churn;
        let head = head_fns.get(path).unwrap_or(&empty_fns);
        let base = window_start
            .as_deref()
            .map_or_else(FnMetrics::new, |rev| base_function_metrics(repo, rev, path));
        if file_improved(head, &base) {
            improving_churn += churn;
        }
    }

    let improving_pct = 100.0 * improving_churn / total_churn;
    // Degrading is the red-band complement, so the two shares sum back to the
    // band's churn_share_pct exactly (same denominator, every red file in one
    // bucket).
    let degrading_pct = 100.0 * (red_churn - improving_churn) / total_churn;
    Ok((improving_pct, degrading_pct))
}

#[cfg(test)]
mod tests {
    use super::{EffortExposureRow, populate_bands_table};
    use crate::analyses::code_health::CodeHealthRow;
    use crate::quality_gates::evaluate_effort_exposure_rows;
    use std::collections::HashMap;

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
                corpus_percentile: None,
                beyond_corpus: false,
                corpus_percentile_ci_low: None,
                corpus_percentile_ci_high: None,
            },
            CodeHealthRow {
                path: "src/ok.rs".into(),
                band: "yellow".into(),
                cognitive: 10.0,
                score: 55.0,
                structural_risk: 0.4,
                percentile: 0.50,
                corpus_percentile: None,
                beyond_corpus: false,
                corpus_percentile_ci_low: None,
                corpus_percentile_ci_high: None,
            },
            CodeHealthRow {
                path: "src/good.rs".into(),
                band: "green".into(),
                cognitive: 2.0,
                score: 85.0,
                structural_risk: 0.1,
                percentile: 0.10,
                corpus_percentile: None,
                beyond_corpus: false,
                corpus_percentile_ci_low: None,
                corpus_percentile_ci_high: None,
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
            total_commits AS (
                SELECT COUNT(DISTINCT c.rev) AS v
                FROM changes c
                INNER JOIN win          USING (rev)
                INNER JOIN eh_bands_v1 b ON b.path = c.path
            ),
            total_churn   AS (
                SELECT COALESCE(SUM(c.loc_added + c.loc_deleted), 0) AS v
                FROM changes c
                INNER JOIN win          USING (rev)
                INNER JOIN eh_bands_v1 b ON b.path = c.path
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
                churn_share_improving_pct: None,
                churn_share_degrading_pct: None,
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

    /// Minimal `CodeHealthRow` for a path in a band — the effort-exposure SQL
    /// reads only `path` and `band` off each row; the scoring fields are inert.
    fn band_row(path: &str, band: &str) -> CodeHealthRow {
        CodeHealthRow {
            path: path.into(),
            band: band.into(),
            cognitive: 0.0,
            score: 0.0,
            structural_risk: 0.0,
            percentile: 0.0,
            corpus_percentile: None,
            beyond_corpus: false,
            corpus_percentile_ci_low: None,
            corpus_percentile_ci_high: None,
        }
    }

    /// Heavy churn on unscored paths (a lockfile and a markdown doc) must not
    /// dilute the band shares: with the band-restricted denominators the red
    /// band reads 60.0 % of *scorable* churn and trips a 30 % ceiling. Under an
    /// unrestricted (raw-repo) denominator the same red band read 6.0 % and
    /// slipped the ceiling — the regression this exercises. Runs the real
    /// `run_effort_exposure_with_health` SQL, not an inline copy.
    #[test]
    fn shares_exclude_churn_on_unbanded_paths() {
        use super::run_effort_exposure_with_health;
        use crate::Options;
        use crate::facts::FactsDb;

        let db = FactsDb::new_in_memory().expect("in-memory db");

        // Three in-window commits on one date (inside any positive window):
        //   c1 → a red code file, c2 → a green code file,
        //   c3 → a lockfile + a markdown doc (neither is code-health-scored).
        db.conn()
            .execute(
                "INSERT INTO commits (rev, author_email, author_name, \
                 committer_email, canonical_author, date, committer_date, \
                 message, is_merge, parent_count) VALUES \
                 ('c1','a@b.com','A','a@b.com','A',TIMESTAMPTZ '2026-01-01',TIMESTAMPTZ '2026-01-01','red',false,1), \
                 ('c2','a@b.com','A','a@b.com','A',TIMESTAMPTZ '2026-01-01',TIMESTAMPTZ '2026-01-01','green',false,1), \
                 ('c3','a@b.com','A','a@b.com','A',TIMESTAMPTZ '2026-01-01',TIMESTAMPTZ '2026-01-01','deps',false,1)",
                [],
            )
            .expect("insert commits");

        // Red 300 churn, green 200 churn (both banded); lockfile + markdown
        // 4500 churn (unbanded). Raw window churn 5000; banded churn 500.
        db.conn()
            .execute(
                "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) VALUES \
                 ('c1','src/bad.rs','modified',200,100), \
                 ('c2','src/good.rs','modified',150,50), \
                 ('c3','Cargo.lock','modified',3000,0), \
                 ('c3','README.md','modified',1500,0)",
                [],
            )
            .expect("insert changes");

        // SLOC only for the two scored code files (unscored paths have none).
        db.conn()
            .execute(
                "INSERT INTO complexity_metrics (path, name, rev, sloc) VALUES \
                 ('src/bad.rs','f','c1',500), \
                 ('src/good.rs','f','c2',200)",
                [],
            )
            .expect("insert complexity");

        // Health rows exist ONLY for the code files — the lockfile and the doc
        // are structurally absent from eh_bands_v1.
        let health = vec![
            band_row("src/bad.rs", "red"),
            band_row("src/good.rs", "green"),
        ];

        let opts = Options {
            // src = "changes"; the fix is independent of the source table.
            use_canonical_lineage: false,
            ..Options::default()
        };
        let rows = run_effort_exposure_with_health(&db, &opts, &health)
            .expect("run effort-exposure with synthetic health");

        let red = rows.iter().find(|r| r.band == "red").expect("red row");
        let green = rows.iter().find(|r| r.band == "green").expect("green row");

        // Band-consistent churn shares: 300/500 and 200/500 — NOT x/5000.
        assert!(
            (red.churn_share_pct - 60.0).abs() < 1e-6,
            "red churn share must be 60.0 (300 of 500 banded churn), got {}",
            red.churn_share_pct
        );
        assert!(
            (green.churn_share_pct - 40.0).abs() < 1e-6,
            "green churn share must be 40.0, got {}",
            green.churn_share_pct
        );
        assert!(
            (red.churn_share_pct + green.churn_share_pct - 100.0).abs() < 1e-6,
            "banded churn shares must sum to 100, got {}",
            red.churn_share_pct + green.churn_share_pct
        );

        // Commit shares carry the same fix: c3 (lockfile + doc only) is not a
        // banded commit, so the denominator is 2, not 3 → 50/50, not 33/33.
        assert!(
            (red.commit_share_pct - 50.0).abs() < 1e-6,
            "red commit share must be 50.0 (1 of 2 banded commits), got {}",
            red.commit_share_pct
        );
        assert!(
            (green.commit_share_pct - 50.0).abs() < 1e-6,
            "green commit share must be 50.0, got {}",
            green.commit_share_pct
        );

        // Wilson CI stays coherent — it must bracket the banded commit share.
        let red_share = red.commit_share_pct / 100.0;
        assert!(
            red.commit_share_ci_low <= red_share + 1e-9
                && red.commit_share_ci_high >= red_share - 1e-9,
            "Wilson CI [{}, {}] must contain commit share {red_share}",
            red.commit_share_ci_low,
            red.commit_share_ci_high
        );

        // The gate now fires: 60.0 % red churn exceeds a 30 % ceiling.
        let violations = evaluate_effort_exposure_rows(30.0, &rows);
        assert_eq!(violations.len(), 1, "red 60 % must trip the 30 % ceiling");
        assert_eq!(violations[0].gate, "max_red_effort_pct");
        let reported: f64 = violations[0].actual.parse().expect("actual is f64");
        assert!(
            (reported - 60.0).abs() < 1e-6,
            "violation must report the band-consistent 60.0, got {reported}"
        );
    }

    // ───────── improving-churn decomposition core (file_improved) ─────────

    use super::{FnMetrics, file_improved, net_movement};

    /// One function keyed by bare name, with LOC / cyclomatic. LOC 71+ or
    /// cyclomatic 11+ is High; 31+ / 6+ is Medium; below is Low — the fixed
    /// `delta-health` bands the classifier reuses.
    fn fns(entries: &[(&str, u32, f64)]) -> FnMetrics {
        entries
            .iter()
            .map(|&(n, loc, cyc)| (n.to_string(), (loc, cyc)))
            .collect()
    }

    #[test]
    fn net_movement_is_signed_weight_and_agrees_with_file_improved() {
        // A High-risk function (loc 120) shrinks to Low (loc 10): good weight is
        // the head LOC (10), no bad weight → net +10, and file_improved (net > 0)
        // agrees. The `[new_code]` touched band reads this same number and asks
        // net >= 0 instead.
        let base = fns(&[("f", 120, 3.0)]);
        let head = fns(&[("f", 10, 3.0)]);
        assert!((net_movement(&head, &base) - 10.0).abs() < f64::EPSILON);
        assert!(file_improved(&head, &base));

        // A Low→High degradation books the head LOC (120) as bad weight → net
        // -120, and file_improved is false.
        let base = fns(&[("f", 10, 3.0)]);
        let head = fns(&[("f", 120, 3.0)]);
        assert!((net_movement(&head, &base) + 120.0).abs() < f64::EPSILON);
        assert!(!file_improved(&head, &base));

        // A neutral-only change (Medium→Medium) and an untouched file both net
        // exactly zero — the typo-fix case the touched band lets pass.
        let base = fns(&[("f", 40, 3.0)]);
        let head = fns(&[("f", 50, 3.0)]);
        assert!(net_movement(&head, &base).abs() < f64::EPSILON);
        let same = fns(&[("f", 40, 3.0)]);
        assert!(net_movement(&same, &same.clone()).abs() < f64::EPSILON);
    }

    #[test]
    fn file_improved_true_when_a_function_is_refactored_down() {
        // A High-risk function (loc 120) shrinks to Low (loc 10): good weight,
        // no bad weight → the file net-improved.
        let base = fns(&[("f", 120, 3.0)]);
        let head = fns(&[("f", 10, 3.0)]);
        assert!(file_improved(&head, &base));
    }

    #[test]
    fn file_improved_false_when_a_function_degrades() {
        // Low (loc 10) grows to High (loc 120): bad weight dominates → degrading.
        let base = fns(&[("f", 10, 3.0)]);
        let head = fns(&[("f", 120, 3.0)]);
        assert!(!file_improved(&head, &base));
    }

    #[test]
    fn file_improved_false_on_neutral_only_change() {
        // Medium (loc 40) → Medium (loc 50): Neutral outcome, no good/bad weight
        // → not a demonstrable improvement, so it stays degrading (conservative).
        let base = fns(&[("f", 40, 3.0)]);
        let head = fns(&[("f", 50, 3.0)]);
        assert!(!file_improved(&head, &base));
    }

    #[test]
    fn file_improved_false_when_no_functions_changed() {
        // Identical rows = untouched functions = no movement → degrading bucket.
        let same = fns(&[("f", 40, 3.0), ("g", 5, 1.0)]);
        assert!(!file_improved(&same, &same.clone()));
    }

    #[test]
    fn file_improved_added_high_risk_function_is_degrading() {
        // No baseline (file born in the window): a High-risk added function is
        // bad weight → degrading.
        let base = FnMetrics::new();
        let head = fns(&[("monster", 200, 20.0)]);
        assert!(!file_improved(&head, &base));
    }

    #[test]
    fn file_improved_removing_a_high_risk_function_is_good() {
        // Deleting a High-risk function is good weight (base LOC) → improving.
        let base = fns(&[("monster", 200, 20.0)]);
        let head = FnMetrics::new();
        assert!(file_improved(&head, &base));
    }

    #[test]
    fn file_improved_net_of_concurrent_improve_and_degrade() {
        // One function cleaned up (High→Low, weight 10) and one worsened
        // (Low→High, weight 200) in the same window: bad weight dominates → the
        // NET is degrading, as documented (a large regression masks a small
        // cleanup).
        let base = fns(&[("clean", 120, 3.0), ("rot", 10, 1.0)]);
        let head = fns(&[("clean", 10, 3.0), ("rot", 200, 20.0)]);
        assert!(!file_improved(&head, &base));
        // Now a heavy cleanup outweighs a light regression: deleting a High-risk
        // function books its full base LOC (200) as good weight, dwarfing the
        // Low→Medium regression's 40 → net improving.
        let base2 = fns(&[("clean", 200, 20.0), ("rot", 10, 1.0)]);
        let head2 = fns(&[("rot", 40, 3.0)]);
        assert!(file_improved(&head2, &base2));
    }
}
