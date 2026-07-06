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
//! biomarker carries an intensity ∈ [0,1]. Four of them (complex-method,
//! large-method, god-class, dry) are a per-language `PERCENT_RANK` of the
//! file's worst value over the full file set; shotgun-surgery is a
//! `PERCENT_RANK` over the coupled-file set only (no language partition). The
//! per-smell weights sum to 1.0, so `structural_risk` stays in [0,1] and
//! spreads across the file distribution. Smells absent for a file contribute
//! 0, so co-occurrence is implicit — a file flagged by more smells accumulates
//! more weighted terms.
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

use crate::analyses::coupling::{run_coupling, run_coupling_scoped};
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// What revision / sources a code-health scan runs against. `head()` resolves
/// to today's HEAD tables so existing behaviour is byte-identical.
#[derive(Debug, Clone)]
pub struct HealthScanCtx {
    /// Complexity source table (HEAD: `"complexity_metrics"`).
    pub complexity_source: String,
    /// Imports source table for god-class fan-in/out (HEAD: `"imports"`).
    pub imports_source: String,
    /// When `Some(ts)`, history terms (churn, author, coupling) are limited to
    /// `commits.date <= ts`.
    pub history_cutoff: Option<String>,
    /// Include the clone/DRY biomarker (true at HEAD; false at a historical rev
    /// where clone detection is unavailable).
    pub include_clones: bool,
}

impl HealthScanCtx {
    /// The HEAD scan — every source resolves to today's table, DRY included.
    #[must_use]
    pub fn head() -> Self {
        Self {
            complexity_source: "complexity_metrics".to_string(),
            imports_source: "imports".to_string(),
            history_cutoff: None,
            include_clones: true,
        }
    }
}

/// Divisor appended to the `structural_risk` SUM when the DRY biomarker is
/// excluded: the four remaining weights (0.30+0.25+0.15+0.15) sum to 0.85, so
/// dividing by 0.85 renormalizes the risk scale back to 1.0. Empty at HEAD.
const STRUCTURAL_SCALE_NO_DRY: &str = " / 0.85";

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
            END){structural_scale}) AS structural_risk
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
        -- red = high on a majority of the weighted smell mass.
        CASE
            WHEN structural_risk >= 0.55 THEN 'red'
            WHEN structural_risk >= 0.28 THEN 'yellow'
            ELSE 'green'
        END AS band
    FROM scored
    ORDER BY score ASC, path ASC
    LIMIT ?
";

/// A changes view limited to commits at/-before a cutoff timestamp, so
/// history-derived terms (churn, author fragmentation, coupling) are rev-scoped.
const CHANGES_AT_TS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY VIEW changes_at_ts AS
    SELECT c.* FROM changes c
    INNER JOIN commits ON commits.rev = c.rev
    WHERE commits.date <= CAST(? AS TIMESTAMP)
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
        FROM {cm_src}
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

fn materialize_biomarkers(db: &FactsDb, opts: &Options, cx: &HealthScanCtx) -> Result<()> {
    db.conn()
        .execute(BIOMARKERS_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create biomarker temp table: {e}")))?;
    let biomarkers_insert = BIOMARKERS_INSERT.replace("{cm_src}", &cx.complexity_source);
    db.conn()
        .execute(&biomarkers_insert, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert complexity biomarkers: {e}")))?;
    db.conn()
        .execute(SHOTGUN_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert shotgun-surgery biomarkers: {e}")))?;

    // God-class and DRY are SPARSE smells (present for few files). Rank each
    // file's raw metric over the FULL per-language file universe — the same set
    // complex-method / large-method rank over — with absent files contributing
    // 0. This keeps a lone or tied occurrence high (it still ranks above the
    // zero-majority) instead of collapsing to a 0 rank, and aligns god-class /
    // dry with the complex/large per-file percentile scheme (min-max before).
    // (shotgun-surgery ranks over its own coupled-file set — see SHOTGUN_INSERT.)
    //
    // `--rows N` MUST NOT propagate into `run_god_classes`: the biomarker set
    // needs ALL god classes, not the user's output truncation.
    let gods = crate::analyses::god_classes::run_god_classes_scoped(
        db,
        &opts.with_no_row_limit(),
        &cx.complexity_source,
        &cx.imports_source,
    )?;
    let god_by_path: HashMap<String, f64> =
        gods.iter().map(|g| (g.path.clone(), g.god_score)).collect();

    let dry_counts: HashMap<String, u32> = if cx.include_clones {
        let clones = crate::analyses::clones::run_clones(opts)?;
        let mut m: HashMap<String, u32> = HashMap::new();
        for c in &clones {
            *m.entry(c.entity.clone()).or_insert(0) += 1;
        }
        m
    } else {
        HashMap::new()
    };

    // Full file universe (files with complexity data), grouped by language —
    // the same universe the SQL-side complex/large biomarkers rank over.
    let universe_sql = "SELECT DISTINCT path FROM {cm_src} \
         WHERE cyclomatic IS NOT NULL AND loc IS NOT NULL"
        .replace("{cm_src}", &cx.complexity_source);
    let universe = crate::analyses::query::query_map_collect(
        db,
        &universe_sql,
        [],
        "biomarker-universe",
        |r| r.get::<_, String>(0),
    )?;
    let mut by_lang: HashMap<&'static str, Vec<String>> = HashMap::new();
    for path in universe {
        let lang =
            crate::complexity::Tier1Language::from_path(&path).map_or("other", |l| l.as_str());
        by_lang.entry(lang).or_default().push(path);
    }

    let mut stmt = db
        .conn()
        .prepare("INSERT INTO code_health_biomarkers_v1 (path, smell, intensity) VALUES (?, ?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare biomarker insert: {e}")))?;
    for files in by_lang.values() {
        if files.len() <= 1 {
            continue; // PERCENT_RANK is degenerate for a single-file language
        }
        let denom = f64::from(u32::try_from(files.len() - 1).unwrap_or(u32::MAX));
        let smells: &[&str] = if cx.include_clones {
            &["god-class", "dry"]
        } else {
            &["god-class"]
        };
        for &smell in smells {
            let vals: Vec<f64> = files
                .iter()
                .map(|p| {
                    if smell == "god-class" {
                        god_by_path.get(p).copied().unwrap_or(0.0)
                    } else {
                        dry_counts.get(p).map_or(0.0, |n| f64::from(*n))
                    }
                })
                .collect();
            for (path, &v) in files.iter().zip(vals.iter()) {
                if v <= 0.0 {
                    continue; // only files that HAVE the smell get a row
                }
                let less = vals.iter().filter(|&&x| x < v).count();
                let intensity = f64::from(u32::try_from(less).unwrap_or(u32::MAX)) / denom;
                stmt.execute(params![path, smell, intensity])
                    .map_err(|e| CodeLoreError::Analysis(format!("{smell} biomarker: {e}")))?;
            }
        }
    }
    Ok(())
}

/// Build the centrality temp table from Fisher-significant coupling pairs.
/// Each path appears once with `centrality = count of pairs that include it`.
/// When `cx.history_cutoff` is set, coupling is restricted to the already-
/// materialized `changes_at_ts` view; at HEAD (cutoff None) the standard
/// memoised [`run_coupling`] path is taken — output is byte-identical.
fn materialize_centrality(db: &FactsDb, opts: &Options, cx: &HealthScanCtx) -> Result<()> {
    // `--rows N` MUST NOT propagate into the inner coupling query — the
    // centrality term needs the FULL coupled-pair graph, not the user's
    // output truncation. See `Options::with_no_row_limit` for the full
    // bug narrative.
    let pairs = if cx.history_cutoff.is_some() {
        run_coupling_scoped(db, &opts.with_no_row_limit(), "changes_at_ts")?
    } else {
        run_coupling(db, &opts.with_no_row_limit())?
    };

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
    // Route complexity reads through the grouped table when `--group-file` is
    // active (the `grouped_complexity` contract), so the cognitive + biomarker
    // CTEs key on the same group names the `changes` history is rewritten to.
    // Without grouping this resolves to `complexity_metrics`, keeping HEAD
    // output byte-identical.
    let cx = HealthScanCtx {
        complexity_source: crate::analyses::grouped_complexity::source_table(opts).to_string(),
        ..HealthScanCtx::head()
    };
    run_code_health_scoped(db, opts, &cx)
}

/// Code health against the sources named by `cx`. `cx = HealthScanCtx::head()`
/// reproduces the HEAD analysis byte-for-byte.
pub fn run_code_health_scoped(
    db: &FactsDb,
    opts: &Options,
    cx: &HealthScanCtx,
) -> Result<Vec<CodeHealthRow>> {
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src_owned;
    let src: &str = if let Some(ts) = &cx.history_cutoff {
        db.conn()
            .execute(CHANGES_AT_TS_DDL, params![ts])
            .map_err(|e| CodeLoreError::Analysis(format!("create changes_at_ts view: {e}")))?;
        src_owned = "changes_at_ts".to_string();
        &src_owned
    } else {
        crate::analyses::lineage::source_table(opts)
    };
    materialize_centrality(db, opts, cx)?;
    materialize_biomarkers(db, opts, cx)?;

    let cm_src = &cx.complexity_source;
    let structural_scale = if cx.include_clones {
        ""
    } else {
        STRUCTURAL_SCALE_NO_DRY
    };
    let sql = SQL
        .replace("{src}", src)
        .replace("{cm_src}", cm_src)
        .replace("{structural_scale}", structural_scale);
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

#[cfg(test)]
mod tests {
    #[test]
    fn head_ctx_defaults_to_head_tables() {
        let c = super::HealthScanCtx::head();
        assert_eq!(c.complexity_source, "complexity_metrics");
        assert_eq!(c.imports_source, "imports");
        assert!(c.history_cutoff.is_none());
        assert!(c.include_clones);
    }

    #[test]
    fn no_dry_scale_renormalizes_to_one() {
        // 0.30 + 0.25 + 0.15 + 0.15 = 0.85; dividing by 0.85 restores a 1.0 ceiling.
        let sum = 0.30 + 0.25 + 0.15_f64 + 0.15;
        assert!((sum - 0.85).abs() < 1e-9);
        assert_eq!(super::STRUCTURAL_SCALE_NO_DRY, " / 0.85");
    }
}
