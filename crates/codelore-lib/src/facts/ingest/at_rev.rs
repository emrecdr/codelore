//! Rev-parameterized ingest helpers. Materializes complexity metrics and
//! import-graph edges for an arbitrary revision into caller-named temporary
//! tables, enabling the health timeline to build a `HealthScanCtx` that
//! points at historical data instead of the HEAD tables.

use crate::analyses::import_graph::ImportGraph;
use crate::{CodeLoreError, Result};

use super::FactsDb;
use super::consumer::{dedup_entities, f64_to_i32_clamped};

/// Scan Tier-1 source blobs at `rev`, compute complexity metrics, and write
/// the results into a freshly-created temporary table named `dest_table`.
/// The temporary table has the same column shape as `complexity_metrics`.
///
/// `live_paths` is the caller-supplied slice of repo-relative paths that
/// exist at `rev`. Each path is checked via `repo.read_blob_at(rev, path)`;
/// paths absent at that revision are silently skipped.
///
/// Mirrors `ingest_complexity_at_head`'s rayon-parallel + serial-drain
/// pattern: blob reads and parsing run in parallel; the INSERT drain runs
/// serially on the connection-owning thread via a prepared statement.
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

    // Phase 2 (serial drain): INSERT via prepared statement.
    // DuckDB's Appender checks the connection access mode and rejects writes
    // on read-only connections, even for temporary tables. Temporary tables
    // live in an in-memory catalog separate from the file, so SQL INSERT goes
    // through a different path and succeeds on read-only connections.
    let mut stmt = db
        .conn()
        .prepare(&format!(
            "INSERT INTO {dest_table} VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
        ))
        .map_err(|e| CodeLoreError::Analysis(format!("prepare insert {dest_table}: {e}")))?;

    for batch in batches {
        let Some((path, entities)) = batch else {
            continue;
        };
        for ent in &entities {
            stmt.execute(duckdb::params![
                path,
                ent.name,
                rev,
                f64_to_i32_clamped(ent.cyclomatic),
                f64_to_i32_clamped(ent.cognitive),
                ent.halstead_volume,
                ent.halstead_difficulty,
                ent.halstead_effort,
                ent.mi,
                i32::try_from(ent.nom).unwrap_or(i32::MAX),
                i32::try_from(ent.nexits).unwrap_or(i32::MAX),
                i32::try_from(ent.loc).unwrap_or(i32::MAX),
                i32::try_from(ent.sloc).unwrap_or(i32::MAX),
                i32::try_from(ent.max_nesting).unwrap_or(i32::MAX),
                ent.mean_nesting,
                ent.sd_nesting,
                i32::try_from(ent.total_nesting).unwrap_or(i32::MAX),
            ])
            .map_err(|e| CodeLoreError::Analysis(format!("insert {dest_table}: {e}")))?;
        }
    }
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
///
/// Consequence for callers: the live `imports` table also holds
/// *unresolved* external edges (`resolved = FALSE`, e.g. std-lib / package
/// imports), and the god-class `fan_out` CTE counts them via
/// `COUNT(DISTINCT target)` without a `resolved` filter. This table omits
/// them, so a god-class `fan_out` (and the `god_score` it feeds) computed
/// against this source is resolved-only and under-counts relative to a
/// live-`imports` HEAD scan — by however many external imports each file
/// makes. A timeline that builds *every* sample (including the newest)
/// through this helper stays internally consistent; do not compare its
/// most-recent point to the standalone HEAD `code-health` number, which
/// counts external fan-out.
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

    // INSERT via prepared statement — same rationale as ingest_complexity_at_rev:
    // DuckDB's Appender is blocked on read-only connections; SQL INSERT into a
    // temporary table works because temp tables are in-memory (not file-backed).
    let mut stmt = db
        .conn()
        .prepare(&format!("INSERT INTO {dest_table} VALUES (?,?,?,?,?,?)"))
        .map_err(|e| CodeLoreError::Analysis(format!("prepare insert {dest_table}: {e}")))?;

    for (u, neighbors) in graph.adj.iter().enumerate() {
        let src_path = &graph.id_to_path[u];
        for &v in neighbors {
            let target_path = &graph.id_to_path[v];
            stmt.execute(duckdb::params![
                "_at_rev_",
                src_path,
                target_path, // resolved path used as the raw target string too
                true,        // resolved = TRUE — ImportGraph only holds resolved edges
                target_path, // target_path
                "absolute",  // kind
            ])
            .map_err(|e| CodeLoreError::Analysis(format!("insert {dest_table} row: {e}")))?;
        }
    }
    Ok(())
}
