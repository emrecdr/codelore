//! `architecture-trend` analysis — structural decay over the commit
//! sequence.
//!
//! The HEAD-only architecture metrics (`architecture-metrics`,
//! `dependency-cycles`) answer "how tangled is the code *now*". This
//! answers the more useful question — "is it getting *worse*, and when
//! did it start?" — by recomputing the structural-health metrics at a
//! series of historical revisions and emitting one row per sample point.
//!
//! ## How it samples history
//!
//! Up to [`SAMPLE_POINTS`] commits are picked evenly across the full
//! date-ordered history (the newest is always included). At each sampled
//! rev the import graph is rebuilt **from scratch in memory** — no
//! persistent table is touched:
//!
//! 1. files live at that rev come from the `changes`/`commits` history
//!    (latest change at-or-before the rev's date that isn't a deletion —
//!    the same date-anchored liveness `code-age` uses),
//! 2. each live source blob is read at that rev (`Repo::read_blob_at`),
//!    parsed for imports, and resolved in memory with the same
//!    per-language resolvers the HEAD scan uses,
//! 3. the resolved edges feed the shared SCC + reachability kernel.
//!
//! ## Cost
//!
//! This is the one analysis that re-parses source at many revisions, so
//! it is markedly heavier than the SQL-only analyses (roughly
//! `SAMPLE_POINTS ×` a HEAD import scan). It is computed on demand and
//! never cached.

use std::collections::HashSet;

use crate::analyses::import_graph::{build_import_graph_from_edges, graph_metrics};
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::{Options, Result};

/// Maximum number of historical sample points on the trend.
pub const SAMPLE_POINTS: usize = 12;

/// One sampled point on the architecture-decay trend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchitectureTrendRow {
    /// Sample date (`YYYY-MM-DD`).
    pub date: String,
    /// The sampled commit's short SHA (first 12 chars).
    pub rev: String,
    /// Files in the resolved import graph at this rev.
    pub files: u32,
    /// Propagation cost — density of the transitive-closure matrix
    /// (`sum(vfo)/n²`); "a change to a random file reaches this fraction
    /// of the system". The headline decay signal.
    pub propagation_cost: f64,
    /// Number of non-trivial dependency cycles (SCCs of size ≥ 2).
    pub cycle_count: u32,
    /// Size of the largest dependency cycle (0 if acyclic).
    pub largest_cycle: u32,
}

/// The ≤`SAMPLE_POINTS` evenly-spaced `(rev, timestamp)` commit samples,
/// oldest→newest (newest always included). Shared by `architecture-trend` and
/// `health-trend` so the rev set is identical between the two views.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors.
pub(crate) fn sampled_commits(db: &FactsDb) -> Result<Vec<(String, String)>> {
    let commits: Vec<(String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT rev, CAST(date AS TEXT) FROM commits ORDER BY date ASC, rowid ASC",
        [],
        "sampled-commits",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let picks = evenly_spaced_indices(commits.len(), SAMPLE_POINTS);
    Ok(picks.into_iter().map(|i| commits[i].clone()).collect())
}

/// Run the `architecture-trend` analysis. Returns one row per sampled
/// rev, oldest first. Needs repository access (it reads historical
/// blobs), unlike the SQL-only analyses.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// and [`crate::CodeLoreError::Repo`] on object-database I/O failures.
#[tracing::instrument(name = "architecture-trend", skip_all)]
pub fn run_architecture_trend<R: Repo>(
    db: &FactsDb,
    repo: &R,
    // Unused, but kept for signature parity with the other analyses.
    _opts: &Options,
) -> Result<Vec<ArchitectureTrendRow>> {
    let samples = sampled_commits(db)?;
    let mut rows = Vec::with_capacity(samples.len());
    for (rev, ts) in &samples {
        let graph = import_graph_at_rev(db, repo, rev, ts)?;
        // Shared kernel — so `architecture-trend` and the HEAD
        // `architecture-metrics` report the same numbers by construction.
        let m = graph_metrics(&graph);
        rows.push(ArchitectureTrendRow {
            // Calendar date = the `YYYY-MM-DD` prefix of the timestamp text.
            date: ts.get(..10).unwrap_or(ts).to_string(),
            rev: rev.chars().take(12).collect(),
            files: u32::try_from(m.n).unwrap_or(u32::MAX),
            propagation_cost: m.propagation_cost,
            cycle_count: m.cycle_count,
            largest_cycle: m.largest_cycle,
        });
    }
    Ok(rows)
}

/// Reconstruct the resolved import graph as it existed at `rev` (whose
/// commit timestamp is `ts`), entirely in memory — no `imports`-table
/// writes. The shared historical-scan primitive: `architecture-trend`
/// calls it once per sample point; `cycle-origins` calls it repeatedly
/// while bisecting history for a cycle's formation commit.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError`] on the `DuckDB` live-paths query.
/// Per-file blob/parse failures are swallowed (the file is skipped),
/// matching the HEAD scan's tolerance.
pub(crate) fn import_graph_at_rev<R: Repo>(
    db: &FactsDb,
    repo: &R,
    rev: &str,
    ts: &str,
) -> Result<crate::analyses::import_graph::ImportGraph> {
    let live = live_paths_at(db, ts)?;
    Ok(import_graph_from_live_paths(repo, rev, &live))
}

/// Build the resolved import graph at `rev` from an already-computed
/// live-path set. Split out of [`import_graph_at_rev`] so a caller that also
/// needs `live_paths_at`'s result for other work (e.g. a per-rev complexity
/// scan) can compute the live set once and feed it to both.
pub(crate) fn import_graph_from_live_paths<R: Repo>(
    repo: &R,
    rev: &str,
    live: &[String],
) -> crate::analyses::import_graph::ImportGraph {
    let edges = resolve_imports_at_rev(repo, rev, live);
    build_import_graph_from_edges(&edges)
}

/// Pick up to `k` evenly-spaced indices over `0..len`, always including
/// the last (newest commit). Returns `0..len` when `len <= k`.
pub(crate) fn evenly_spaced_indices(len: usize, k: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    if len <= k {
        return (0..len).collect();
    }
    let mut out = Vec::with_capacity(k);
    for i in 0..k {
        // Map i ∈ [0, k-1] → index ∈ [0, len-1], hitting both ends.
        out.push(i * (len - 1) / (k - 1));
    }
    out.dedup();
    out
}

/// Files live at the rev whose timestamp is `ts`: the latest change
/// at-or-before that instant that isn't a deletion. Date-anchored
/// liveness (mirrors `code-age`), so it approximates tree membership on
/// mostly-linear histories without needing a tree walk at the rev.
pub(crate) fn live_paths_at(db: &FactsDb, ts: &str) -> Result<Vec<String>> {
    crate::analyses::query::query_map_collect(
        db,
        "SELECT path FROM ( \
            SELECT c.path, \
                   arg_max(c.change_type, ROW(commits.date, -commits.rowid)) AS change_type \
            FROM changes c \
            INNER JOIN commits ON commits.rev = c.rev \
            WHERE commits.date <= CAST(? AS TIMESTAMP) \
            GROUP BY c.path \
         ) WHERE change_type != 'deleted'",
        duckdb::params![ts],
        "architecture-trend live-paths",
        |r| r.get::<_, String>(0),
    )
}

/// Extract + resolve every import edge among `live_paths` at `rev`,
/// entirely in memory. Mirrors the HEAD scan's extract pass
/// (`populate_imports_at_head`) and resolver dispatch
/// (`resolve_imports_at_head`), but reads blobs at an arbitrary rev and
/// never touches the `imports` table.
fn resolve_imports_at_rev<R: Repo>(
    repo: &R,
    rev: &str,
    live_paths: &[String],
) -> Vec<(String, String)> {
    use crate::imports::{ImportLanguage, extract_imports, resolve_by_extension};
    use rayon::prelude::*;

    let live_set: HashSet<String> = live_paths.iter().cloned().collect();
    let candidates: Vec<(String, ImportLanguage)> = live_paths
        .iter()
        .filter_map(|rel| {
            let lang = ImportLanguage::from_path(std::path::Path::new(rel))?;
            Some((rel.clone(), lang))
        })
        .collect();

    // Parallel blob-read + extract + per-language resolve. A read/parse
    // failure on one file skips that file (a corrupt blob mustn't sink
    // the whole sample point), matching the HEAD scan's tolerance.
    let edges: Vec<(String, String)> = candidates
        .into_par_iter()
        .flat_map_iter(|(rel, lang)| {
            let mut out: Vec<(String, String)> = Vec::new();
            let Ok(Some(code)) = repo.read_blob_at(rev, &rel) else {
                return out.into_iter();
            };
            if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                return out.into_iter();
            }
            let Ok(imports) = extract_imports(&code, lang) else {
                return out.into_iter();
            };
            for imp in imports {
                if let Some(target_path) = resolve_by_extension(&rel, &imp.target, &live_set) {
                    out.push((rel.clone(), target_path));
                }
            }
            out.into_iter()
        })
        .collect();
    edges
}
