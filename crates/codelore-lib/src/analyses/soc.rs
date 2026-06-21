//! Sum of Coupling (`soc`) — code-maat parity.
//!
//! Each commit of size N contributes `(N-1)` to every entity in it.
//! A solo commit contributes 0. The total per-entity `SoC` is the sum of
//! that contribution across every commit the entity appears in.
//!
//! Semantically: "how many distinct files has this file been changed
//! alongside, totaled across commits?". High `SoC` = central node in the
//! change-coupling graph.
//!
//! ## Threshold semantics (divergence from code-maat)
//!
//! Code-maat overloaded `--min-revs` to mean "minimum `SoC` sum" in this
//! one analysis (while it meant "minimum revision count" everywhere else).
//! `CodeLore` exposes a dedicated `--min-soc` flag with the honest name.
//! Under `--code-maat-compat`, `--min-revs` falls back to the legacy
//! "minimum `SoC` sum" semantic for migration users.
//!
//! Research basis: see `docs/research-foundations.md` entry "soc"
//! (Tornhill, *Software Design X-Rays*, 2018 — per-file centrality
//! across the change-coupling graph).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SocRow {
    pub entity: String,
    pub soc: u32,
}

/// `src` is one of `"changes"` or `"changes_bucketed"` (closed-enum
/// choice; not user input). Selecting `changes_bucketed` collapses
/// commits in the same time-bucket so a single rev key represents
/// multiple physical commits — affects `SoC` because `rev_sizes` then
/// counts unique paths-per-bucket instead of paths-per-commit.
///
/// F5 fix: pre-filter changesets by `max_changeset_size` so a single
/// massive sweep (lockfile bump, monorepo-wide rename, vendored
/// dependency import) doesn't dominate every participating file's `SoC`.
/// Without this, a 1000-file commit added 999 to every one of those
/// files' scores, producing false-positive "central nodes" that are
/// really just bystanders of a one-off sweep. Mirrors the `good_commits`
/// CTE pattern in `coupling.rs`.
///
/// F29 fix: under `--time-bucket` the `good_commits` filter now counts
/// files per PHYSICAL commit (via `good_commits_cte`), then derives
/// surviving bucket keys via `HAVING MAX(files) <= ?`. Pre-F29 the
/// filter counted paths per bucket — drowning any active week/month
/// in active repos and silently returning empty results.
fn build_soc_sql(
    src: &str,
    code_maat_compat: bool,
    bucket: Option<crate::options::TimeBucket>,
    use_lineage: bool,
) -> String {
    // DEEP-4: code-maat's `as-soc` filter is `(> n min-revs)` — strict
    // greater-than. Under `--code-maat-compat` we honour that semantic
    // so threshold-boundary results match exactly. CodeLore's modern
    // default uses `>=`, which is more intuitive ("SoC of at least N").
    let threshold_op = if code_maat_compat { ">" } else { ">=" };
    let good_cte = crate::analyses::coupling::good_commits_cte(bucket, use_lineage);
    format!(
        "WITH {good_cte},
         filtered_changes AS (
             -- Pre-filter `changes` against `good_commits` ONCE so both
             -- downstream CTEs share the result. DuckDB materializes a CTE
             -- referenced 2+ times (rev_sizes + the outer SELECT), so the
             -- self-implicit double scan of the raw `{src}` table is
             -- collapsed into one filter pass + two cached reads. Same
             -- pattern as `coupling.rs` (see the comment there for the
             -- O(N²)-to-O(K²) complexity rationale on large repos).
             SELECT rev, path
             FROM {src}
             INNER JOIN good_commits USING(rev)
         ),
         rev_sizes AS (
             -- (rev, path) is the changes PK so per `GROUP BY rev` each
             -- path appears at most once. Plain COUNT skips DuckDB's
             -- distinct-tracking overhead.
             SELECT rev, COUNT(path) AS n
             FROM filtered_changes
             GROUP BY rev
         )
         SELECT c.path AS entity, SUM(rs.n - 1)::INTEGER AS soc
         FROM filtered_changes c
         INNER JOIN rev_sizes rs USING (rev)
         GROUP BY c.path
         HAVING SUM(rs.n - 1) {threshold_op} ?
         ORDER BY soc DESC, entity ASC
         LIMIT ?"
    )
}

#[tracing::instrument(name = "soc", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_soc(db: &FactsDb, opts: &Options) -> Result<Vec<SocRow>> {
    // Unified dispatch: --time-bucket > canonical lineage > raw.
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);

    // Modern: --min-soc N gates the SoC value. Legacy compat: fall back
    // to --min-revs for users who scripted against code-maat's overloaded
    // semantic. Default (neither flag set): 1 (drop solo commits).
    let threshold: u32 = opts.min_soc.unwrap_or(if opts.code_maat_compat {
        opts.min_revs
    } else {
        1
    });
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    let sql = build_soc_sql(
        src,
        opts.code_maat_compat,
        opts.time_bucket,
        opts.use_canonical_lineage,
    );
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.max_changeset_size, threshold, row_limit],
        "soc",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare soc: {e}")))?;
    let rows = stmt
        .query_map(
            params![opts.max_changeset_size, threshold, row_limit],
            |r| {
                Ok(SocRow {
                    entity: r.get::<_, String>(0)?,
                    soc: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
                })
            },
        )
        .map_err(|e| CodeLoreError::Analysis(format!("query soc: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect soc: {e}")))
}
