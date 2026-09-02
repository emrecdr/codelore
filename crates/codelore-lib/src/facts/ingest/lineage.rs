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
/// lookup table — typically a handful of rows even on large repos (renames
/// are rare). Cycles are bounded by `depth < 50`, far above any realistic
/// rename chain; the `ROW_NUMBER() ... ORDER BY depth DESC` deterministically
/// picks the last name in the chain. Rows where `old_path == canonical_path`
/// are filtered out — the join then has nothing to merge for files that have
/// never been renamed (the common case).
///
/// A name can be retired MORE THAN ONCE (`a → b`, later a new unrelated
/// file is created as `a` and renamed `a → c`), so the map carries one row
/// per *retirement epoch*: `(old_path, canonical_path, retired_date,
/// retired_rowid, prev_retired_date, prev_retired_rowid)`. The `retired_*`
/// pair is the commit that renamed this epoch's file away from `old_path`;
/// the `prev_retired_*` pair is the previous epoch's retirement (NULL for
/// the first). Together they bound the half-open window of history rows
/// that belong to this epoch's file, which is what lets the application
/// join in [`materialize_changes_lineage`] avoid canonicalizing a recycled
/// filename's NEW file onto the OLD file's lineage target.
///
/// Only `change_type = 'renamed'` rows seed or extend chains: a `copied`
/// row also carries `rename_from`, but a copy does not retire its source —
/// the source lives on and keeps its own history.
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
        WITH RECURSIVE lineage(
            orig, current, current_date, current_rowid,
            retired_date, retired_rowid, depth
        ) AS (
            SELECT DISTINCT c.rename_from, c.path, co.date, co.rowid,
                            co.date, co.rowid, 1
            FROM changes c
            INNER JOIN commits co ON co.rev = c.rev
            WHERE c.rename_from IS NOT NULL AND c.change_type = 'renamed'
            UNION ALL
            SELECT l.orig, c.path, co.date, co.rowid,
                   l.retired_date, l.retired_rowid, l.depth + 1
            FROM lineage l
            INNER JOIN changes c
                    ON c.rename_from = l.current
                   AND c.change_type = 'renamed'
            INNER JOIN commits co ON co.rev = c.rev
            WHERE l.depth < 50
              AND (co.date > l.current_date
                   OR (co.date = l.current_date AND co.rowid < l.current_rowid))
        ),
        resolved AS (
            SELECT orig AS old_path, current AS canonical_path,
                   retired_date, retired_rowid,
                   ROW_NUMBER() OVER (
                       -- One winner per (orig, retirement epoch): each seed
                       -- rename resolves its own chain independently, so a
                       -- twice-retired name maps each epoch to its own
                       -- canonical target.
                       PARTITION BY orig, retired_date, retired_rowid
                       -- Secondary order on `current` so ties at the same
                       -- depth (possible when a non-linear rename graph
                       -- reaches the same intermediate via multiple paths)
                       -- break deterministically and run-to-run output stays
                       -- byte-equal.
                       ORDER BY depth DESC, current ASC
                   ) AS rn
            FROM lineage
        ),
        epochs AS (
            -- Epoch windows are computed BEFORE the self-mapping filter so
            -- a chain that resolves back to its own name still contributes
            -- its retirement boundary to the next epoch's window.
            -- Chronological order over retirements: date ASC, and at
            -- same-second ties rowid DESC (newer commits get SMALLER
            -- rowids during the reverse-chronological ingest walk).
            SELECT old_path, canonical_path, retired_date, retired_rowid,
                   LAG(retired_date) OVER (
                       PARTITION BY old_path
                       ORDER BY retired_date ASC, retired_rowid DESC
                   ) AS prev_retired_date,
                   LAG(retired_rowid) OVER (
                       PARTITION BY old_path
                       ORDER BY retired_date ASC, retired_rowid DESC
                   ) AS prev_retired_rowid
            FROM resolved
            WHERE rn = 1
        )
        SELECT * FROM epochs WHERE old_path != canonical_path";
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
    // The join is time-bounded to the retirement epoch: a change row at
    // `old_path` is canonicalized only when its commit falls inside the
    // half-open window (previous retirement, this retirement). Without the
    // bound, a recycled filename's NEW file would inherit the OLD file's
    // lineage target — the exact conflation the recursive CTE's own date
    // guard prevents during chain construction. Same-second ties follow
    // the ingest convention (smaller rowid = newer commit): a row belongs
    // to the retired file when it is strictly OLDER than the retiring
    // rename (larger rowid), and to a later epoch when strictly NEWER
    // than the previous retirement (smaller rowid). The retiring commit
    // itself never carries a row at `old_path` — a rename writes exactly
    // one row, keyed on the new path.
    let sql = "CREATE OR REPLACE TEMPORARY TABLE changes_lineage AS
        SELECT
            c.rev,
            COALESCE(pl.canonical_path, c.path) AS path,
            c.change_type,
            c.rename_from,
            c.loc_added,
            c.loc_deleted
        FROM changes c
        INNER JOIN commits co ON co.rev = c.rev
        LEFT JOIN path_lineage pl
               ON pl.old_path = c.path
              AND (co.date < pl.retired_date
                   OR (co.date = pl.retired_date
                       AND co.rowid > pl.retired_rowid))
              AND (pl.prev_retired_date IS NULL
                   OR co.date > pl.prev_retired_date
                   OR (co.date = pl.prev_retired_date
                       AND co.rowid < pl.prev_retired_rowid))";
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
