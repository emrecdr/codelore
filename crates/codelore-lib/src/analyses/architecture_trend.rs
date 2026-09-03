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

use crate::analyses::import_graph::{build_import_graph_seeded, graph_metrics};
use crate::complexity::Tier1Language;
use crate::facts::FactsDb;
use crate::facts::ingest::coverage::{
    REASON_BLOB_READ, REASON_PARSE_ERROR, ScanCoverage, ScanOutcome,
};
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
    /// Live Tier-1 source files in the import graph at this rev (isolated
    /// files included, not only resolved-import participants).
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
        "SELECT rev, CAST(date AS TEXT) FROM commits ORDER BY date ASC, rowid DESC",
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
    // Seed from the Tier-1 source files live at this rev so isolated files
    // are counted, keeping the shared kernel's `n` in step with the HEAD
    // `architecture-metrics` node universe (`complexity_metrics`) — the
    // newest trend sample must equal the HEAD tile by construction.
    // Documented acceptable divergence: the historical seed is the
    // extension-filtered live-path set, which can't cheaply reproduce
    // HEAD's oversized-blob exclusion at a past rev, so an over-cap file
    // that the HEAD scan would drop can appear here as a singleton.
    let seeds: Vec<String> = live
        .iter()
        .filter(|p| Tier1Language::from_path(p.as_str()).is_some())
        .cloned()
        .collect();
    let edges = resolve_imports_at_rev(repo, rev, live);
    build_import_graph_seeded(&seeds, &edges)
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
/// at-or-before that instant that isn't a deletion, excluding paths a
/// rename retired at-or-before `ts`. Date-anchored liveness (mirrors
/// `code-age`), so it approximates tree membership on mostly-linear
/// histories without needing a tree walk at the rev.
///
/// The rename exclusion is era-bounded, NOT lineage-folding: the returned
/// names feed `Repo::blob_reader_at(rev)`, so they must be the names that
/// exist in that era's tree — folding to today's canonical names would
/// make every pre-rename blob read miss. A rename writes no deletion row
/// for its source, so without the exclusion a renamed-away path read as
/// live forever and the historical import graph carried the same file
/// under two names. A recycled name stays live: its own newer rows
/// postdate the rename that retired the earlier file.
pub(crate) fn live_paths_at(db: &FactsDb, ts: &str) -> Result<Vec<String>> {
    crate::analyses::query::query_map_collect(
        db,
        "SELECT path FROM ( \
            SELECT c.path, \
                   arg_max(c.change_type, ROW(commits.date, -commits.rowid)) AS change_type, \
                   MAX(ROW(commits.date, -commits.rowid)) AS last_seen \
            FROM changes c \
            INNER JOIN commits ON commits.rev = c.rev \
            WHERE commits.date <= CAST(? AS TIMESTAMP) \
            GROUP BY c.path \
         ) l \
         WHERE l.change_type != 'deleted' \
           AND NOT EXISTS ( \
             SELECT 1 FROM changes r \
             INNER JOIN commits rc ON rc.rev = r.rev \
             WHERE r.rename_from = l.path \
               AND r.change_type = 'renamed' \
               AND rc.date <= CAST(? AS TIMESTAMP) \
               AND ROW(rc.date, -rc.rowid) > l.last_seen \
           )",
        duckdb::params![ts, ts],
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

    // Parallel blob-read + extract + per-language resolve. One `BlobReader`
    // per rayon worker (`map_init`) resolves `rev`'s root tree once and
    // reuses a warm object-decode cache for every file that worker reads —
    // `resolve_imports_at_rev` runs once per `architecture-trend` sample
    // point (and repeatedly during `cycle-origins`' bisection), so this is
    // the "worse offender" the per-call `read_blob_at` path used to re-pay
    // in full every time. A read/parse failure on one file skips that file
    // (a corrupt blob mustn't sink the whole sample point), matching the
    // HEAD scan's tolerance.
    let outcomes: Vec<ScanOutcome<Vec<(String, String)>>> = candidates
        .into_par_iter()
        .map_init(
            || repo.blob_reader_at(rev),
            |reader, (rel, lang)| -> ScanOutcome<Vec<(String, String)>> {
                // Every one of these branches used to be a bare `return
                // out` — three silent drops, not even a `debug!`. The
                // consequence is one-directional and lands on the trend
                // chart: `seeds` above is built from the LIVE-PATH list,
                // not from successful reads, so a rev whose blobs fail
                // keeps its node count while losing its edges. Propagation
                // cost falls, `arch_health` rises, and `HealthTrendRow.files`
                // — the one column a reader could use to notice — is that
                // same seed count and does not move. A rev that scanned
                // nothing renders as full coverage at perfect health.
                let code = match reader.read(&rel) {
                    Ok(Some(code)) => code,
                    // Not present at this rev: legitimately absent, and no
                    // row was owed. `live_paths_at` is derived from history,
                    // so this is expected rather than a loss.
                    Ok(None) => return ScanOutcome::NotCounted,
                    Err(e) => {
                        tracing::warn!("architecture-trend: blob read failed for {rel}: {e}");
                        return ScanOutcome::Lost(REASON_BLOB_READ);
                    }
                };
                if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                    return ScanOutcome::SkippedOversize;
                }
                let Ok(imports) = extract_imports(&code, lang) else {
                    tracing::warn!("architecture-trend: import parse failed for {rel}");
                    return ScanOutcome::Lost(REASON_PARSE_ERROR);
                };
                let mut out: Vec<(String, String)> = Vec::new();
                for imp in imports {
                    if let Some(target_path) = resolve_by_extension(&rel, &imp.target, &live_set) {
                        out.push((rel.clone(), target_path));
                    }
                }
                // A file with no resolvable imports is still fully covered —
                // read and parsed, it simply contributes no edges.
                ScanOutcome::Scored(out)
            },
        )
        .collect();

    let coverage = ScanCoverage::tally(&outcomes);
    coverage.warn_if_degraded("architecture-trend import", "import graph");
    coverage.warn_if_mostly_oversize("architecture-trend import", "import graph");

    outcomes
        .into_iter()
        .filter_map(|o| match o {
            ScanOutcome::Scored(edges) => Some(edges),
            _ => None,
        })
        .flatten()
        .collect()
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::live_paths_at;
    use crate::facts::FactsDb;

    fn seed_commit(db: &FactsDb, rev: &str, day: u32) {
        db.execute_batch(&format!(
            "INSERT INTO commits (rev, author_email, author_name, committer_email, \
             canonical_author, date, committer_date, message, is_merge, parent_count) \
             VALUES ('{rev}', 'a@x', 'A', 'a@x', 'a@x', \
             '2026-03-{day:02} 12:00:00', '2026-03-{day:02} 12:00:00', 'm', false, 1)"
        ))
        .expect("seed commit");
    }

    /// A renamed-away source must drop out of the live set once the rename
    /// happened, stay live BEFORE it (era-correct), and come back when the
    /// name is recycled by a new file — all under raw era names, because the
    /// caller reads these paths' blobs at the historical rev.
    #[test]
    fn renamed_away_paths_are_dead_after_the_rename_and_live_before_it() {
        let db = FactsDb::new_in_memory().expect("db");
        // Newest first: c3 recycles a.rs; c2 renames a.rs -> b.rs; c1 adds a.rs.
        seed_commit(&db, "c3", 5);
        seed_commit(&db, "c2", 3);
        seed_commit(&db, "c1", 1);
        for stmt in [
            "INSERT INTO changes VALUES ('c1', 'a.rs', 'added', NULL, 5, 0)",
            "INSERT INTO changes VALUES ('c2', 'b.rs', 'renamed', 'a.rs', 0, 0)",
            "INSERT INTO changes VALUES ('c3', 'a.rs', 'added', NULL, 7, 0)",
        ] {
            db.execute_batch(stmt).expect("seed");
        }

        let at = |ts: &str| {
            let mut v = live_paths_at(&db, ts).expect("live_paths_at");
            v.sort();
            v
        };
        assert_eq!(
            at("2026-03-02 00:00:00"),
            vec!["a.rs"],
            "before the rename the source is live under its era name"
        );
        assert_eq!(
            at("2026-03-04 00:00:00"),
            vec!["b.rs"],
            "after the rename only the new name is live — the retired source must not linger"
        );
        assert_eq!(
            at("2026-03-06 00:00:00"),
            vec!["a.rs", "b.rs"],
            "a recycled name is a NEW live file alongside the rename target"
        );
    }
}
