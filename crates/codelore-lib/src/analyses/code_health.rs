//! Code Health composite analysis.
//!
//! ```text
//! codehealth(entity) = 100 × (1
//!     - w_sr · structural_risk(biomarkers)
//!     - w_cn · normalize(churn_rate)
//!     - w_au · normalize(author_fragmentation_FV)
//! )
//!
//! defaults: w_sr = 0.50, w_cn = 0.30, w_au = 0.20
//! ```
//!
//! `structural_risk` is a weighted sum over the per-file biomarker table
//! (complex-method, large-method, shotgun-surgery, god-class, dry). Each
//! biomarker carries an intensity ∈ [0,1] computed as a per-language
//! `PERCENT_RANK` of the file's worst value for that smell; the per-smell
//! weights sum to 1.0, so `structural_risk` stays in [0,1] and spreads across the
//! file distribution. Smells absent for a file contribute 0, so co-occurrence
//! is implicit — a file flagged by more smells accumulates more weighted terms.
//! Coupling centrality (Fisher-significant pairs from `coupling::run_coupling`)
//! enters once, as the shotgun-surgery biomarker; it is deliberately not also a
//! separate behavioral term. Score range: [0, 100]; higher = healthier. Band
//! (red/yellow/green) is derived from `structural_risk` thresholds; percentile
//! is the per-language `PERCENT_RANK` of `structural_risk` (Alves, Ypma &
//! Visser 2010).
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "code-health" (composite signal developed in `CodeLore`; underlying
//! inputs cite cyclomatic complexity and LOC biomarker intensities (Campbell 2018),
//! churn (Nagappan & Ball 2005), ownership fragmentation (Mockus & Herbsleb 2002),
//! and coupling centrality (Tornhill 2018)).

use std::collections::HashMap;

use duckdb::params;

use crate::analyses::coupling::run_coupling;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeHealthRow {
    pub path: String,
    pub cognitive: f64,
    pub score: f64,           // 0..=100; higher = healthier
    pub structural_risk: f64, // 0..=1; higher = worse
    pub percentile: f64, // 0..=1; per-language self-relative rank of structural_risk (1 = riskiest)
    pub band: String,    // "red" | "yellow" | "green"
}

const SQL: &str = "
    WITH file_cognitive AS (
        SELECT path, MAX(cognitive) AS cognitive
        FROM {cm_src}
        GROUP BY path
    ),
    file_churn AS (
        SELECT path, COALESCE(SUM(loc_added), 0) + COALESCE(SUM(loc_deleted), 0) AS churn
        FROM {src}
        GROUP BY path
    ),
    file_revs AS (
        -- (rev, path) is the changes PK; COUNT(rev) == COUNT(DISTINCT rev)
        -- per path. Plain COUNT skips DuckDB's distinct-tracking overhead.
        SELECT path, COUNT(rev) AS revs
        FROM {src}
        GROUP BY path
        HAVING revs >= ?
    ),
    author_revs AS (
        SELECT
            c.path,
            commits.canonical_author AS author,
            -- (rev, path) is the changes PK so rev is unique within each
            -- (path, author) group. Plain COUNT skips DuckDB's
            -- distinct-tracking overhead.
            COUNT(c.rev) AS revs
        FROM {src} c
        INNER JOIN commits ON c.rev = commits.rev
        GROUP BY c.path, commits.canonical_author
    ),
    file_fv AS (
        SELECT
            ar.path,
            1.0 - SUM(POWER(ar.revs::DOUBLE / NULLIF(t.total, 0), 2)) AS fv
        FROM author_revs ar
        INNER JOIN (SELECT path, SUM(revs) AS total FROM author_revs GROUP BY path) t
            ON ar.path = t.path
        GROUP BY ar.path
    ),
    -- Weighted sum of per-file biomarker intensities (each a per-language
    -- PERCENT_RANK in [0,1]; smells absent for a file contribute 0). The
    -- weights sum to 1.0 and are ordered by empirical defect-correlation
    -- strength, so structural risk stays in [0,1] and spreads across the file
    -- distribution instead of saturating at the ceiling. Co-occurrence is
    -- implicit: a file flagged by more smells accumulates more weighted terms.
    file_structural AS (
        SELECT
            path,
            LEAST(1.0, SUM(intensity * CASE smell
                WHEN 'complex-method'  THEN 0.30
                WHEN 'god-class'       THEN 0.25
                WHEN 'large-method'    THEN 0.15
                WHEN 'dry'             THEN 0.15
                WHEN 'shotgun-surgery' THEN 0.15
                ELSE 0.0
            END)) AS structural_risk
        FROM code_health_biomarkers_v1
        GROUP BY path
    ),
    -- Coupling centrality enters the composite once, as the shotgun-surgery
    -- biomarker inside structural_risk. It is deliberately NOT also added as a
    -- separate behavioral term here — that would double-count the same signal.
    joined AS (
        SELECT
            fc.path,
            fc.cognitive,
            COALESCE(fch.churn, 0) AS churn,
            COALESCE(ffv.fv, 0.0) AS fv,
            COALESCE(fs.structural_risk, 0.0) AS structural_risk
        FROM file_cognitive fc
        INNER JOIN file_revs fr ON fc.path = fr.path
        LEFT JOIN file_churn fch ON fc.path = fch.path
        LEFT JOIN file_fv ffv ON fc.path = ffv.path
        LEFT JOIN file_structural fs ON fc.path = fs.path
    ),
    normalized AS (
        SELECT
            path,
            cognitive,
            churn,
            fv,
            structural_risk,
            CASE WHEN MAX(churn) OVER () > 0 THEN churn::DOUBLE / MAX(churn) OVER () ELSE 0 END AS n_cn,
            fv AS n_au
        FROM joined
    ),
    scored AS (
        SELECT
            path,
            cognitive,
            structural_risk,
            GREATEST(0.0, LEAST(100.0,
                100.0 * (1.0 - 0.50 * structural_risk - 0.30 * n_cn - 0.20 * n_au)
            )) AS score,
            CASE lower(split_part(path, '.', -1))
                WHEN 'rs' THEN 'rust'
                WHEN 'py' THEN 'python' WHEN 'pyi' THEN 'python'
                WHEN 'java' THEN 'java'
                WHEN 'js' THEN 'javascript' WHEN 'jsx' THEN 'javascript'
                WHEN 'mjs' THEN 'javascript' WHEN 'cjs' THEN 'javascript'
                WHEN 'ts' THEN 'typescript' WHEN 'tsx' THEN 'typescript'
                ELSE 'other'
            END AS lang
        FROM normalized
    )
    SELECT
        path,
        cognitive,
        score,
        structural_risk,
        PERCENT_RANK() OVER (PARTITION BY lang ORDER BY structural_risk) AS percentile,
        -- Absolute structural_risk thresholds (tunable; Phase-2 cross-repo
        -- corpus calibration will replace these self-relative cut points).
        -- red = high on at least half the weighted smell mass.
        CASE
            WHEN structural_risk >= 0.50 THEN 'red'
            WHEN structural_risk >= 0.25 THEN 'yellow'
            ELSE 'green'
        END AS band
    FROM scored
    ORDER BY score ASC, path ASC
    LIMIT ?
";

/// DDL for the temporary centrality table that backs the `shotgun-surgery`
/// biomarker (its intensity is the `PERCENT_RANK` of a file's Fisher-significant
/// coupling-partner count). Computed in Rust from the Fisher-filtered output of
/// `coupling::run_coupling`, then materialized in a session-local temp table so
/// `SHOTGUN_INSERT` can read it.
const CENTRALITY_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE coupling_centrality_v1 (
        path TEXT PRIMARY KEY,
        centrality INTEGER NOT NULL
    )
";

/// DDL for the per-file biomarker table. NOTE: this session-local table is
/// materialized as a side effect of `run_code_health` and is also read by an
/// EXTERNAL consumer — `analyses::refactoring_targets` queries it (after
/// calling `run_code_health`) to find each file's dominant smell. It is
/// therefore a cross-analysis contract: it must stay session-scoped and
/// readable after `run_code_health` returns — do not make it private, drop it
/// on exit, or rename its columns without updating that consumer.
const BIOMARKERS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE code_health_biomarkers_v1 (
        path    TEXT   NOT NULL,
        smell   TEXT   NOT NULL,
        intensity DOUBLE NOT NULL
    )
";

/// Populate function-level structural biomarkers from the raw
/// `complexity_metrics` snapshot. Intensity is the per-language
/// `PERCENT_RANK` of the function metric, rolled up to the file by `MAX`.
/// Language is derived from the file extension; no stored language column
/// exists in `complexity_metrics` so the CASE expression mirrors the one
/// in the scored CTE of the main code-health SQL.
const BIOMARKERS_INSERT: &str = "
    INSERT INTO code_health_biomarkers_v1 (path, smell, intensity)
    WITH lang_fn AS (
        SELECT
            path,
            name,
            cyclomatic,
            loc,
            CASE lower(split_part(path, '.', -1))
                WHEN 'rs'  THEN 'rust'
                WHEN 'py'  THEN 'python'
                WHEN 'pyi' THEN 'python'
                WHEN 'java' THEN 'java'
                WHEN 'js'  THEN 'javascript'
                WHEN 'jsx' THEN 'javascript'
                WHEN 'mjs' THEN 'javascript'
                WHEN 'cjs' THEN 'javascript'
                WHEN 'ts'  THEN 'typescript'
                WHEN 'tsx' THEN 'typescript'
                ELSE 'other'
            END AS lang
        FROM complexity_metrics
        WHERE cyclomatic IS NOT NULL
            AND loc IS NOT NULL
    ),
    -- Aggregate to the file FIRST (worst function per file), THEN rank files.
    -- Ranking functions and taking the per-file MAX saturates: any file with
    -- enough functions has one in the top percentile, so nearly every file
    -- scored ~1.0. Ranking files against files spreads the intensity uniformly.
    file_metric AS (
        SELECT path, lang, MAX(cyclomatic) AS file_cx, MAX(loc) AS file_loc
        FROM lang_fn
        GROUP BY path, lang
    ),
    ranked AS (
        SELECT
            path,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_cx) AS cx_i,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_loc) AS loc_i
        FROM file_metric
    )
    SELECT path, 'complex-method' AS smell, cx_i AS intensity FROM ranked
    UNION ALL
    SELECT path, 'large-method' AS smell, loc_i AS intensity FROM ranked
";

/// Shotgun Surgery / Divergent Change: a file that co-changes with many
/// Fisher-significant partners is definitionally a temporal smell. Reuses the
/// already-materialized centrality table; intensity = self-relative rank.
const SHOTGUN_INSERT: &str = "
    INSERT INTO code_health_biomarkers_v1 (path, smell, intensity)
    SELECT path, 'shotgun-surgery' AS smell,
           PERCENT_RANK() OVER (ORDER BY centrality) AS intensity
    FROM coupling_centrality_v1
    WHERE centrality > 0
";

fn materialize_biomarkers(db: &FactsDb, opts: &Options) -> Result<()> {
    db.conn()
        .execute(BIOMARKERS_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create biomarker temp table: {e}")))?;
    db.conn()
        .execute(BIOMARKERS_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert complexity biomarkers: {e}")))?;
    db.conn()
        .execute(SHOTGUN_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert shotgun-surgery biomarkers: {e}")))?;

    // God Class: reuse the existing analysis; intensity = normalized god_score.
    // `--rows N` MUST NOT propagate here — the biomarker set needs ALL god
    // classes, not the user's output truncation. A truncated set drifts a
    // surviving file's normalized intensity and breaks the score invariant.
    let gods = crate::analyses::god_classes::run_god_classes(db, &opts.with_no_row_limit())?;
    let max_god = gods.iter().map(|g| g.god_score).fold(0.0_f64, f64::max);

    // DRY: reuse clone detection (walks HEAD worktree); intensity = normalized
    // count of cloned functions per file.
    let clones = crate::analyses::clones::run_clones(opts)?;
    let mut dry_counts: std::collections::HashMap<String, u32> = HashMap::new();
    for c in &clones {
        *dry_counts.entry(c.entity.clone()).or_insert(0) += 1;
    }
    let max_dry = dry_counts.values().copied().max().unwrap_or(0);

    let mut stmt = db
        .conn()
        .prepare("INSERT INTO code_health_biomarkers_v1 (path, smell, intensity) VALUES (?, ?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare biomarker insert: {e}")))?;
    if max_god > 0.0 {
        for g in &gods {
            stmt.execute(params![g.path, "god-class", g.god_score / max_god])
                .map_err(|e| CodeLoreError::Analysis(format!("god-class biomarker: {e}")))?;
        }
    }
    if max_dry > 0 {
        for (path, n) in &dry_counts {
            stmt.execute(params![path, "dry", f64::from(*n) / f64::from(max_dry)])
                .map_err(|e| CodeLoreError::Analysis(format!("dry biomarker: {e}")))?;
        }
    }
    Ok(())
}

/// Build the centrality temp table from Fisher-significant coupling pairs.
/// Each path appears once with `centrality = count of pairs that include it`.
fn materialize_centrality(db: &FactsDb, opts: &Options) -> Result<()> {
    // `--rows N` MUST NOT propagate into the inner coupling query — the
    // centrality term needs the FULL coupled-pair graph, not the user's
    // output truncation. See `Options::with_no_row_limit` for the full
    // bug narrative.
    let pairs = run_coupling(db, &opts.with_no_row_limit())?;

    // Count Fisher-significant partners per path. Each pair contributes to
    // both endpoints' centrality.
    let mut counts: HashMap<String, u32> = HashMap::new();
    for p in &pairs {
        *counts.entry(p.entity_a.clone()).or_insert(0) += 1;
        *counts.entry(p.entity_b.clone()).or_insert(0) += 1;
    }

    db.conn()
        .execute(CENTRALITY_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create centrality temp table: {e}")))?;

    if counts.is_empty() {
        return Ok(()); // Nothing to insert; LEFT JOIN handles absence.
    }

    // Bulk INSERT via prepared statement — small N (typically <= 100s of
    // distinct paths), so per-row insert is fine without the Appender.
    let mut stmt = db
        .conn()
        .prepare("INSERT INTO coupling_centrality_v1 (path, centrality) VALUES (?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare centrality insert: {e}")))?;
    for (path, count) in &counts {
        stmt.execute(params![path, *count])
            .map_err(|e| CodeLoreError::Analysis(format!("centrality row insert: {e}")))?;
    }
    Ok(())
}

#[tracing::instrument(name = "code-health", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    // Materialize Fisher-filtered coupling centrality before the SQL runs.
    // `materialize_centrality` -> `run_coupling` ALSO materializes
    // `changes_lineage` when canonical lineage is on; the outer SQL below
    // must read from the same source so the JOIN on path matches the
    // canonical centrality entries (otherwise renamed files lose their
    // centrality term silently).
    materialize_centrality(db, opts)?;
    materialize_biomarkers(db, opts)?;

    // Unified dispatch honours both --time-bucket and --use-canonical-lineage.
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let cm_src = crate::analyses::grouped_complexity::source_table(opts);
    let sql = SQL.replace("{src}", src).replace("{cm_src}", cm_src);
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "code-health",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-health: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(CodeHealthRow {
                path: r.get::<_, String>(0)?,
                cognitive: r.get::<_, f64>(1)?,
                score: r.get::<_, f64>(2)?,
                structural_risk: r.get::<_, f64>(3)?,
                percentile: r.get::<_, f64>(4)?,
                band: r.get::<_, String>(5)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query code-health: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-health: {e}")))
}
