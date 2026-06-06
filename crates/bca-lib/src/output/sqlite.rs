//! `SQLite` output via `DuckDB` ATTACH ... (TYPE SQLITE).
//!
//! Dumps the entire fact store (7 tables) to a `SQLite` file. Plan 5 ships
//! the full dump; future plans may add per-analysis filtered dumps.

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};
use std::path::Path;

pub fn write_full_fact_store_sqlite(db: &FactsDb, _opts: &Options, path: &Path) -> Result<()> {
    // Ensure the target file doesn't exist first (DuckDB ATTACH errors otherwise)
    let _ = std::fs::remove_file(path);

    let path_str = path.display().to_string().replace('\'', "''");
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
         DETACH sink;"
    );
    db.conn()
        .execute_batch(&sql)
        .map_err(|e| BcaError::Output(format!("sqlite: {e}")))?;
    Ok(())
}
