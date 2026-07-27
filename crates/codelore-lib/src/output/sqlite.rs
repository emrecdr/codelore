//! `SQLite` output via `DuckDB` ATTACH ... (TYPE SQLITE).
//!
//! Dumps every base table in `schema_v1.sql` (currently: `commits`,
//! `changes`, `hunks`, `entities`, `complexity_metrics`, `author_aliases`,
//! `provenance`, `clones`, `imports`, `commit_parents`) to a `SQLite` file.
//! When a new base table is added to `schema_v1.sql`, it MUST be appended
//! here too — otherwise consumers of the `SQLite` export silently lose that
//! data with no error at write time. The round-trip test derives its expected
//! table set from the live catalog so a missed table fails loudly.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};
use std::path::Path;

pub fn write_full_fact_store_sqlite(db: &FactsDb, _opts: &Options, path: &Path) -> Result<()> {
    // Atomic publish: write to a temp sibling and rename over `path` only on
    // success, so an interrupted export never destroys the previous good file.
    // `atomic_publish` hands a path that does not yet exist, which is exactly
    // what `ATTACH ... (TYPE SQLITE)` requires (it errors on an existing file).
    crate::output::atomic_publish(path, |tmp| {
        let path_str = tmp.display().to_string().replace('\'', "''");
        let sql = format!(
            "INSTALL sqlite; LOAD sqlite;
             ATTACH '{path_str}' AS sink (TYPE SQLITE);
             CREATE TABLE sink.commits             AS SELECT * FROM commits;
             CREATE TABLE sink.changes             AS SELECT * FROM changes;
             CREATE TABLE sink.hunks               AS SELECT * FROM hunks;
             CREATE TABLE sink.entities            AS SELECT * FROM entities;
             CREATE TABLE sink.complexity_metrics  AS SELECT * FROM complexity_metrics;
             CREATE TABLE sink.author_aliases      AS SELECT * FROM author_aliases;
             CREATE TABLE sink.provenance          AS SELECT * FROM provenance;
             CREATE TABLE sink.clones              AS SELECT * FROM clones;
             CREATE TABLE sink.imports             AS SELECT * FROM imports;
             CREATE TABLE sink.commit_parents      AS SELECT * FROM commit_parents;
             DETACH sink;"
        );
        db.conn()
            .execute_batch(&sql)
            .map_err(|e| CodeLoreError::Output(format!("sqlite: {e}")))?;
        Ok(())
    })
}
