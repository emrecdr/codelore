//! `main-dev` / `main-dev-by-revs` / `main-dev-by-deletions` analyses.
//!
//! Three variants of the same query shape — author-aggregation, top-by-metric,
//! ownership ratio — switching only the metric column.
//!
//! | Analysis                        | Metric                | Code-maat name        | Honest CSV headers                            |
//! |---------------------------------|-----------------------|-----------------------|-----------------------------------------------|
//! | `main-dev`                      | `SUM(loc_added)`      | `main-dev`            | `entity,main-dev,added,total-added,ownership`     |
//! | `main-dev-by-revs`              | `COUNT(*)`            | `main-dev-by-revs`    | `entity,main-dev,revisions,total-revisions,ownership` |
//! | `main-dev-by-deletions`         | `SUM(loc_deleted)`    | `refactoring-main-dev`| `entity,main-dev,removed,total-removed,ownership` |
//!
//! ## Modernization decisions
//!
//! - **Honest column headers**: code-maat reused `added` / `total-added`
//!   for `main-dev-by-revs` even though the values are revision counts.
//!   We emit `revisions` / `total-revisions` instead. Under `--code-maat-
//!   compat` the legacy headers are restored.
//! - **Deterministic tiebreaks**: when two authors tie on the metric, the
//!   alphabetically-first author wins (`ORDER BY metric DESC, author ASC`)
//!   instead of code-maat's `(first (reverse (sort-by metric-fn ...)))`
//!   arbitrary pick.
//! - **`refactoring-main-dev` is an alias** for `main-dev-by-deletions`.
//!   Code-maat's name implies a refactor-commit-message filter that doesn't
//!   exist (the analysis is just "main-dev with metric=deleted-lines"; the
//!   "refactoring" framing is Tornhill's heuristic that removing code is a
//!   deliberate design choice). Both names dispatch to the same query.
//! - **`canonical_author`** (post-mailmap) is the grouping key, not raw
//!   `author_email`.
//!
//! Research basis: see `docs/research-foundations.md` entry "main-dev"
//! (Mockus & Herbsleb, ICSE 2002 — expertise identification;
//! D'Ambros et al., *Empirical Software Engineering* 2010 — three-metric
//! decomposition; Tornhill, *Your Code as a Crime Scene*, 2015 — the
//! refactoring-main-dev heuristic).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

/// Which metric drives the "main developer" ranking.
#[derive(Debug, Clone, Copy)]
pub enum MainDevMetric {
    /// Top author by `SUM(loc_added)`.
    Added,
    /// Top author by `SUM(loc_deleted)`.
    Deleted,
    /// Top author by `COUNT(*)` revisions.
    RevCount,
}

impl MainDevMetric {
    /// SQL expression that computes the metric. Aliased to `metric` in the
    /// outer query so the same downstream logic applies regardless of which
    /// variant ran.
    const fn sql_expr(self) -> &'static str {
        match self {
            Self::Added => "SUM(c.loc_added)",
            Self::Deleted => "SUM(c.loc_deleted)",
            Self::RevCount => "COUNT(*)",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MainDevRow {
    pub entity: String,
    pub main_dev: String,
    /// Numeric value of the metric for the winning author. Column header in
    /// CSV is `added` / `revisions` / `removed` depending on the variant.
    pub metric: u64,
    /// Sum of the metric across all authors for this entity. Header in CSV
    /// is `total-added` / `total-revisions` / `total-removed`.
    pub total: u64,
    /// `metric / total`, rounded to 2 decimal places (code-maat parity).
    pub ownership: f64,
}

fn run(db: &FactsDb, opts: &Options, metric: MainDevMetric) -> Result<Vec<MainDevRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let metric_expr = metric.sql_expr();

    // SQL builds the inline expression from the typed Metric enum, so there's
    // no string interpolation of user-controlled input. The other two bind
    // values (min_revs, row_limit) go through params!.
    let sql = format!(
        "WITH ea AS (
             SELECT c.path AS entity, m.canonical_author AS author,
                    {metric_expr}::BIGINT AS metric
             FROM changes c JOIN commits m USING (rev)
             GROUP BY c.path, m.canonical_author
         ),
         winners AS (
             -- `first(... ORDER BY ...)` + per-group MAX/SUM collapses the
             -- old `ranked`+`totals` pair plus their self-join into a single
             -- grouped aggregate (deterministic ASC author tiebreak).
             SELECT entity,
                    first(author ORDER BY metric DESC, author ASC) AS author,
                    MAX(metric) AS metric,
                    SUM(metric) AS total
             FROM ea
             GROUP BY entity
         ),
         file_revs AS (
             -- (rev, path) is the changes PK; COUNT(rev) == COUNT(DISTINCT rev)
             -- per path. Plain COUNT skips DuckDB's distinct-tracking overhead.
             SELECT path, COUNT(rev) AS revs FROM changes
             GROUP BY path HAVING revs >= ?
         )
         SELECT w.entity, w.author, w.metric, w.total,
                ROUND(w.metric::DOUBLE / GREATEST(w.total, 1), 2) AS ownership
         FROM winners w
         INNER JOIN file_revs fr ON fr.path = w.entity
         ORDER BY w.entity ASC
         LIMIT ?"
    );

    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(&sql, opts);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "main-dev",
        opts,
    )?;
    crate::analyses::query::query_map_collect(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "main-dev",
        |r| {
            Ok(MainDevRow {
                entity: r.get::<_, String>(0)?,
                main_dev: r.get::<_, String>(1)?,
                metric: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(u64::MAX),
                total: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(u64::MAX),
                ownership: r.get::<_, f64>(4)?,
            })
        },
    )
}

/// Top author per file ranked by lines added. Code-maat parity.
#[tracing::instrument(name = "main-dev", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_main_dev(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> {
    run(db, opts, MainDevMetric::Added)
}

/// Top author per file ranked by lines deleted. Code-maat ships this as
/// `refactoring-main-dev` (Tornhill's heuristic that removing code is a
/// deliberate design choice). The honest name is `main-dev-by-deletions`;
/// `refactoring-main-dev` is an accepted alias.
#[tracing::instrument(name = "main-dev-by-deletions", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_main_dev_by_deletions(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> {
    run(db, opts, MainDevMetric::Deleted)
}

/// Top author per file ranked by revision count. Code-maat parity.
#[tracing::instrument(name = "main-dev-by-revs", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_main_dev_by_revs(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> {
    run(db, opts, MainDevMetric::RevCount)
}
