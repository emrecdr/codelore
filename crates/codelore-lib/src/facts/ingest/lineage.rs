//! Rename-lineage materialization. Builds the recursive `path_lineage` lookup
//! that maps every historically-renamed path to its latest canonical name, and
//! the `changes_lineage` view that rewrites `changes.path` through that map so
//! opt-in analyses aggregate rename-aware.

use super::FactsDb;
use crate::{CodeLoreError, Result};

/// Materialize the rename-lineage map as a temporary table.
///
/// Walks `changes.rename_from` recursively to find the LATEST canonical path
/// for every path that has ever been renamed. `path_lineage` is a small
/// `(old_path, canonical_path)` lookup table — typically a handful of rows
/// even on large repos (renames are rare). Cycles are bounded by `depth < 50`,
/// far above any realistic rename chain; the `ROW_NUMBER() ... ORDER BY depth
/// DESC` deterministically picks the last name in the chain. Rows where
/// `old_path == canonical_path` are filtered out — the join then has nothing
/// to merge for files that have never been renamed (the common case).
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_path_lineage(db: &FactsDb) -> Result<()> {
    use duckdb::params;
    // The recursive CTE walks the rename graph, but a NAIVE join on
    // `c.rename_from = l.current` would conflate a recycled filename with
    // its earlier life. Example: commit 1 renames `A → B`; commit 10 takes
    // a different file `C` and renames it `C → A`. Without a chronological
    // constraint the CTE would walk `(C, A, depth=1) → (C, B, depth=2)`
    // by joining `c.rename_from = 'A'` on commit 1's row — producing a
    // fictitious `C → A → B` lineage that merges two unrelated files'
    // history into one entity.
    //
    // The fix joins each step with `commits.date` and only extends the
    // chain when the NEXT rename happened AFTER the current step's date.
    // Date is fetched via `INNER JOIN commits ON commits.rev = c.rev` in
    // both the seed and the recursive step.
    //
    // Same-second tiebreak: strict `>` on date would terminate a chain
    // where two sequential renames (A → B then B → C) land in commits
    // sharing the exact same second. Carry `commits.rowid` through the CTE
    // and break date-ties via `co.rowid < l.current_rowid`: gix walks
    // reverse-chronologically (HEAD first), so newer commits receive
    // smaller rowids during ingest; the next step in a forward rename
    // chain must come from a newer commit, hence the smaller rowid.
    let sql = "CREATE OR REPLACE TEMPORARY TABLE path_lineage AS
        WITH RECURSIVE lineage(orig, current, current_date, current_rowid, depth) AS (
            SELECT DISTINCT c.rename_from, c.path, co.date, co.rowid, 1
            FROM changes c
            INNER JOIN commits co ON co.rev = c.rev
            WHERE c.rename_from IS NOT NULL
            UNION ALL
            SELECT l.orig, c.path, co.date, co.rowid, l.depth + 1
            FROM lineage l
            INNER JOIN changes c ON c.rename_from = l.current
            INNER JOIN commits co ON co.rev = c.rev
            WHERE l.depth < 50
              AND (co.date > l.current_date
                   OR (co.date = l.current_date AND co.rowid < l.current_rowid))
        )
        SELECT orig AS old_path, current AS canonical_path
        FROM (
            SELECT orig, current, depth,
                   ROW_NUMBER() OVER (
                       PARTITION BY orig
                       -- Secondary order on `current` so ties at the same
                       -- depth (possible when a non-linear rename graph
                       -- reaches the same intermediate via multiple paths)
                       -- break deterministically and run-to-run output stays
                       -- byte-equal.
                       ORDER BY depth DESC, current ASC
                   ) AS rn
            FROM lineage
        )
        WHERE rn = 1 AND orig != current";
    db.conn()
        .execute(sql, params![])
        .map_err(|e| CodeLoreError::Analysis(format!("materialize path_lineage: {e}")))?;
    Ok(())
}

/// Materialize `changes_lineage` — `changes` with `path` canonicalized via
/// the rename-lineage map. Calls [`materialize_path_lineage`] first so the
/// lookup table is in scope.
///
/// Built once per fact store: the view's content is a pure function of the
/// immutable `changes` / `commits` tables, so a per-`FactsDb` guard skips
/// the recursive CTE + full table copy + index builds on every call after
/// the first (12+ lineage-opt-in callers under `--use-canonical-lineage`
/// otherwise each rebuilt it). `apply_grouping`'s in-place `changes` swap
/// invalidates the guard so the post-grouping rebuild still happens exactly
/// once.
///
/// Analyses that opt into rename-aware aggregation should `FROM
/// changes_lineage` instead of `FROM changes`. The schema is identical
/// modulo `path` being the post-rename canonical name.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_changes_lineage(db: &FactsDb) -> Result<()> {
    use duckdb::params;
    if db.is_changes_lineage_built() {
        return Ok(());
    }
    materialize_path_lineage(db)?;
    let sql = "CREATE OR REPLACE TEMPORARY TABLE changes_lineage AS
        SELECT
            c.rev,
            COALESCE(pl.canonical_path, c.path) AS path,
            c.change_type,
            c.rename_from,
            c.similarity,
            c.loc_added,
            c.loc_deleted
        FROM changes c
        LEFT JOIN path_lineage pl ON pl.old_path = c.path";
    db.conn()
        .execute(sql, params![])
        .map_err(|e| CodeLoreError::Analysis(format!("materialize changes_lineage: {e}")))?;
    // Without explicit indexes the downstream GROUP BY path / GROUP BY rev
    // aggregations fall to full scans even when the base `changes` table
    // has covering indexes. Mirror them on the temp table.
    for stmt in [
        "CREATE INDEX IF NOT EXISTS idx_changes_lineage_path ON changes_lineage(path)",
        "CREATE INDEX IF NOT EXISTS idx_changes_lineage_rev ON changes_lineage(rev)",
    ] {
        db.conn()
            .execute(stmt, params![])
            .map_err(|e| CodeLoreError::Analysis(format!("index changes_lineage: {e}")))?;
    }
    db.mark_changes_lineage_built();
    tracing::info!("materialized changes_lineage with canonical rename paths");
    Ok(())
}
