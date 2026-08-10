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
        // `INSTALL` fetches the extension from `DuckDB`'s registry the first
        // time it runs, so this one statement needs network access while
        // everything after it is local. It is issued separately for that
        // reason: an air-gapped or firewalled host otherwise fails with a bare
        // download error attributed to "sqlite", which reads as a bug in the
        // export rather than a missing prerequisite. Splitting lets the hint
        // attach to the step it actually explains, without pattern-matching
        // `DuckDB`'s error text.
        db.conn()
            .execute_batch("INSTALL sqlite; LOAD sqlite;")
            .map_err(|e| {
                CodeLoreError::Output(format!(
                    "sqlite: could not load DuckDB's sqlite extension: {e}\n\
                     hint: `INSTALL sqlite` fetches the extension on first use and caches \
                     it under ~/.duckdb/extensions/<duckdb-version>/<platform>/, so it needs \
                     both network access and a writable cache directory. To export from an \
                     offline or locked-down host, run a sqlite export once where both are \
                     available and copy that directory across, or choose another --format."
                ))
            })?;
        let sql = format!(
            "ATTACH '{path_str}' AS sink (TYPE SQLITE);
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
