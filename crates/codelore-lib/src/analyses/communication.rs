//! Communication analysis per spec §1.1 — Conway's law shared-work author pairs.
//!
//! For each pair of authors who co-edit the same files, compute:
//! - shared: distinct paths they both edited
//! - average: mean of their individual total commits
//! - strength: 100 × shared / average (percentage)
//!
//! Self-pairs are excluded.
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "communication" (Conway, *Datamation* 1968 — original Conway's-law
//! essay; Bird et al., *Comm. ACM* 2009 — empirical follow-up on
//! Windows Vista development showing organisational structure shapes
//! defect density).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunicationRow {
    pub author_a: String,
    pub author_b: String,
    pub shared: u32,   // distinct paths both authors touched
    pub average: u32,  // mean of authors' total commits
    pub strength: f64, // 100 * shared / average
}

// DEEP-11 + DEEP-12: communication has two numeric divergences vs
// code-maat's `communication.clj with-commit-stats`:
//
//   - `average`: code-maat uses `(math/ceil (m/average my peer))` — ceiling
//     rounding. CodeLore's modern default uses integer-div floor `(a+b)/2`
//     which is more conservative ("you've communicated at least this often").
//   - `strength`: code-maat uses `(int (m/as-percentage ...))` — truncated
//     integer. CodeLore's modern default emits a float `XX.XX` for precision.
//
// Under `--code-maat-compat`, we honour both code-maat semantics so
// downstream tools parsing the legacy CSV see identical values.
fn build_communication_sql(code_maat_compat: bool) -> String {
    let avg_expr = if code_maat_compat {
        // CEIL of mean — matches code-maat's `(math/ceil (m/average …))`.
        "CAST(CEIL((ta.commits + tb.commits) / 2.0) AS UINTEGER)"
    } else {
        // Integer floor via DuckDB integer division.
        "(ta.commits + tb.commits) / 2"
    };
    let strength_expr = if code_maat_compat {
        // Truncated to integer, then re-cast to DOUBLE so the row type
        // stays uniform; the Rust orchestrator formats as `XX`.
        "CAST(CAST(100.0 * p.shared / NULLIF((ta.commits + tb.commits) / 2.0, 0) AS INTEGER) \
         AS DOUBLE)"
    } else {
        // Float with two-decimal CSV formatting.
        "100.0 * p.shared / NULLIF((ta.commits + tb.commits) / 2.0, 0)"
    };
    format!(
        "
    WITH author_files AS (
        SELECT DISTINCT
            changes.path,
            commits.canonical_author AS author
        FROM commits
        INNER JOIN changes ON changes.rev = commits.rev
    ),
    pairs AS (
        SELECT
            a.author AS author_a,
            b.author AS author_b,
            COUNT(DISTINCT a.path) AS shared
        FROM author_files a
        INNER JOIN author_files b ON a.path = b.path AND a.author < b.author
        GROUP BY a.author, b.author
        HAVING shared >= ?
    ),
    totals AS (
        SELECT
            canonical_author AS author,
            -- `commits.rev` is the PRIMARY KEY of the commits table, so
            -- COUNT(rev) == COUNT(DISTINCT rev) per author. Plain COUNT
            -- skips DuckDB's distinct-tracking overhead.
            COUNT(rev) AS commits
        FROM commits
        GROUP BY canonical_author
    )
    SELECT
        p.author_a,
        p.author_b,
        p.shared,
        {avg_expr} AS average,
        {strength_expr} AS strength
    FROM pairs p
    INNER JOIN totals ta ON ta.author = p.author_a
    INNER JOIN totals tb ON tb.author = p.author_b
    ORDER BY strength DESC, p.author_a ASC, p.author_b ASC
    LIMIT ?
"
    )
}

pub fn run_communication(db: &FactsDb, opts: &Options) -> Result<Vec<CommunicationRow>> {
    // Route through `changes_lineage` when canonical lineage is enabled so
    // two authors who co-edited the SAME logical file across a rename still
    // count as having shared work. Without this rewrite, Conway's-law output
    // underreported team coupling on any history with renames. Mirrors the
    // pattern already in `entity_effort`, `messages`, `ownership`, etc.
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let base_sql = build_communication_sql(opts.code_maat_compat);
    let sql = crate::analyses::lineage::rewrite(&base_sql, opts);
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_shared_revs, row_limit],
        "communication",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare communication: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_shared_revs, row_limit], |r| {
            Ok(CommunicationRow {
                author_a: r.get::<_, String>(0)?,
                author_b: r.get::<_, String>(1)?,
                shared: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                average: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
                strength: r.get::<_, f64>(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query communication: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect communication: {e}")))
}
