//! Architectural grouping and time-bucketing materializations. Rewrites
//! `changes.path` in place to logical group names per a `GroupMap`, rolls up
//! the matching per-group complexity metrics, and collapses commits within a
//! time window into a single logical commit for the coupling-family analyses.

use super::FactsDb;
use crate::facts::GroupMap;
use crate::options::TimeBucket;
use crate::{CodeLoreError, Result};

/// Materialize a session-temporary `changes_bucketed` table that collapses
/// commits within the same `date_trunc(<bucket>, commit.date)` window into
/// a single logical "commit" per (bucket, path). Coupling-family analyses
/// (`coupling`, `clone-coupling` indirectly, `soc`) query this table when
/// `opts.time_bucket.is_some()` so commits landed across the same
/// day/week/month count as one for pair-counting purposes.
///
/// The bucket key (a date string like `2024-01-15` for day-buckets) takes
/// the place of `rev`. Within a bucket, `loc_added` and `loc_deleted` are
/// summed; `change_type` collapses to MAX (string-alphabetical max — close
/// enough since the bucketed-table is consumed only by analyses that care
/// about pair counts, not type details).
///
/// Idempotent: `CREATE OR REPLACE TEMPORARY TABLE`. Call once per analysis
/// run after the main ingest finishes. Cheap — single SQL pass over
/// `changes` JOIN `commits`.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_changes_bucketed(
    db: &FactsDb,
    bucket: TimeBucket,
    use_lineage: bool,
) -> Result<()> {
    use duckdb::params;
    let unit = bucket.as_sql_unit();
    // When canonical lineage is on, bucket on top of the lineage-resolved
    // view so rename ancestry survives the temporal collapse. Without this,
    // a renamed file's pre- and post-rename commits aggregate under
    // separate paths inside the same bucket — the composition bug §2.1
    // flagged by the 2026-06-09 deep-analysis report.
    let src = if use_lineage {
        super::lineage::materialize_changes_lineage(db)?;
        "changes_lineage"
    } else {
        "changes"
    };
    // unit + src come from closed enums / a small set of literals so the
    // format! interpolation is safe (no user-controlled input).
    let sql = format!(
        "CREATE OR REPLACE TEMPORARY TABLE changes_bucketed AS \
         SELECT \
             CAST(date_trunc('{unit}', m.date) AS TEXT) AS rev, \
             c.path, \
             MAX(c.change_type) AS change_type, \
             arg_max(c.rename_from, ROW(m.date, -m.rowid)) AS rename_from, \
             arg_max(c.similarity, ROW(m.date, -m.rowid)) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM {src} c \
         INNER JOIN commits m ON m.rev = c.rev \
         GROUP BY date_trunc('{unit}', m.date), c.path"
    );
    db.conn().execute(&sql, params![]).map_err(|e| {
        CodeLoreError::Analysis(format!("materialize changes_bucketed ({unit}): {e}"))
    })?;
    tracing::info!("materialized changes_bucketed at {unit} granularity (lineage={use_lineage})");
    Ok(())
}

/// Apply architectural grouping in-place on the `changes` table. Called by
/// [`FactsDb::ingest`] after raw ingest if `opts.group_file.is_some()`.
///
/// Implementation:
///   1. Build a `(raw_path → group_name)` mapping in Rust from every distinct
///      path in `changes` against the [`GroupMap`].
///   2. Insert the mapping into a temporary table.
///   3. Build a `changes_grouped` temporary table that JOINs against the
///      mapping, replaces the path with the group name (or keeps raw under
///      non-strict mode for unmapped paths), and aggregates `loc_added` /
///      `loc_deleted` per `(rev, new_path)`.
///   4. Snapshot the `hunks` rows whose path survives under its own name
///      (identity-mapped / non-strict unmapped paths); hunks of collapsed
///      or dropped paths are discarded (line-range semantics don't
///      translate to group level).
///   5. Swap by rebuild: DROP `hunks` (child first) and `changes`,
///      re-run the idempotent schema script to recreate them, then
///      insert the aggregated rows and restore the snapshotted hunks.
///      DELETE-based swaps can't work here — `DuckDB` checks the
///      `hunks → changes` FK immediately per statement AND verifies it
///      against index entries that persist for already-deleted rows, so
///      both delete orders trip the FK machinery.
///
/// Strict vs non-strict:
/// - Strict (`opts.strict_grouping = true` / code-maat default): rows whose
///   path doesn't match any rule are DROPPED.
/// - Non-strict (`CodeLore` default): unmapped rows keep their raw path.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn apply_grouping(db: &FactsDb, group_map: &GroupMap) -> Result<()> {
    use duckdb::params;

    let conn = db.conn();

    // Step 1: enumerate distinct paths in `changes` and pre-compute the
    // mapping in Rust. Doing the regex matching here avoids embedding the
    // GroupMap rules into SQL (DuckDB has regexp_matches but doesn't
    // support fancy-regex's lookaround that some code-maat fixtures need).
    let distinct_paths: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT path FROM changes")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare distinct paths: {e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CodeLoreError::Analysis(format!("query distinct paths: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect distinct paths: {e}")))?
    };

    // Step 2: build the mapping table. Mapped paths get the group name; in
    // non-strict mode, unmapped paths get the raw path back; in strict mode,
    // unmapped paths get a sentinel NULL group_name that the WHERE filter
    // in step 3 uses to drop them.
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _grouping_v1 (\
             raw_path TEXT PRIMARY KEY, group_name TEXT\
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("create _grouping_v1: {e}")))?;

    {
        // Step 2a: regex-match every distinct path against the group map
        // in parallel. The regex set is `Send + Sync` (immutable post-build),
        // and `Vec<String>` shares immutable refs across rayon workers.
        // Pre-`f41` this loop ran sequentially on the main thread; for
        // monorepos with N×M = paths × rules in the high millions, the
        // single-threaded matching dominated `apply_grouping` wall-clock.
        use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
        let strict = group_map.strict;
        let mapped: Vec<(String, Option<String>)> = distinct_paths
            .par_iter()
            .map(|path| {
                let raw = path.clone();
                let effective: Option<String> = if strict {
                    group_map.map_entity(path).map(str::to_owned)
                } else {
                    Some(
                        group_map
                            .map_entity(path)
                            .map_or_else(|| path.clone(), str::to_owned),
                    )
                };
                (raw, effective)
            })
            .collect();

        // Step 2b: bulk-append via the DuckDB Appender (Connection is
        // !Send + !Sync, so this drains serially on the owning thread — the
        // same shape the rest of ingest uses, instead of one prepared
        // INSERT execution per distinct path, which on a large monorepo is
        // the whole file universe).
        let mut app = conn
            .appender("_grouping_v1")
            .map_err(|e| CodeLoreError::Analysis(format!("appender _grouping_v1: {e}")))?;
        for (raw, effective) in &mapped {
            app.append_row(params![raw, effective.as_deref()])
                .map_err(|e| CodeLoreError::Analysis(format!("grouping append row: {e}")))?;
        }
        app.flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush _grouping_v1 appender: {e}")))?;
    }

    // Step 3: build the grouped replacement content in a temporary table
    // so the swap below is a pair of short statements against fully
    // pre-aggregated rows.
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _changes_grouped AS \
         SELECT \
             c.rev, \
             g.group_name AS path, \
             MAX(c.change_type) AS change_type, \
             arg_max(c.rename_from, c.path) AS rename_from, \
             arg_max(c.similarity, c.path) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM changes c \
         INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
         WHERE g.group_name IS NOT NULL \
         GROUP BY c.rev, g.group_name",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("build _changes_grouped: {e}")))?;

    // Steps 4+5: snapshot surviving hunks, then rebuild-and-repopulate
    // `changes` + `hunks` with the grouped content.
    swap_grouped_tables(conn)?;

    // `changes.path` was just rewritten in place; any `changes_lineage`
    // built earlier (e.g. by kamei during ingest) now reflects the
    // pre-grouping paths, so invalidate the guard — the next lineage-opt-in
    // analysis rebuilds the view exactly once against the grouped paths.
    db.invalidate_changes_lineage();

    // Step 6: materialise per-group `MAX(cognitive)` + `MAX(unit-MI)`
    // rollups so the four path-aggregating analyses that join
    // `complexity_metrics` (hotspots, code_health, god_classes,
    // stale_code) see grouped paths in the same key space as the
    // rewritten `changes.path`. Without this, those analyses LEFT
    // JOIN raw-path complexity rows against group-name change rows
    // and silently report `0` cognitive for every grouped entity.
    // `_grouping_v1` must still exist (TEMPORARY TABLE,
    // connection-scoped) so this step runs HERE inside
    // `apply_grouping`, not at analysis time.
    materialize_complexity_grouped(conn)?;

    tracing::info!(
        "grouping applied: {} rules, {} distinct paths, strict={}",
        group_map.rules.len(),
        distinct_paths.len(),
        group_map.strict
    );

    Ok(())
}

/// Swap the `changes` content for the pre-aggregated `_changes_grouped`
/// rows while preserving the hunks of paths that survive under their own
/// name.
///
/// A hunk survives iff its path maps to itself (`g.group_name = c.path`
/// — identity-mapped rules and, in non-strict mode, unmapped paths kept
/// under their raw name). Hunks aren't path-rewritten (line-range
/// semantics don't translate to group level), so hunks of collapsed or
/// dropped paths are discarded. The snapshot runs BEFORE either table is
/// touched.
///
/// The swap itself rebuilds rather than deletes: `DuckDB` checks the
/// `hunks → changes` FK immediately per statement — there is no
/// deferred-check window — so any surviving hunk row would make
/// `DELETE FROM changes` fail. Nor is child-first DELETE enough:
/// `DuckDB` verifies the FK against index entries that persist for
/// already-deleted rows (its documented foreign-key limitation), so
/// clearing `hunks` and then `changes` still trips the FK machinery on
/// real-scale ingests. DROP both tables (child first) and re-run the
/// idempotent schema script instead — every statement in it is
/// `IF NOT EXISTS`, so only the two dropped tables (and their indexes)
/// are rebuilt, pristine. The hunks restore cannot violate the FK by
/// construction: every surviving hunk's path maps to itself, so
/// `_changes_grouped` carries a `(rev, path)` row for it under the same
/// name.
fn swap_grouped_tables(conn: &duckdb::Connection) -> Result<()> {
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _hunks_surviving AS \
         SELECT h.* FROM hunks h \
         WHERE EXISTS ( \
             SELECT 1 FROM changes c \
             INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
             WHERE g.group_name = c.path \
               AND c.rev = h.rev \
               AND c.path = h.path \
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("snapshot surviving hunks: {e}")))?;

    conn.execute("DROP TABLE hunks", [])
        .map_err(|e| CodeLoreError::Analysis(format!("drop hunks for swap: {e}")))?;
    conn.execute("DROP TABLE changes", [])
        .map_err(|e| CodeLoreError::Analysis(format!("drop changes for swap: {e}")))?;
    conn.execute_batch(crate::facts::schema::SCHEMA_V1)
        .map_err(|e| CodeLoreError::Analysis(format!("recreate swapped tables: {e}")))?;

    conn.execute(
        "INSERT INTO changes (rev, path, change_type, rename_from, similarity, loc_added, loc_deleted) \
         SELECT rev, path, change_type, rename_from, similarity, loc_added, loc_deleted \
         FROM _changes_grouped",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("swap changes: {e}")))?;
    // `_hunks_surviving` mirrors the `hunks` column order (`SELECT h.*`
    // above), so the positional INSERT stays aligned with the schema.
    // Left in place afterwards, like `_grouping_v1` — TEMPORARY tables
    // are connection-scoped and vanish with the ingest session.
    conn.execute("INSERT INTO hunks SELECT * FROM _hunks_surviving", [])
        .map_err(|e| CodeLoreError::Analysis(format!("restore surviving hunks: {e}")))?;
    Ok(())
}

/// Build `complexity_metrics_grouped` from the raw `complexity_metrics`
/// and `entities` tables joined through `_grouping_v1`. One permanent
/// row per group.
///
/// The table's contract: it must carry every column ANY `{cm_src}`
/// consumer binds (see the grouped-complexity analysis), with
/// "worst function anywhere in this group" rollup semantics. The widest
/// consumer is `code_health` — its biomarker INSERT binds `path`,
/// `name`, `cyclomatic`, `loc`, `max_nesting`, `nargs`, `bool_ops`, and
/// its corpus-aggregate pass binds `cyclomatic`, `cognitive`, `sloc`,
/// `nargs`, `max_nesting`; `hotspots`, `god_classes`, and `stale_code`
/// read only `cognitive` (plus `mi` on the grouped hotspots path).
/// `MAX` is the right rollup because each consumer treats the metric as
/// a per-file "worst function" risk signal, and `MAX(MAX) = MAX`
/// composes to "worst function anywhere in this group"; `MAX` skips
/// NULLs per column, so a metric absent on some functions still rolls
/// up from the rest. `name` is `NULL` — the biomarker INSERT binds the
/// column but never uses it past the binder, and a rolled-up group row
/// has no single function name. `mi` keeps the `kind='unit'`-restricted
/// `MAX` through the `entities` join (the file-level Maintainability
/// Index per the `rust-code-analysis` convention).
///
/// Group names carry no file extension, so `code_health`'s language
/// CASE buckets every group as `other`: biomarker `PERCENT_RANK`s rank
/// groups against groups (self-consistent), and the corpus lens
/// resolves no percentile for groups (honest absence — corpus tables
/// are keyed by real languages).
///
/// Stored as a permanent (not TEMPORARY) table so it survives cache
/// replay — on cache hit, `FactsDb::open_read_only` opens a fresh
/// connection that has no access to `_grouping_v1`, but the persisted
/// rollup table is in the `DuckDB` file and is what the analyses read
/// via the grouped-complexity source-table selector.
fn materialize_complexity_grouped(conn: &duckdb::Connection) -> Result<()> {
    conn.execute("DROP TABLE IF EXISTS complexity_metrics_grouped", [])
        .map_err(|e| CodeLoreError::Analysis(format!("drop complexity_metrics_grouped: {e}")))?;

    conn.execute(
        "CREATE TABLE complexity_metrics_grouped AS \
         WITH fn_metrics AS ( \
             SELECT \
                 g.group_name AS path, \
                 MAX(cm.cognitive)::INTEGER AS cognitive, \
                 MAX(cm.cyclomatic)::INTEGER AS cyclomatic, \
                 MAX(cm.loc)::INTEGER AS loc, \
                 MAX(cm.sloc)::INTEGER AS sloc, \
                 MAX(cm.nargs)::INTEGER AS nargs, \
                 MAX(cm.max_nesting)::INTEGER AS max_nesting, \
                 MAX(cm.bool_ops)::INTEGER AS bool_ops \
             FROM complexity_metrics cm \
             INNER JOIN _grouping_v1 g ON g.raw_path = cm.path \
             WHERE g.group_name IS NOT NULL \
             GROUP BY g.group_name \
         ), \
         mi AS ( \
             SELECT g.group_name AS path, MAX(cm.mi)::DOUBLE AS mi \
             FROM complexity_metrics cm \
             INNER JOIN entities e \
                 ON e.path = cm.path AND e.name = cm.name AND e.rev_last_seen = cm.rev \
             INNER JOIN _grouping_v1 g ON g.raw_path = cm.path \
             WHERE g.group_name IS NOT NULL AND e.kind = 'unit' AND cm.mi IS NOT NULL \
             GROUP BY g.group_name \
         ) \
         SELECT \
             COALESCE(f.path, mi.path) AS path, \
             NULL::TEXT AS name, \
             f.cognitive, f.cyclomatic, f.loc, f.sloc, \
             f.nargs, f.max_nesting, f.bool_ops, \
             mi.mi \
         FROM fn_metrics f FULL OUTER JOIN mi ON f.path = mi.path",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("materialize complexity_metrics_grouped: {e}")))?;

    Ok(())
}
