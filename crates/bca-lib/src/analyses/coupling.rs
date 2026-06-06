//! Change-coupling analysis per spec §3.2.1 correctness invariants:
//!
//! 1. max-changeset-size pre-filter (drops huge commits)
//! 2. Mirrored pair dedup via `path_a < path_b`
//! 3. Empty-changeset filter (implicit — commits with 0 files produce no pairs)
//! 4. `min_revs` filter (per-file)
//! 5. `min_shared_revs` filter (per-pair)
//! 6. `min_coupling_pct` filter (degree threshold)
//! 7. Fisher exact significance test (p < `fisher_significance`, default 0.05)
//!
//! The Fisher test guards against spurious coupling from refactor sweeps
//! that 2025 MSR research identified as the dominant noise source.

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};

/// A single coupling pair produced by [`run_coupling`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct CouplingRow {
    /// Lexicographically smaller path of the pair (canonical ordering).
    pub entity_a: String,
    /// Lexicographically larger path of the pair.
    pub entity_b: String,
    /// Number of commits in which both files were changed together.
    pub shared: u32,
    /// Total commits that touched `entity_a`.
    pub revs_a: u32,
    /// Total commits that touched `entity_b`.
    pub revs_b: u32,
    /// Average of `revs_a` and `revs_b` (integer arithmetic).
    pub average_revs: u32,
    /// Coupling degree: `100.0 * shared / average_revs`.
    pub degree: f64,
    /// Two-tailed Fisher exact p-value for the 2×2 contingency table.
    pub fisher_p: f64,
}

/// Build the SQL query that produces raw coupling candidates.
fn coupling_sql(opts: &Options) -> String {
    let limit = opts
        .rows_limit
        .map(|n| format!(" LIMIT {n}"))
        .unwrap_or_default();
    format!(
        "WITH good_commits AS (
             SELECT rev
             FROM (SELECT rev, COUNT(*) AS files FROM changes GROUP BY rev) t
             WHERE files <= {max}
         ),
         file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs
             FROM changes
             INNER JOIN good_commits USING(rev)
             GROUP BY path
             HAVING revs >= {min_revs}
         ),
         pairs AS (
             SELECT
                 a.path AS path_a,
                 b.path AS path_b,
                 COUNT(DISTINCT a.rev) AS shared
             FROM changes a
             INNER JOIN changes b ON a.rev = b.rev AND a.path < b.path
             INNER JOIN good_commits ON good_commits.rev = a.rev
             GROUP BY a.path, b.path
             HAVING shared >= {min_shared}
         )
         SELECT
             p.path_a,
             p.path_b,
             p.shared,
             fr_a.revs AS revs_a,
             fr_b.revs AS revs_b,
             (fr_a.revs + fr_b.revs) / 2 AS average_revs,
             100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2.0, 0) AS degree
         FROM pairs p
         INNER JOIN file_revs fr_a ON fr_a.path = p.path_a
         INNER JOIN file_revs fr_b ON fr_b.path = p.path_b
         WHERE 100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2.0, 0) >= {min_pct}
         ORDER BY degree DESC, average_revs DESC, p.path_a ASC, p.path_b ASC{limit}",
        max = opts.max_changeset_size,
        min_revs = opts.min_revs,
        min_shared = opts.min_shared_revs,
        min_pct = opts.min_coupling_pct,
        limit = limit,
    )
}

/// Compute the two-tailed Fisher exact p-value for a coupling pair.
///
/// Returns `None` for degenerate tables (values > `i32::MAX`).
///
/// # 2×2 contingency table layout
///
/// ```text
///                    | b touched | b NOT touched
///  a touched         |  shared   | revs_a - shared
///  a NOT touched     | revs_b -  | total - revs_a - revs_b + shared
///                    |  shared   |
/// ```
fn fisher_two_tail(shared: u32, revs_a: u32, revs_b: u32, total: u32) -> Option<f64> {
    let a = shared;
    let b = revs_a.saturating_sub(shared);
    let c = revs_b.saturating_sub(shared);
    let d = total.saturating_sub(a).saturating_sub(b).saturating_sub(c);
    fishers_exact::fishers_exact(&[a, b, c, d])
        .ok()
        .map(|pv| pv.two_tail_pvalue)
}

/// Run change-coupling analysis over the ingested fact store.
///
/// Returns rows sorted by `degree DESC, average_revs DESC, entity_a ASC, entity_b ASC`.
///
/// # Errors
///
/// Returns [`BcaError::Analysis`] on any SQL error.
pub fn run_coupling(db: &FactsDb, opts: &Options) -> Result<Vec<CouplingRow>> {
    // Total commits after the max_changeset_size pre-filter — denominator for
    // the Fisher 2×2 contingency table.
    let total_sql = format!(
        "SELECT COUNT(*) FROM (
             SELECT rev, COUNT(*) AS files FROM changes GROUP BY rev
         ) t WHERE files <= {max}",
        max = opts.max_changeset_size,
    );
    let total_commits: i64 = db
        .conn()
        .query_row(&total_sql, [], |r| r.get(0))
        .map_err(|e| BcaError::Analysis(format!("total commits query: {e}")))?;
    let total = u32::try_from(total_commits).unwrap_or(u32::MAX);

    let sql = coupling_sql(opts);
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| BcaError::Analysis(format!("prepare coupling: {e}")))?;

    let raw_rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, f64>(6)?,
            ))
        })
        .map_err(|e| BcaError::Analysis(format!("query coupling: {e}")))?;

    let mut out = Vec::new();
    for raw in raw_rows {
        let (path_a, path_b, shared_raw, count_a, count_b, avg_raw, degree) =
            raw.map_err(|e| BcaError::Analysis(format!("collect coupling row: {e}")))?;

        let shared = u32::try_from(shared_raw).unwrap_or(u32::MAX);
        let revs_a = u32::try_from(count_a).unwrap_or(u32::MAX);
        let revs_b = u32::try_from(count_b).unwrap_or(u32::MAX);
        let average_revs = u32::try_from(avg_raw).unwrap_or(u32::MAX);

        // Fisher exact significance filter (step 7).
        let Some(fisher_p) = fisher_two_tail(shared, revs_a, revs_b, total) else {
            continue; // degenerate table — skip pair
        };

        if fisher_p < opts.fisher_significance {
            out.push(CouplingRow {
                entity_a: path_a,
                entity_b: path_b,
                shared,
                revs_a,
                revs_b,
                average_revs,
                degree,
                fisher_p,
            });
        }
    }

    Ok(out)
}
