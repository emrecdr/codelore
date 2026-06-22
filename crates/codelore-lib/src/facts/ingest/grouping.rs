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
             ANY_VALUE(c.rename_from) AS rename_from, \
             ANY_VALUE(c.similarity) AS similarity, \
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
///   4. Replace `changes` content with the aggregated rows.
///   5. Remove `hunks` rows whose `(rev, path)` no longer exists in
///      `changes` (strict mode + dropped paths produces orphans otherwise).
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

        // Step 2b: serial INSERT (DuckDB Connection is !Send + !Sync).
        let mut stmt = conn
            .prepare("INSERT INTO _grouping_v1 (raw_path, group_name) VALUES (?, ?)")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare grouping insert: {e}")))?;
        for (raw, effective) in &mapped {
            stmt.execute(params![raw, effective.as_deref()])
                .map_err(|e| CodeLoreError::Analysis(format!("grouping insert row: {e}")))?;
        }
    }

    // Step 3+4: rewrite `changes` in place. CREATE OR REPLACE TEMPORARY
    // TABLE _changes_grouped + DELETE+INSERT pattern keeps the FK from
    // hunks happy in step 5 (no period where changes is empty AND the
    // grouped data isn't yet ready to INSERT).
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _changes_grouped AS \
         SELECT \
             c.rev, \
             g.group_name AS path, \
             MAX(c.change_type) AS change_type, \
             ANY_VALUE(c.rename_from) AS rename_from, \
             ANY_VALUE(c.similarity) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM changes c \
         INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
         WHERE g.group_name IS NOT NULL \
         GROUP BY c.rev, g.group_name",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("build _changes_grouped: {e}")))?;

    // Step 5: clean hunks for paths that won't survive the swap. Do BEFORE
    // the changes-swap so the FK from hunks → changes never sees a missing
    // referent. Hunks aren't path-rewritten (line-range semantics don't
    // translate to group level), so they get dropped for any path that
    // collapsed or got removed.
    //
    // The previous `NOT IN (SELECT … )` form on a composite key
    // forces some `DuckDB` planner paths into a per-row subquery scan
    // rather than the index-friendly hash anti-join. `NOT EXISTS` with
    // a correlated `h.rev = c.rev AND h.path = c.path` predicate is the
    // textbook planner-friendly form: same semantics (both projected
    // columns are `NOT NULL` per `schema_v1.sql`, so `NOT IN`'s NULL
    // pitfall doesn't apply, but `NOT EXISTS` is uniformly preferred
    // across `DuckDB` versions). The existing
    // `apply_grouping_*` integration tests cover the semantic
    // equivalence — both forms must drop the same hunks for the same
    // group-map input.
    conn.execute(
        "DELETE FROM hunks h \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM changes c \
             INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
             WHERE g.group_name = c.path \
               AND c.rev = h.rev \
               AND c.path = h.path \
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("clean hunks: {e}")))?;

    // Swap the data in changes
    conn.execute("DELETE FROM changes", [])
        .map_err(|e| CodeLoreError::Analysis(format!("clear changes: {e}")))?;
    conn.execute(
        "INSERT INTO changes (rev, path, change_type, rename_from, similarity, loc_added, loc_deleted) \
         SELECT rev, path, change_type, rename_from, similarity, loc_added, loc_deleted \
         FROM _changes_grouped",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("swap changes: {e}")))?;

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

/// Build `complexity_metrics_grouped` from the raw `complexity_metrics`
/// and `entities` tables joined through `_grouping_v1`. One permanent
/// row per group, with `MAX` cognitive across all entities of all
/// files in the group and `MAX` MI restricted to `kind='unit'` rows
/// (the file-level Maintainability Index per the
/// `rust-code-analysis` convention). `MAX` is the right rollup because
/// each consuming analysis treats cognitive as a "worst function" risk
/// signal; `MAX(MAX) = MAX` composes to "worst function anywhere in
/// this group".
///
/// Stored as a permanent (not TEMPORARY) table so it survives cache
/// replay — on cache hit, `FactsDb::open_read_only` opens a fresh
/// connection that has no access to `_grouping_v1`, but the persisted
/// rollup table is in the `DuckDB` file and is what the analyses read
/// via `crate::analyses::grouped_complexity::source_table`.
fn materialize_complexity_grouped(conn: &duckdb::Connection) -> Result<()> {
    conn.execute("DROP TABLE IF EXISTS complexity_metrics_grouped", [])
        .map_err(|e| CodeLoreError::Analysis(format!("drop complexity_metrics_grouped: {e}")))?;

    conn.execute(
        "CREATE TABLE complexity_metrics_grouped AS \
         WITH cog AS ( \
             SELECT g.group_name AS path, MAX(cm.cognitive)::INTEGER AS cognitive \
             FROM complexity_metrics cm \
             INNER JOIN _grouping_v1 g ON g.raw_path = cm.path \
             WHERE g.group_name IS NOT NULL AND cm.cognitive IS NOT NULL \
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
         SELECT COALESCE(cog.path, mi.path) AS path, cog.cognitive, mi.mi \
         FROM cog FULL OUTER JOIN mi ON cog.path = mi.path",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("materialize complexity_metrics_grouped: {e}")))?;

    Ok(())
}
