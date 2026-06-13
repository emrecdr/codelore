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
//!
//! Research basis: see `docs/research-foundations.md` entry "coupling"
//! (Gall, Hajek & Jazayeri, ICSM 1998 — original logical-coupling paper;
//! Tornhill, *Your Code as a Crime Scene*, 2015 — productisation).

use duckdb::params;

use crate::facts::FactsDb;
use crate::options::TimeBucket;
use crate::{CodeLoreError, Options, Result};

/// F29: build the `good_commits` CTE so the per-commit
/// `max_changeset_size` filter is applied to PHYSICAL commits even when
/// the analysis runs against a time-bucketed source.
///
/// - Non-bucketing: `good_commits.rev` = commit SHA.
///   `INNER JOIN {src} USING(rev)` matches naturally.
/// - Bucketing: `good_commits.rev` = bucket date key. A bucket survives
///   iff EVERY contained physical commit has ≤`max_changeset_size`
///   files (`HAVING MAX(files) <= ?`). Conservative semantic — a bucket
///   with even one giant commit is excluded — but no longer drops
///   active periods just because their TOTAL file count exceeds the
///   per-commit threshold (the pre-F29 bug).
///
/// The first placeholder in the returned CTE is always
/// `max_changeset_size`, matching the legacy CTE shape so callers'
/// param-binding order is unchanged.
pub(crate) fn good_commits_cte(bucket: Option<TimeBucket>, use_lineage: bool) -> String {
    let physical_src = if use_lineage {
        "changes_lineage"
    } else {
        "changes"
    };
    if let Some(b) = bucket {
        let unit = b.as_sql_unit();
        format!(
            "good_commits AS (
                 SELECT bucket_rev AS rev FROM (
                     SELECT CAST(date_trunc('{unit}', m.date) AS TEXT) AS bucket_rev,
                            c.rev AS commit_rev,
                            COUNT(*) AS files
                     FROM {physical_src} c
                     INNER JOIN commits m ON m.rev = c.rev
                     GROUP BY c.rev, date_trunc('{unit}', m.date)
                 ) per_commit_in_bucket
                 GROUP BY bucket_rev
                 HAVING MAX(files) <= ?
             )"
        )
    } else {
        format!(
            "good_commits AS (
                 SELECT rev
                 FROM (SELECT rev, COUNT(*) AS files FROM {physical_src} GROUP BY rev) t
                 WHERE files <= ?
             )"
        )
    }
}

/// A single coupling pair produced by [`run_coupling`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

/// Source-table selector for the coupling query family. Returns the SQL
/// identifier name (`"changes"` for raw commit grain or `"changes_bucketed"`
/// when `--time-bucket` is active). Used to swap the source table in the
/// coupling and total-commits SQL queries below.
///
/// The injection is safe: the returned value is a literal compile-time
/// string from a closed match, never user-controlled input.
fn source_table(opts: &Options) -> &'static str {
    // `--time-bucket` wins when both knobs are set — bucketing and
    // lineage compose, but materializing both requires a 4-way matrix
    // (changes, changes_lineage, changes_bucketed, changes_bucketed_lineage)
    // that we ship in a later point release. Lineage is the more common
    // request, so it gets first-class support for the non-bucketed path.
    if opts.time_bucket.is_some() {
        "changes_bucketed"
    } else if opts.use_canonical_lineage {
        "changes_lineage"
    } else {
        "changes"
    }
}

/// Raw coupling candidates SQL builder. Bind values (in order):
///  1. `max_changeset_size` — `good_commits` filter
///  2. `min_revs` — revs floor (semantic depends on `code_maat_compat`,
///     see PAR-6 doc below)
///  3. `min_shared_revs` — per-pair shared floor
///  4. `min_coupling_pct` — lower degree threshold
///  5. `max_coupling_pct` — upper degree threshold (pairs above are usually file splits or copy/rename pairs)
///
/// NOTE: there is no `LIMIT` placeholder here on purpose. `rows_limit`
/// applies AFTER the Rust-side Fisher exact significance filter — applying
/// it inside SQL would let pairs that fail the significance test consume
/// "slots" in the top-N, silently truncating the user's `--rows N` result
/// to fewer than N rows even when more significant pairs exist further
/// down the degree ranking. See `run_coupling` for the post-filter slice.
///
/// The `src` parameter is one of `"changes"` or `"changes_bucketed"` —
/// closed-enum-derived, never user input.
///
/// PAR-6: `code_maat_compat` flips the `min_revs` pivot point:
///
/// - **Default (`CodeLore`)**: per-file filter in `file_revs` CTE
///   (`HAVING revs >= ?`). A pair where one file has 4 revs and the
///   other has 20 is dropped under `--min-revs 5` because the 4-rev
///   file is filtered out before pairing. Stricter; matches the spec
///   §3.2.1 invariants documented in this file's header.
/// - **`--code-maat-compat`**: per-pair-average filter on the final
///   SELECT (`WHERE average_revs >= ?`). The same pair survives because
///   its average is 12. Matches code-maat's `coupling-algos.clj`
///   `within-threshold?` semantic where `revs` is the pair's average,
///   not either file's individual revs.
///
/// Both branches emit exactly 6 `?` placeholders, in fixed positional
/// order:
///
/// 1. `max_changeset_size` — `good_commits` filter
/// 2. `min_revs` — per-file revs floor (default) or dummy comparison
///    consumed by `? IS NOT NULL` (compat — placeholder still consumed
///    so caller's param binding is shape-stable)
/// 3. `min_shared_revs` — per-pair shared floor
/// 4. `min_coupling_pct` — lower degree threshold
/// 5. `max_coupling_pct` — upper degree threshold
/// 6. `min_revs` — per-pair-average filter (compat) or dummy comparison
///    consumed by `? IS NOT NULL` (default)
///
/// `min_revs` is bound twice; only one branch's gate is "live", the
/// other is a tautology. Trade-off: a 1-bind redundancy for a single
/// caller-side `params!` invocation that doesn't branch on the flag.
fn build_coupling_sql(
    src: &str,
    code_maat_compat: bool,
    bucket: Option<TimeBucket>,
    use_lineage: bool,
) -> String {
    // DEEP-3: under compat, average_revs uses CEIL((a+b)/2.0) to match
    // code-maat's `(math/ceil average-revs)`. CodeLore's modern default
    // uses integer-floor `(a+b)/2` (DuckDB integer division). The
    // ceiling differs from the floor by 1 for any odd-sum pair.
    let avg_revs_expr = if code_maat_compat {
        "CAST(CEIL((fr_a.revs + fr_b.revs) / 2.0) AS UINTEGER)"
    } else {
        "(fr_a.revs + fr_b.revs) / 2"
    };
    let (file_revs_gate, pair_avg_gate) = if code_maat_compat {
        // Compat: live gate is per-pair-average on the final SELECT.
        // `file_revs` consumes its placeholder via a tautology that
        // succeeds whenever the input is NOT NULL (always true for
        // a u32-bound `?`).
        (
            "HAVING ? IS NOT NULL",
            "AND (fr_a.revs + fr_b.revs) / 2.0 >= ?",
        )
    } else {
        // Default: live gate is per-file `HAVING revs >= ?`. The
        // per-pair-average placeholder is consumed by a tautology in
        // the final SELECT.
        ("HAVING revs >= ?", "AND ? IS NOT NULL")
    };
    let good_cte = good_commits_cte(bucket, use_lineage);
    format!(
        "WITH {good_cte},
         file_revs AS (
             -- (rev, path) is the changes PK; COUNT(rev) == COUNT(DISTINCT rev)
             -- per path. Plain COUNT skips DuckDB's distinct-tracking overhead.
             SELECT path, COUNT(rev) AS revs
             FROM {src}
             INNER JOIN good_commits USING(rev)
             GROUP BY path
             {file_revs_gate}
         ),
         pairs AS (
             -- The triple (a.rev, a.path, b.path) is unique per (path_a,
             -- path_b) group because (rev, path) is the changes PK and
             -- `a.rev = b.rev` collapses the cardinality to one rev per
             -- joined row. COUNT(a.rev) == COUNT(*) == COUNT(DISTINCT a.rev)
             -- here; plain COUNT skips DuckDB's distinct-tracking overhead.
             SELECT
                 a.path AS path_a,
                 b.path AS path_b,
                 COUNT(a.rev) AS shared
             FROM {src} a
             INNER JOIN {src} b ON a.rev = b.rev AND a.path < b.path
             INNER JOIN good_commits ON good_commits.rev = a.rev
             GROUP BY a.path, b.path
             HAVING shared >= ?
         )
         SELECT
             p.path_a,
             p.path_b,
             p.shared,
             fr_a.revs AS revs_a,
             fr_b.revs AS revs_b,
             {avg_revs_expr} AS average_revs,
             100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2.0, 0) AS degree
         FROM pairs p
         INNER JOIN file_revs fr_a ON fr_a.path = p.path_a
         INNER JOIN file_revs fr_b ON fr_b.path = p.path_b
         WHERE 100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2.0, 0) >= ?
           AND 100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2.0, 0) <= ?
           {pair_avg_gate}
         ORDER BY degree DESC, average_revs DESC, p.path_a ASC, p.path_b ASC"
    )
}

fn build_total_commits_sql(bucket: Option<TimeBucket>, use_lineage: bool) -> String {
    // F29: the "total commits" denominator for Fisher's contingency
    // table must count the same units (physical commits, or buckets)
    // that `good_commits` filters. Reuse the bucketing-aware CTE so
    // the numerator and denominator stay in lockstep — otherwise the
    // p-value math is over a wrong sample-space size under
    // `--time-bucket`.
    let good_cte = good_commits_cte(bucket, use_lineage);
    format!(
        "WITH {good_cte}
         SELECT COUNT(*) FROM good_commits"
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
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn run_coupling(db: &FactsDb, opts: &Options) -> Result<Vec<CouplingRow>> {
    // Unified dispatch: --time-bucket > canonical lineage > raw. When both
    // bucketing and lineage are on, lineage is materialised first and
    // bucketing happens on top so rename ancestry survives.
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = source_table(opts);

    // Total commits after the max_changeset_size pre-filter — denominator for
    // the Fisher 2×2 contingency table.
    let total_sql = build_total_commits_sql(opts.time_bucket, opts.use_canonical_lineage);
    let total_commits: i64 = db
        .conn()
        .query_row(&total_sql, params![opts.max_changeset_size], |r| r.get(0))
        .map_err(|e| CodeLoreError::Analysis(format!("total commits query: {e}")))?;
    let total = u32::try_from(total_commits).unwrap_or(u32::MAX);

    // PAR-6: `build_coupling_sql` now takes `code_maat_compat` and the
    // bind list has 6 entries (min_revs bound twice — only one branch's
    // gate is live, the other is a tautology). See builder's doc.
    let coupling_sql = build_coupling_sql(
        src,
        opts.code_maat_compat,
        opts.time_bucket,
        opts.use_canonical_lineage,
    );
    crate::analyses::query::explain_if_requested(
        db,
        &coupling_sql,
        params![
            opts.max_changeset_size,
            opts.min_revs,
            opts.min_shared_revs,
            opts.min_coupling_pct,
            opts.max_coupling_pct,
            opts.min_revs,
        ],
        "coupling",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&coupling_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare coupling: {e}")))?;

    let raw_rows = stmt
        .query_map(
            params![
                opts.max_changeset_size,
                opts.min_revs,
                opts.min_shared_revs,
                opts.min_coupling_pct,
                opts.max_coupling_pct,
                opts.min_revs,
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, f64>(6)?,
                ))
            },
        )
        .map_err(|e| CodeLoreError::Analysis(format!("query coupling: {e}")))?;

    // Collect ALL candidates first, filter by Fisher exact significance, THEN
    // truncate to `rows_limit`. The previous in-SQL `LIMIT ?` ran BEFORE the
    // Fisher filter, so significance failures stole slots from the top-N and
    // users requesting `--rows 100` could see 0–99 rows even when more
    // significant pairs existed further down the degree ranking.
    let mut out = Vec::new();
    for raw in raw_rows {
        let (path_a, path_b, shared_raw, count_a, count_b, avg_raw, degree) =
            raw.map_err(|e| CodeLoreError::Analysis(format!("collect coupling row: {e}")))?;

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

    if let Some(n) = opts.rows_limit {
        out.truncate(n as usize);
    }

    Ok(out)
}

/// Count the number of nodes in the behavioral coupling graph universe.
/// Returns the count of distinct files in the same `file_revs` candidate
/// set used by [`run_coupling`] — i.e. files with `revs >= opts.min_revs`
/// after the configured `--time-bucket` / `--use-canonical-lineage`
/// rewrite. Honours `opts.max_changeset_size` via the same `good_commits`
/// pre-filter so isolated nodes inside large commits don't inflate the
/// denominator of [`density`].
///
/// # Errors
///
/// Propagates `DuckDB` errors from the underlying query.
pub fn count_coupling_nodes(db: &FactsDb, opts: &Options) -> Result<u64> {
    let src = source_table(opts);
    let use_lineage =
        opts.use_canonical_lineage && opts.time_bucket.is_none() && src == "changes_lineage";
    if use_lineage {
        crate::analyses::lineage::materialize_if_needed(db, opts)?;
    }
    // Mirrors the `good_commits` + `file_revs` CTE pair in
    // build_coupling_sql, narrowed to a single scalar.
    let sql = format!(
        "WITH good_commits AS (
             SELECT rev FROM (
                 SELECT rev, COUNT(path) AS n
                 FROM {src}
                 GROUP BY rev
             ) WHERE n <= ?
         ),
         file_revs AS (
             SELECT path, COUNT(rev) AS revs
             FROM {src}
             INNER JOIN good_commits USING(rev)
             GROUP BY path
             HAVING revs >= ?
         )
         SELECT COUNT(*) FROM file_revs"
    );
    let count: i64 = db
        .conn()
        .query_row(
            &sql,
            params![opts.max_changeset_size, opts.min_revs],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| CodeLoreError::Analysis(format!("count coupling nodes: {e}")))?;
    Ok(u64::try_from(count).unwrap_or(0))
}

/// Density of the behavioral coupling graph: `2·E / (V·(V−1))`, in `[0, 1]`.
///
/// `V` is the node count from [`count_coupling_nodes`]; `E` is the count
/// of Fisher-significant coupling pairs (typically the length of the
/// [`run_coupling`] result vector). The graph is undirected and each pair
/// is counted once.
///
/// Returns `0.0` when `V < 2` (a graph with fewer than two nodes has no
/// possible edges). Returns `1.0` when `E ≥ V·(V−1)/2` (every possible
/// pair is coupled — fully connected).
///
/// Range guidance (empirical, repo-dependent):
/// - `< 0.01` — sparsely coupled, files change largely independently
/// - `0.01 – 0.10` — typical for modular codebases
/// - `> 0.10` — tightly coupled; candidate for refactoring or a sign
///   of a small/cohesive codebase
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn density(node_count: u64, edge_count: usize) -> f64 {
    if node_count < 2 {
        return 0.0;
    }
    let max_edges = node_count.saturating_mul(node_count - 1) / 2;
    if max_edges == 0 {
        return 0.0;
    }
    let e = edge_count as f64;
    let max = max_edges as f64;
    (e / max).clamp(0.0, 1.0)
}

#[cfg(test)]
mod density_tests {
    use super::density;

    #[test]
    fn density_zero_when_fewer_than_two_nodes() {
        assert!(density(0, 0).abs() < f64::EPSILON);
        assert!(density(1, 0).abs() < f64::EPSILON);
        // Pathological case: an edge count > 0 with < 2 nodes — return 0.0
        // (graph is structurally impossible; defensive guard).
        assert!(density(1, 5).abs() < f64::EPSILON);
    }

    #[test]
    fn density_zero_on_empty_edge_set() {
        assert!(density(100, 0).abs() < f64::EPSILON);
    }

    #[test]
    fn density_one_when_complete_graph() {
        // K_4 has 6 edges (4·3/2).
        assert!((density(4, 6) - 1.0).abs() < f64::EPSILON);
        // Over-count is clamped to 1.0.
        assert!((density(4, 99) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn density_matches_empirical_codelore_repo() {
        // Empirically measured on the CodeLore repo at v0.4.4: 59
        // candidate nodes (files with revs >= min_revs after the
        // good_commits pre-filter) and 47 Fisher-significant edges. The
        // `~0.0275` figure documents the calibration in
        // research-foundations.md. NOTE: the denominator uses the
        // FULL candidate set including isolated nodes (no coupling
        // partners), matching graphology / Newman convention.
        let d = density(59, 47);
        assert!(
            (d - 0.0275).abs() < 0.001,
            "expected ~0.0275, got {d} — re-measure if the fixture has drifted"
        );
    }
}
