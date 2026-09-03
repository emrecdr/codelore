//! Rev-parameterized ingest helpers. Materializes complexity metrics and
//! import-graph edges for an arbitrary revision into caller-named temporary
//! tables, enabling the health timeline to build a `HealthScanCtx` that
//! points at historical data instead of the HEAD tables.

use super::coverage::{REASON_BLOB_READ, REASON_PARSE_ERROR, ScanCoverage, ScanOutcome};
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
            total_nesting       INTEGER,
            nargs               INTEGER,
            bool_ops            INTEGER
        )"
    ))?;

    let live_paths: Vec<String> = live_paths.to_vec();
    let rev_owned = rev.to_string();

    // Phase 1 (parallel): read blob at `rev` + tree-sitter parse. One
    // `BlobReader` per rayon worker (`map_init`) resolves the rev's root
    // tree once and keeps a warm object-decode cache — mirroring the
    // at-rev import scan in `analyses/architecture_trend.rs`, whose comment
    // names the per-call `read_blob_at` path as the worse offender.
    // Per-file errors are logged + skipped — a single unreadable file does
    // not abort the scan, matching the same resilience contract as the HEAD
    // pass.
    // Per-file logging existed here already, but nothing tallied it: a rev
    // where every blob failed produced a stream of warnings and then a
    // complexity table indistinguishable from a genuinely clean rev. The
    // aggregate is what a caller can act on — `repo_code_health` returns
    // 100.0 over zero rows, so an unaccounted scan reads as perfect health.
    let outcomes: Vec<ScanOutcome<(String, Vec<crate::complexity::ComplexityEntity>)>> = live_paths
        .into_par_iter()
        .map_init(
            || repo.blob_reader_at(&rev_owned),
            |reader, path| {
                // Not a Tier-1 source file: no row owed, correctly outside
                // the denominator.
                let Some(lang) = Tier1Language::from_path(&path) else {
                    return ScanOutcome::NotCounted;
                };
                let source = match reader.read(&path) {
                    Ok(Some(b)) => b,
                    Ok(None) => {
                        // Absent at this rev — `live_paths` is derived from
                        // history, so this is expected, not a loss.
                        tracing::debug!(
                            "at_rev complexity: {path} not tracked at {rev_owned}; skipping"
                        );
                        return ScanOutcome::NotCounted;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "at_rev complexity: blob read failed for {path} at {rev_owned}: {e}"
                        );
                        return ScanOutcome::Lost(REASON_BLOB_READ);
                    }
                };
                if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                    tracing::debug!(
                        "at_rev complexity: skipping {path} at {rev_owned} \
                         ({size} bytes > {cap}-byte AST cap)",
                        size = source.len(),
                        cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                    );
                    return ScanOutcome::SkippedOversize;
                }
                let synth_path = std::path::Path::new(&path);
                let entities = match compute_for_file(synth_path, source, lang) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("at_rev complexity: parse error {path} at {rev_owned}: {e}");
                        return ScanOutcome::Lost(REASON_PARSE_ERROR);
                    }
                };
                let deduped = dedup_entities(entities);
                // A file with no entities is still covered — read and parsed.
                ScanOutcome::Scored((path, deduped))
            },
        )
        .collect();

    let coverage = ScanCoverage::tally(&outcomes);
    coverage.warn_if_degraded("at-rev complexity", dest_table);
    coverage.warn_if_mostly_oversize("at-rev complexity", dest_table);

    let batches: Vec<Option<(String, Vec<crate::complexity::ComplexityEntity>)>> = outcomes
        .into_iter()
        .map(|o| match o {
            ScanOutcome::Scored(row) => Some(row),
            _ => None,
        })
        .collect();

    // Phase 2 (serial drain): batched multi-row INSERT.
    insert_complexity_rows(db, rev, dest_table, &batches)
}

/// Rows bound per multi-row `INSERT` statement for the at-rev drains.
///
/// `DuckDB` is an OLAP engine: single-row `INSERT ... VALUES` round-trips are
/// pathologically slow (the HEAD path uses the bulk `Appender`, which a
/// read-only connection rejects here). Batching many rows per statement
/// amortizes the per-execute overhead — the historical health timeline drains
/// thousands of complexity rows at every sampled rev, so this is the dominant
/// cost of the trend scan when left un-batched.
const DRAIN_BATCH_ROWS: usize = 256;

/// Build a multi-row `VALUES` clause: `rows` parenthesised groups of `cols`
/// `?` placeholders each, comma-joined — e.g. `values_clause(2, 3)` yields
/// `"(?,?),(?,?),(?,?)"`. `rows == 0` yields the empty string (callers
/// short-circuit before issuing an empty INSERT).
fn values_clause(cols: usize, rows: usize) -> String {
    let group = format!(
        "({})",
        std::iter::repeat_n("?", cols).collect::<Vec<_>>().join(",")
    );
    std::iter::repeat_n(group.as_str(), rows)
        .collect::<Vec<_>>()
        .join(",")
}

/// `Option<f64>` → a `DuckDB` value, binding SQL `NULL` for `None` so the
/// batched drain reproduces the per-row `params!` binding (which bound the
/// `Option` directly) byte-for-byte.
fn opt_double(v: Option<f64>) -> duckdb::types::Value {
    v.map_or(duckdb::types::Value::Null, duckdb::types::Value::Double)
}

/// Drain `rows` into `dest_table` as multi-row `INSERT ... VALUES` batches of
/// [`DRAIN_BATCH_ROWS`], expanding each row to `cols` bound values via
/// `push_row`. ONE prepared statement is reused across every full-size batch
/// (their SQL is identical); only a trailing remainder batch prepares a second
/// right-sized statement. Re-preparing the wide multi-row statement per batch
/// would itself dominate the drain, so the reuse matters.
///
/// A multi-row `INSERT` is used rather than `DuckDB`'s `Appender`: the Appender
/// checks the connection access mode and rejects writes on read-only
/// connections, even for temporary tables. Temporary tables live in an
/// in-memory catalog separate from the file, so SQL `INSERT` goes through a
/// different path and succeeds on a read-only connection.
fn drain_batched<T>(
    db: &FactsDb,
    dest_table: &str,
    cols: usize,
    rows: &[T],
    mut push_row: impl FnMut(&T, &mut Vec<duckdb::types::Value>),
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let full_sql = format!(
        "INSERT INTO {dest_table} VALUES {}",
        values_clause(cols, DRAIN_BATCH_ROWS)
    );
    let mut full_stmt =
        if rows.len() >= DRAIN_BATCH_ROWS {
            Some(db.conn().prepare(&full_sql).map_err(|e| {
                CodeLoreError::Analysis(format!("prepare insert {dest_table}: {e}"))
            })?)
        } else {
            None
        };

    let mut values: Vec<duckdb::types::Value> = Vec::with_capacity(DRAIN_BATCH_ROWS * cols);
    for chunk in rows.chunks(DRAIN_BATCH_ROWS) {
        values.clear();
        for row in chunk {
            push_row(row, &mut values);
        }
        let params: Vec<&dyn duckdb::ToSql> =
            values.iter().map(|v| v as &dyn duckdb::ToSql).collect();
        if chunk.len() == DRAIN_BATCH_ROWS
            && let Some(stmt) = full_stmt.as_mut()
        {
            stmt.execute(params.as_slice())
                .map_err(|e| CodeLoreError::Analysis(format!("insert {dest_table}: {e}")))?;
        } else {
            let sql = format!(
                "INSERT INTO {dest_table} VALUES {}",
                values_clause(cols, chunk.len())
            );
            db.conn()
                .prepare(&sql)
                .map_err(|e| CodeLoreError::Analysis(format!("prepare insert {dest_table}: {e}")))?
                .execute(params.as_slice())
                .map_err(|e| CodeLoreError::Analysis(format!("insert {dest_table}: {e}")))?;
        }
    }
    Ok(())
}

/// Batched drain of the parsed per-file complexity entities into `dest_table`.
///
/// The bound column order mirrors [`super::consumer::append_metric_row`]
/// (the HEAD-time Appender path) — the two write the same
/// `complexity_metrics` shape via different `DuckDB` APIs, so keep the two
/// column orders in sync if the table grows a column.
fn insert_complexity_rows(
    db: &FactsDb,
    rev: &str,
    dest_table: &str,
    batches: &[Option<(String, Vec<crate::complexity::ComplexityEntity>)>],
) -> Result<()> {
    use duckdb::types::Value;

    /// Column count of the `complexity_metrics` shape written below.
    const COLS: usize = 19;

    // Flatten the per-file entity batches into a single row stream, preserving
    // order, then bind them in fixed-size multi-row INSERT chunks.
    let rows: Vec<(&String, &crate::complexity::ComplexityEntity)> = batches
        .iter()
        .filter_map(Option::as_ref)
        .flat_map(|(path, entities)| entities.iter().map(move |ent| (path, ent)))
        .collect();

    drain_batched(db, dest_table, COLS, &rows, |&(path, ent), values| {
        values.push(Value::Text(path.clone()));
        values.push(Value::Text(ent.name.clone()));
        values.push(Value::Text(rev.to_string()));
        values.push(Value::Int(f64_to_i32_clamped(ent.cyclomatic)));
        values.push(Value::Int(f64_to_i32_clamped(ent.cognitive)));
        values.push(opt_double(ent.halstead_volume));
        values.push(opt_double(ent.halstead_difficulty));
        values.push(opt_double(ent.halstead_effort));
        values.push(opt_double(ent.mi));
        values.push(Value::Int(i32::try_from(ent.nom).unwrap_or(i32::MAX)));
        values.push(Value::Int(i32::try_from(ent.nexits).unwrap_or(i32::MAX)));
        values.push(Value::Int(i32::try_from(ent.loc).unwrap_or(i32::MAX)));
        values.push(Value::Int(i32::try_from(ent.sloc).unwrap_or(i32::MAX)));
        values.push(Value::Int(
            i32::try_from(ent.max_nesting).unwrap_or(i32::MAX),
        ));
        values.push(Value::Double(ent.mean_nesting));
        values.push(Value::Double(ent.sd_nesting));
        values.push(Value::Int(
            i32::try_from(ent.total_nesting).unwrap_or(i32::MAX),
        ));
        values.push(Value::Int(i32::try_from(ent.nargs).unwrap_or(i32::MAX)));
        values.push(Value::Int(i32::try_from(ent.bool_ops).unwrap_or(i32::MAX)));
    })
}

/// Write resolved `(src_path, target_path)` import `edges` into a
/// freshly-created temporary table named `dest_table`. The table has the same
/// column shape as `imports` (`rev, src_path, target, resolved, target_path,
/// kind`) without the FK constraint on `rev`.
///
/// The caller passes the resolved edges directly (e.g.
/// `ImportGraph::resolved_edges`) so this `facts::ingest` helper never names
/// an `analyses` type. Every row lands with `resolved = TRUE`,
/// `kind = 'absolute'`, and `target = target_path`; the `rev` column is set to
/// `"_at_rev_"` — a stable placeholder the god-class and biomarker CTEs never
/// filter on.
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
    edges: &[(&str, &str)],
    dest_table: &str,
) -> Result<()> {
    use duckdb::types::Value;

    /// Column count of the `imports` shape written below.
    const COLS: usize = 6;

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

    if edges.is_empty() {
        return Ok(());
    }

    // Drain the resolved edges in fixed-size multi-row INSERT batches.
    drain_batched(
        db,
        dest_table,
        COLS,
        edges,
        |&(src_path, target_path), values| {
            values.push(Value::Text("_at_rev_".to_string()));
            values.push(Value::Text(src_path.to_string()));
            // resolved path used as the raw target string too.
            values.push(Value::Text(target_path.to_string()));
            // resolved = TRUE — only resolved edges are passed in.
            values.push(Value::Boolean(true));
            values.push(Value::Text(target_path.to_string())); // target_path
            values.push(Value::Text("absolute".to_string())); // kind
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complexity::ComplexityEntity;

    #[test]
    fn values_clause_shapes() {
        assert_eq!(values_clause(2, 3), "(?,?),(?,?),(?,?)");
        assert_eq!(values_clause(6, 1), "(?,?,?,?,?,?)");
        assert_eq!(values_clause(19, 0), "");
    }

    fn entity(name: &str, cognitive: f64, mi: Option<f64>) -> ComplexityEntity {
        ComplexityEntity {
            path: String::new(), // unused by the drain (path comes from the batch tuple)
            name: name.to_string(),
            kind: "function".to_string(),
            start_line: 0,
            end_line: 0,
            cyclomatic: 1.0,
            cognitive,
            halstead_volume: Some(1.0),
            halstead_difficulty: None, // exercises Option→NULL binding
            halstead_effort: Some(3.0),
            mi,
            nom: 1,
            nexits: 1,
            nargs: 0,
            loc: 10,
            sloc: 8,
            max_nesting: 2,
            mean_nesting: 0.0,
            sd_nesting: 0.0,
            total_nesting: 3,
            bool_ops: 0,
        }
    }

    fn create_dest(db: &FactsDb, name: &str) {
        db.execute_batch(&format!(
            "CREATE OR REPLACE TEMPORARY TABLE {name} (
                path TEXT NOT NULL, name TEXT NOT NULL, rev TEXT NOT NULL,
                cyclomatic INTEGER, cognitive INTEGER, halstead_volume DOUBLE,
                halstead_difficulty DOUBLE, halstead_effort DOUBLE, mi DOUBLE,
                nom INTEGER, nexits INTEGER, loc INTEGER, sloc INTEGER,
                max_nesting INTEGER, mean_nesting DOUBLE, sd_nesting DOUBLE,
                total_nesting INTEGER, nargs INTEGER, bool_ops INTEGER
            )"
        ))
        .unwrap();
    }

    #[test]
    fn insert_complexity_rows_batches_across_boundary_and_remainder() {
        let db = FactsDb::new_in_memory().unwrap();
        create_dest(&db, "cm_test");

        // One full DRAIN_BATCH_ROWS chunk plus a 5-row remainder, split across
        // two file batches with a `None` batch interleaved (must be skipped).
        // The boundary lands mid-file so a single chunk spans both files.
        let n = DRAIN_BATCH_ROWS + 5;
        let mut first: Vec<ComplexityEntity> = Vec::new();
        for i in 0..n - 1 {
            let mi = if i % 2 == 0 { Some(1.0) } else { None };
            first.push(entity(&format!("f{i}"), 1.0, mi));
        }
        let batches = vec![
            Some(("a.rs".to_string(), first)),
            None,
            Some(("b.rs".to_string(), vec![entity("marker", 42.0, Some(7.5))])),
        ];
        insert_complexity_rows(&db, "deadbeef", "cm_test", &batches).unwrap();

        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM cm_test", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, i64::try_from(n).unwrap());

        // Spot-check the marker row (last row, in the remainder chunk): rev
        // bound, cognitive clamped to i32, mi kept.
        let (rev, cog, mi): (String, i32, f64) = db
            .query_row(
                "SELECT rev, cognitive, mi FROM cm_test WHERE path = 'b.rs' AND name = 'marker'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(rev, "deadbeef");
        assert_eq!(cog, 42);
        assert!((mi - 7.5).abs() < 1e-9);

        // Every row's halstead_difficulty was `None` → SQL NULL.
        let null_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM cm_test WHERE halstead_difficulty IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(null_count, i64::try_from(n).unwrap());
    }
}
