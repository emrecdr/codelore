//! Code Health composite analysis per spec §4.6.
//!
//! ```text
//! codehealth(entity) = 100 × (1
//!     - w_cx · normalize(cognitive_complexity)
//!     - w_cn · normalize(churn_rate)
//!     - w_au · normalize(author_fragmentation_FV)
//!     - w_cp · normalize(coupling_centrality_SoC)
//! )
//!
//! defaults: w_cx = 0.40, w_cn = 0.25, w_au = 0.15, w_cp = 0.20
//! ```
//!
//! All 4 inputs are wired. Coupling centrality uses the Fisher-significant
//! pairs from `coupling::run_coupling` (the same gate used in the standalone
//! `coupling` analysis). Normalization uses the in-repo maximum as the
//! empirical upper bound (min-max). Score range: [0, 100]; higher = healthier.
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "code-health" (composite signal developed in `CodeLore`; underlying
//! inputs cite cognitive complexity (Campbell 2018), churn (Nagappan
//! & Ball 2005), ownership fragmentation (Mockus & Herbsleb 2002), and
//! coupling centrality (Tornhill 2018)).

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
    pub percentile: f64,      // 0..=1; per-language self-relative rank of structural_risk (1 = riskiest)
    pub band: String,         // "red" | "yellow" | "green"
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
    -- coupling_centrality_v1 is a TEMPORARY TABLE populated from
    -- coupling::run_coupling output (Fisher-filtered pairs) before this
    -- SQL runs. Centrality = count of Fisher-significant partners.
    joined AS (
        SELECT
            fc.path,
            fc.cognitive,
            COALESCE(fch.churn, 0) AS churn,
            COALESCE(ffv.fv, 0.0) AS fv,
            COALESCE(fcp.centrality, 0) AS centrality
        FROM file_cognitive fc
        INNER JOIN file_revs fr ON fc.path = fr.path
        LEFT JOIN file_churn fch ON fc.path = fch.path
        LEFT JOIN file_fv ffv ON fc.path = ffv.path
        LEFT JOIN coupling_centrality_v1 fcp ON fc.path = fcp.path
    ),
    normalized AS (
        SELECT
            path,
            cognitive,
            churn,
            fv,
            centrality,
            CASE WHEN MAX(cognitive) OVER () > 0 THEN cognitive / MAX(cognitive) OVER () ELSE 0 END AS n_cx,
            CASE WHEN MAX(churn) OVER () > 0 THEN churn::DOUBLE / MAX(churn) OVER () ELSE 0 END AS n_cn,
            fv AS n_au,
            CASE WHEN MAX(centrality) OVER () > 0 THEN centrality::DOUBLE / MAX(centrality) OVER () ELSE 0 END AS n_cp
        FROM joined
    ),
    scored AS (
        SELECT
            path,
            cognitive,
            n_cx AS structural_risk,
            GREATEST(0.0, LEAST(100.0,
                100.0 * (1.0 - 0.40 * n_cx - 0.25 * n_cn - 0.15 * n_au - 0.20 * n_cp)
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
        CASE
            WHEN structural_risk >= 0.66 THEN 'red'
            WHEN structural_risk >= 0.33 THEN 'yellow'
            ELSE 'green'
        END AS band
    FROM scored
    ORDER BY score ASC, path ASC
    LIMIT ?
";

/// DDL for the temporary centrality table that backs the composite score's
/// `n_cp` term. Computed in Rust from the Fisher-filtered output of
/// `coupling::run_coupling`, then materialized in a session-local temp table
/// so the main code-health SQL can JOIN it like any other source.
const CENTRALITY_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE coupling_centrality_v1 (
        path TEXT PRIMARY KEY,
        centrality INTEGER NOT NULL
    )
";

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
