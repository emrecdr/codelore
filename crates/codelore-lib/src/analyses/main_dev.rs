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

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

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
         totals AS (
             SELECT entity, SUM(metric) AS total FROM ea GROUP BY entity
         ),
         ranked AS (
             SELECT entity, author, metric,
                    ROW_NUMBER() OVER (
                        PARTITION BY entity ORDER BY metric DESC, author ASC
                    ) AS rn
             FROM ea
         ),
         file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs FROM changes
             GROUP BY path HAVING revs >= ?
         )
         SELECT r.entity, r.author, r.metric, t.total,
                ROUND(r.metric::DOUBLE / GREATEST(t.total, 1), 2) AS ownership
         FROM ranked r
         INNER JOIN totals t USING (entity)
         INNER JOIN file_revs fr ON fr.path = r.entity
         WHERE r.rn = 1
         ORDER BY r.entity ASC
         LIMIT ?"
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare main-dev: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(MainDevRow {
                entity: r.get::<_, String>(0)?,
                main_dev: r.get::<_, String>(1)?,
                metric: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(u64::MAX),
                total: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(u64::MAX),
                ownership: r.get::<_, f64>(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query main-dev: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect main-dev: {e}")))
}

/// Top author per file ranked by lines added. Code-maat parity.
pub fn run_main_dev(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> {
    run(db, opts, MainDevMetric::Added)
}

/// Top author per file ranked by lines deleted. Code-maat ships this as
/// `refactoring-main-dev` (Tornhill's heuristic that removing code is a
/// deliberate design choice). The honest name is `main-dev-by-deletions`;
/// `refactoring-main-dev` is an accepted alias.
pub fn run_main_dev_by_deletions(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> {
    run(db, opts, MainDevMetric::Deleted)
}

/// Top author per file ranked by revision count. Code-maat parity.
pub fn run_main_dev_by_revs(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> {
    run(db, opts, MainDevMetric::RevCount)
}
