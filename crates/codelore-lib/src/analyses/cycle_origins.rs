//! `cycle-origins` analysis — pinpoint the commit where each HEAD
//! dependency cycle first formed.
//!
//! `dependency-cycles` lists the import-graph cycles that exist *now*.
//! This goes further: for each cycle it bisects history — reading source
//! at past revisions via `Repo::read_blob_at` — to find the earliest
//! commit where that exact cycle existed, i.e. the commit that closed the
//! loop. Actionable archaeology: "the `a ↔ b ↔ c` tangle formed at
//! `abc1234` on 2026-03-15."
//!
//! ## Method
//!
//! The HEAD cycles come from the shared SCC kernel applied to the import
//! graph reconstructed at HEAD (via
//! [`architecture_trend::import_graph_at_rev`], so the cycle definition
//! and the bisect predicate use the exact same machinery). For each
//! cycle, a binary search over the date-ordered commit list finds the
//! transition from "cycle absent" to "cycle present", rebuilding the
//! import graph in memory at each probe and testing whether the cycle's
//! files sit in a single SCC there.
//!
//! Binary search assumes a cycle, once formed, stays formed — its
//! presence is monotonic over time. Cycles that form, break and reform
//! are rare; for those the search still returns one valid formation
//! point. Only the [`MAX_CYCLES`] largest cycles are traced (each costs
//! ~log₂(commits) graph rebuilds), bounding cost on tangled repos.

use crate::analyses::architecture_trend::import_graph_at_rev;
use crate::analyses::import_graph::{ImportGraph, tarjan_scc};
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::{Options, Result};

/// Maximum number of cycles to trace (largest first), bounding the
/// historical-scan cost on heavily-tangled repositories.
pub const MAX_CYCLES: usize = 10;

/// One traced dependency cycle and the commit where it first formed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CycleOriginRow {
    /// Number of files in the cycle.
    pub size: u32,
    /// Short SHA (first 12 chars) of the commit that first closed the loop.
    pub formed_at_rev: String,
    /// Calendar date (`YYYY-MM-DD`) of that commit.
    pub formed_at_date: String,
    /// The cycle's member files, `; `-joined (sorted).
    pub members: String,
}

/// Run the `cycle-origins` analysis. Returns one row per traced HEAD
/// cycle, largest first. Needs repository access (it reads historical
/// blobs).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// and [`crate::CodeLoreError::Repo`] on object-database I/O failures.
#[tracing::instrument(name = "cycle-origins", skip_all)]
pub fn run_cycle_origins<R: Repo>(
    db: &FactsDb,
    repo: &R,
    _opts: &Options,
) -> Result<Vec<CycleOriginRow>> {
    // Date-ordered history: (rev, timestamp, calendar-date).
    let commits: Vec<(String, String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT rev, CAST(date AS TEXT), CAST(CAST(date AS DATE) AS TEXT) \
         FROM commits ORDER BY date ASC, rowid ASC",
        [],
        "cycle-origins commits",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        },
    )?;
    // HEAD cycles, derived from the SAME in-memory reconstruction the
    // bisect uses (so a cycle found here is guaranteed present at the
    // last commit — the binary search's upper bound holds by construction).
    let Some((head_rev, head_ts, _)) = commits.last() else {
        return Ok(Vec::new());
    };
    let head_graph = import_graph_at_rev(db, repo, head_rev, head_ts)?;
    let mut cycles = cycle_member_sets(&head_graph);
    // Largest first; cap to bound historical-scan cost.
    cycles.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    cycles.truncate(MAX_CYCLES);

    let mut rows = Vec::with_capacity(cycles.len());
    for members in cycles {
        let idx = bisect_formation(db, repo, &commits, &members)?;
        let (rev, _, date) = &commits[idx];
        rows.push(CycleOriginRow {
            size: u32::try_from(members.len()).unwrap_or(u32::MAX),
            formed_at_rev: rev.chars().take(12).collect(),
            formed_at_date: date.clone(),
            members: members.join("; "),
        });
    }
    Ok(rows)
}

/// Member-path sets of every non-trivial SCC (cycle) in `graph`, each
/// sorted for deterministic output.
fn cycle_member_sets(graph: &ImportGraph) -> Vec<Vec<String>> {
    tarjan_scc(&graph.adj)
        .into_iter()
        .filter(|comp| comp.len() >= 2)
        .map(|comp| {
            let mut paths: Vec<String> = comp
                .into_iter()
                .map(|id| graph.id_to_path[id].clone())
                .collect();
            paths.sort();
            paths
        })
        .collect()
}

/// Binary-search `commits` (oldest → newest) for the earliest index at
/// which `members` form a single dependency cycle. The predicate is true
/// at the newest commit by construction (the cycle came from there), so
/// the search always converges.
fn bisect_formation<R: Repo>(
    db: &FactsDb,
    repo: &R,
    commits: &[(String, String, String)],
    members: &[String],
) -> Result<usize> {
    let mut lo = 0usize;
    let mut hi = commits.len() - 1; // present here by construction
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let (rev, ts, _) = &commits[mid];
        let graph = import_graph_at_rev(db, repo, rev, ts)?;
        if members_form_one_cycle(&graph, members) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

/// True iff every path in `members` is present in `graph` AND they all
/// land in the same strongly-connected component (so they are mutually
/// cyclic at this revision).
fn members_form_one_cycle(graph: &ImportGraph, members: &[String]) -> bool {
    // Map every member to a node id; bail if any isn't present yet.
    let mut ids = Vec::with_capacity(members.len());
    for p in members {
        match graph.path_to_id.get(p) {
            Some(&id) => ids.push(id),
            None => return false,
        }
    }
    if ids.len() < 2 {
        return false;
    }
    // SCC id per node, then check all members share one component.
    let sccs = tarjan_scc(&graph.adj);
    let mut scc_of = vec![usize::MAX; graph.len()];
    for (ci, comp) in sccs.iter().enumerate() {
        for &n in comp {
            scc_of[n] = ci;
        }
    }
    let first = scc_of[ids[0]];
    ids.iter().all(|&id| scc_of[id] == first)
}
