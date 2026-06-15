//! `bus-factor` analysis.
//!
//! Computes per-module (per directory or per group-file group) bus
//! factor: the minimum number of authors whose departure would
//! leave the module unmaintained. Approximates Vladimir Filatov's
//! 2010 definition: the smallest set of contributors whose combined
//! commit count covers ≥ X% (default 80%) of the module's total
//! commits.
//!
//! ## Where the module boundary comes from
//!
//! Default: the top-level directory of each file path (e.g.
//! `src/foo/bar.rs` → module `src`). When `--group-file` is set,
//! the ingest's `apply_grouping` pass has already rewritten the
//! `changes.path` column to group names — the analysis just rolls
//! up per `path` which is now group-shaped. This is the intended
//! interaction: `--group-file` defines architectural modules; the
//! bus-factor analysis answers "what's the risk per architectural
//! module?".
//!
//! ## `CodeScene` parity-and-better
//!
//! `CodeScene`'s "Key Personnel" widget computes file-level bus
//! factor. This analysis lifts it to module-level, which is what
//! tech-leads actually care about — per-file is too granular to act
//! on. Bus factor = 1 module = a clear "who else needs to learn
//! this?" answer.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BusFactorRow {
    pub module: String,
    /// Total commits touching files in this module across the
    /// analysis window.
    pub total_commits: u32,
    /// Bus factor (Filatov 2010): the minimum number of authors
    /// whose combined commit count covers ≥ 80% of the module's
    /// total commits. Smaller = more concentrated knowledge.
    pub bus_factor: u32,
    /// Top contributor's name (highest commit count in the module).
    /// Useful for "if X leaves, what hits the floor first" follow-up.
    pub top_contributor: String,
    /// Top contributor's share of the module's total commits, in
    /// `[0, 1]`. 1.0 means a single author owns 100% of the module.
    pub top_contributor_share: f64,
}

/// Run the `bus-factor` analysis.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` errors.
pub fn run_bus_factor(db: &FactsDb, opts: &Options) -> Result<Vec<BusFactorRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    // SQL: build per-(module, author) commit counts, sort within
    // each module by count DESC, compute the cumulative share, and
    // surface the threshold-crossing position as the bus factor.
    let sql = "
        WITH per_module_author AS (
            SELECT
                regexp_extract(c.path, '^[^/]+', 0) AS module,
                co.canonical_author AS author,
                COUNT(DISTINCT c.rev) AS commits
            FROM changes c
            INNER JOIN commits co ON co.rev = c.rev
            WHERE co.is_merge = FALSE
              AND c.path LIKE '%/%'
            GROUP BY module, co.canonical_author
        ),
        per_module AS (
            SELECT
                module,
                SUM(commits) AS total_commits
            FROM per_module_author
            GROUP BY module
        ),
        ranked AS (
            SELECT
                pma.module,
                pma.author,
                pma.commits,
                pm.total_commits,
                ROW_NUMBER() OVER (PARTITION BY pma.module ORDER BY pma.commits DESC, pma.author ASC) AS rank,
                SUM(pma.commits) OVER (
                    PARTITION BY pma.module
                    ORDER BY pma.commits DESC, pma.author ASC
                    ROWS UNBOUNDED PRECEDING
                ) AS cum_commits
            FROM per_module_author pma
            INNER JOIN per_module pm ON pm.module = pma.module
        ),
        bus_factor_calc AS (
            SELECT
                module,
                MIN(rank) AS bus_factor
            FROM ranked
            WHERE cum_commits >= total_commits * 0.8
            GROUP BY module
        ),
        top AS (
            SELECT module, author AS top_author, commits AS top_commits
            FROM ranked
            WHERE rank = 1
        )
        SELECT
            pm.module,
            CAST(pm.total_commits AS UINTEGER) AS total_commits,
            CAST(COALESCE(bfc.bus_factor, 1) AS UINTEGER) AS bus_factor,
            t.top_author,
            (t.top_commits::DOUBLE / NULLIF(pm.total_commits, 0)::DOUBLE) AS top_share
        FROM per_module pm
        LEFT JOIN bus_factor_calc bfc ON bfc.module = pm.module
        INNER JOIN top t ON t.module = pm.module
        WHERE pm.module IS NOT NULL AND pm.module != ''
        ORDER BY bus_factor ASC, pm.total_commits DESC, pm.module ASC
        LIMIT ?
    ";

    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare bus-factor: {e}")))?;
    let rows = stmt
        .query_map(params![row_limit], |r| {
            Ok(BusFactorRow {
                module: r.get(0)?,
                total_commits: r.get(1)?,
                bus_factor: r.get(2)?,
                top_contributor: r.get(3)?,
                top_contributor_share: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query bus-factor: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect bus-factor: {e}")))
}
