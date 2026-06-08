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
fn build_soc_sql(src: &str) -> String {
    format!(
        "WITH rev_sizes AS (
             SELECT rev, COUNT(DISTINCT path) AS n FROM {src} GROUP BY rev
         )
         SELECT c.path AS entity, SUM(rs.n - 1)::INTEGER AS soc
         FROM {src} c JOIN rev_sizes rs USING (rev)
         GROUP BY c.path
         HAVING SUM(rs.n - 1) >= ?
         ORDER BY soc DESC, entity ASC
         LIMIT ?"
    )
}

pub fn run_soc(db: &FactsDb, opts: &Options) -> Result<Vec<SocRow>> {
    // PAR-8: if --time-bucket is active, materialize changes_bucketed first.
    if let Some(bucket) = opts.time_bucket {
        crate::facts::ingest::materialize_changes_bucketed(db, bucket)?;
    }
    let src = if opts.time_bucket.is_some() {
        "changes_bucketed"
    } else {
        "changes"
    };

    // Modern: --min-soc N gates the SoC value. Legacy compat: fall back
    // to --min-revs for users who scripted against code-maat's overloaded
    // semantic. Default (neither flag set): 1 (drop solo commits).
    let threshold: u32 = opts.min_soc.unwrap_or(if opts.code_maat_compat {
        opts.min_revs
    } else {
        1
    });
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);

    let sql = build_soc_sql(src);
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare soc: {e}")))?;
    let rows = stmt
        .query_map(params![threshold, row_limit], |r| {
            Ok(SocRow {
                entity: r.get::<_, String>(0)?,
                soc: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query soc: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect soc: {e}")))
}
