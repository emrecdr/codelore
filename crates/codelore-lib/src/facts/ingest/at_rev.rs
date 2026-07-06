//! Rev-parameterized ingest helpers. Materializes complexity metrics and
//! import-graph edges for an arbitrary revision into caller-named temporary
//! tables, enabling the health timeline to build a `HealthScanCtx` that
//! points at historical data instead of the HEAD tables.

use crate::analyses::import_graph::ImportGraph;
use crate::{CodeLoreError, Result};

use super::FactsDb;
use super::consumer::{append_metric_row, dedup_entities};

/// Scan Tier-1 source blobs at `rev`, compute complexity metrics, and write
/// the results into a freshly-created temporary table named `dest_table`.
/// The temporary table has the same column shape as `complexity_metrics`.
///
/// `live_paths` is the caller-supplied slice of repo-relative paths that
/// exist at `rev`. Each path is checked via `repo.read_blob_at(rev, path)`;
/// paths absent at that revision are silently skipped.
///
/// Mirrors `ingest_complexity_at_head`'s rayon-parallel + serial-drain
/// pattern: blob reads and parsing run in parallel; the DuckDB Appender
/// drain runs on the connection-owning thread.
pub fn ingest_complexity_at_rev<R: crate::repo::Repo>(
    db: &FactsDb,
    repo: &R,
    rev: &str,
    live_paths: &[String],
    dest_table: &str,
) -> Result<()> {
    use crate::complexity::{Tier1Language, compute_for_file};
    use rayon::prelude::*;

    // Create the destination temp table with the same column shape as
    // `complexity_metrics`. PRIMARY KEY is omitted — snapshot temp tables
    // don't need the uniqueness constraint.
    db.execute_batch(&format!(
        "CREATE OR REPLACE TEMPORARY TABLE {dest_table} (
            path                TEXT NOT NULL,
            name                TEXT NOT NULL,
            rev                 TEXT NOT NULL,
            cyclomatic          INTEGER,
            cognitive           INTEGER,
            halstead_volume     DOUBLE,
            halstead_difficulty DOUBLE,
            halstead_effort     DOUBLE,
            mi                  DOUBLE,
            nom                 INTEGER,
            nexits              INTEGER,
            loc                 INTEGER,
            sloc                INTEGER,
            max_nesting         INTEGER,
            mean_nesting        DOUBLE,
            sd_nesting          DOUBLE,
            total_nesting       INTEGER
        )"
    ))?;

    let live_paths: Vec<String> = live_paths.to_vec();
    let rev_owned = rev.to_string();

    // Phase 1 (parallel): read blob at `rev` + tree-sitter parse.
    // Per-file errors are logged + skipped — a single unreadable file does
    // not abort the scan, matching the same resilience contract as the HEAD
    // pass.
    let batches: Vec<Option<(String, Vec<crate::complexity::ComplexityEntity>)>> = live_paths
        .into_par_iter()
        .map_init(
            || (),
            |_state, path| {
                let lang = Tier1Language::from_path(&path)?;
                let source = match repo.read_blob_at(&rev_owned, &path) {
                    Ok(Some(b)) => b,
                    Ok(None) => {
                        tracing::debug!(
                            "at_rev complexity: {path} not tracked at {rev_owned}; skipping"
                        );
                        return None;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "at_rev complexity: blob read failed for {path} at {rev_owned}: {e}"
                        );
                        return None;
                    }
                };
                if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                    tracing::debug!(
                        "at_rev complexity: skipping {path} at {rev_owned} \
                         ({size} bytes > {cap}-byte AST cap)",
                        size = source.len(),
                        cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                    );
                    return None;
                }
                let synth_path = std::path::Path::new(&path);
                let entities = match compute_for_file(synth_path, source, lang) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("at_rev complexity: parse error {path} at {rev_owned}: {e}");
                        return None;
                    }
                };
                let deduped = dedup_entities(entities);
                Some((path, deduped))
            },
        )
        .collect();

    // Phase 2 (serial drain): Appender on the connection-owning thread.
    // `duckdb::Appender` is `!Send + !Sync` — must stay on this thread.
    let mut metrics_app = db
        .conn()
        .appender(dest_table)
        .map_err(|e| CodeLoreError::Analysis(format!("appender {dest_table}: {e}")))?;

    for batch in batches {
        let Some((path, entities)) = batch else {
            continue;
        };
        for ent in &entities {
            append_metric_row(&mut metrics_app, &path, ent, rev)?;
        }
    }

    metrics_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush {dest_table}: {e}")))?;
    Ok(())
}

/// Write the resolved edges of `graph` into a freshly-created temporary table
/// named `dest_table`. The table has the same column shape as `imports`
/// (`rev, src_path, target, resolved, target_path, kind`) without the FK
/// constraint on `rev`.
///
/// `ImportGraph` carries only resolved edges (built from
/// `WHERE target_path IS NOT NULL`), so every row lands with
/// `resolved = TRUE`, `kind = 'absolute'`, and `target = target_path`.
/// The `rev` column is set to `"_at_rev_"` — a stable placeholder that
/// the god-class and biomarker CTEs never filter on.
pub fn materialize_imports_at_rev(
    db: &FactsDb,
    graph: &ImportGraph,
    dest_table: &str,
) -> Result<()> {
    db.execute_batch(&format!(
        "CREATE OR REPLACE TEMPORARY TABLE {dest_table} (
            rev         TEXT NOT NULL,
            src_path    TEXT NOT NULL,
            target      TEXT NOT NULL,
            resolved    BOOLEAN NOT NULL,
            target_path TEXT,
            kind        TEXT NOT NULL
        )"
    ))?;

    if graph.is_empty() {
        return Ok(());
    }

    let mut app = db
        .conn()
        .appender(dest_table)
        .map_err(|e| CodeLoreError::Analysis(format!("appender {dest_table}: {e}")))?;

    for (u, neighbors) in graph.adj.iter().enumerate() {
        let src_path = &graph.id_to_path[u];
        for &v in neighbors {
            let target_path = &graph.id_to_path[v];
            app.append_row(duckdb::params![
                "_at_rev_",
                src_path,
                target_path, // resolved path used as the raw target string too
                true,        // resolved = TRUE — ImportGraph only holds resolved edges
                target_path, // target_path
                "absolute",  // kind
            ])
            .map_err(|e| CodeLoreError::Analysis(format!("append {dest_table} row: {e}")))?;
        }
    }

    app.flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush {dest_table}: {e}")))?;
    Ok(())
}
