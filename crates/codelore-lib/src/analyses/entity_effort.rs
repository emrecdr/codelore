//! `entity-effort` analysis — code-maat parity.
//!
//! Per-`(entity, author)` row showing how many revisions a given author
//! contributed to a given file, alongside the total revisions of that file.
//! Useful for surfacing "who's doing the work on this file" without
//! collapsing to a single main-dev.
//!
//! Output sorted by `entity ASC, author_revs DESC, author ASC` —
//! deterministic across runs (code-maat sorts only by the first two and
//! relies on Clojure's stable sort).
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "entity-effort / entity-ownership" (D'Ambros, Gall, Lanza & Pinzger
//! 2008 — per-author-per-file detail as input to expertise heat maps
//! and refactoring-task assignment).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityEffortRow {
    pub entity: String,
    pub author: String,
    pub author_revs: u32,
    pub total_revs: u32,
}

const SQL: &str = "
    WITH ea AS (
        SELECT c.path AS entity, m.canonical_author AS author,
               COUNT(*)::BIGINT AS author_revs
        FROM changes c JOIN commits m USING (rev)
        GROUP BY c.path, m.canonical_author
    )
    SELECT entity, author, author_revs,
           SUM(author_revs) OVER (PARTITION BY entity) AS total_revs
    FROM ea
    ORDER BY entity ASC, author_revs DESC, author ASC
    LIMIT ?
";

#[tracing::instrument(name = "entity-effort", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_entity_effort(db: &FactsDb, opts: &Options) -> Result<Vec<EntityEffortRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    // The SQL has a single `?` placeholder (LIMIT). Param list MUST
    // match the `query_map` call below — DuckDB's EXPLAIN rejects
    // length mismatches with `Got N, needed M`.
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![row_limit],
        "entity-effort",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare entity-effort: {e}")))?;
    let rows = stmt
        .query_map(params![row_limit], |r| {
            Ok(EntityEffortRow {
                entity: r.get::<_, String>(0)?,
                author: r.get::<_, String>(1)?,
                author_revs: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                total_revs: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query entity-effort: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect entity-effort: {e}")))
}
