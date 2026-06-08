//! `entity-ownership` analysis — code-maat parity.
//!
//! Per-`(entity, author)` churn breakdown — how many lines added/deleted
//! by each author on each file. Differs from `ownership` (Fractal Value
//! summary per file) by emitting one row per `(entity, author)` pair
//! instead of one row per entity.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityOwnershipRow {
    pub entity: String,
    pub author: String,
    pub added: u64,
    pub deleted: u64,
}

const SQL: &str = "
    SELECT c.path AS entity,
           m.canonical_author AS author,
           COALESCE(SUM(c.loc_added), 0)::BIGINT AS added,
           COALESCE(SUM(c.loc_deleted), 0)::BIGINT AS deleted
    FROM changes c JOIN commits m USING (rev)
    GROUP BY c.path, m.canonical_author
    ORDER BY entity ASC, author ASC
    LIMIT ?
";

pub fn run_entity_ownership(db: &FactsDb, opts: &Options) -> Result<Vec<EntityOwnershipRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![row_limit],
        "entity-ownership",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare entity-ownership: {e}")))?;
    let rows = stmt
        .query_map(params![row_limit], |r| {
            Ok(EntityOwnershipRow {
                entity: r.get::<_, String>(0)?,
                author: r.get::<_, String>(1)?,
                added: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(u64::MAX),
                deleted: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(u64::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query entity-ownership: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect entity-ownership: {e}")))
}
