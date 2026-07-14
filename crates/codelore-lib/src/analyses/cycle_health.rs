//! `cycle-health` analysis — per-cycle behavioral heat, live/fossil
//! verdict, and the cheapest cut point for each import tangle.
//!
//! `dependency-cycles` lists the members of every non-trivial SCC of the
//! structural import graph; this analysis ranks those tangles by how much
//! they matter *right now* and says where to start dismantling them. It
//! is the structure×history fusion of Kazman & Cai's hotspot lineage
//! (Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns*) applied to the cyclic
//! groups of the "hidden structure" view (Baldwin, `MacCormack` & Rusnak
//! 2014):
//!
//! - **`heat_pct`** — the cycle members' share of repo LOC churn
//!   (`loc_added + loc_deleted`) over the trailing `--window-days`
//!   window, anchored to the repo's last commit date (reproducible on
//!   archived repos), over the lineage-aware changes source. A tangle
//!   nobody touches costs little; a hot one taxes every change.
//! - **`verdict`** — `live` when at least one member appears in a window
//!   commit (a zero-LOC touch still counts), `fossil` otherwise.
//! - **`extract_candidate`** — the member whose trial removal best
//!   dismantles the tangle: smallest largest-surviving-SCC, then fewest
//!   surviving cyclic nodes, then lexicographically smallest path.
//! - **`predicted_pc_drop`** — the whole-graph propagation-cost drop
//!   (`MacCormack`, Rusnak & Baldwin 2006) if the candidate node were
//!   extracted (every edge touching it removed). The trial-removal search
//!   and this prediction run only for tangles of ≤ 64 members; above the
//!   bound the value is absent (honest absence, not an estimate) and the
//!   candidate falls back to the member with the highest in-cycle degree.
//!
//! Accuracy follows the import resolver's language coverage, same caveat
//! as `dependency-cycles`.

use std::collections::HashMap;

use crate::analyses::import_graph::{
    ImportGraph, build_import_graph, build_import_graph_from_edges, graph_metrics, tarjan_scc,
};
use crate::analyses::lineage;
use crate::analyses::query::query_map_collect;
use crate::facts::FactsDb;
use crate::{Options, Result};

/// Largest cycle size for which the per-member trial-removal search and
/// the propagation-cost prediction run. Above this, trial Tarjan per
/// member (O(size × edges) per cycle) stops being worth the precision,
/// and the candidate falls back to the highest in-cycle degree.
const TRIAL_REMOVAL_BOUND: usize = 64;

/// One import cycle's health. One row per non-trivial SCC.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CycleHealthRow {
    /// Dense 0-indexed cycle id, ranked by size (largest first), ties by
    /// lexicographically smallest member path — the same ranking rule as
    /// `dependency-cycles`.
    pub cycle_id: u32,
    /// Number of files in this cycle.
    pub size: u32,
    /// First 3 members lexicographically, `+N more` suffix when the
    /// cycle is larger. Full membership remains `dependency-cycles`' job.
    pub members_preview: String,
    /// Members' share of repo LOC churn over the trailing window, 0–100.
    pub heat_pct: f64,
    /// `live` — at least one member appears in a window commit; `fossil`
    /// otherwise.
    pub verdict: String,
    /// The member whose extraction best dismantles the tangle.
    pub extract_candidate: String,
    /// Whole-graph propagation-cost drop if `extract_candidate` were
    /// removed. Absent for cycles above the trial-removal bound.
    pub predicted_pc_drop: Option<f64>,
}

/// Per-path activity over the trailing window, fetched once for the whole
/// repo so per-cycle sums happen in Rust — no per-cycle SQL round-trips.
struct WindowActivity {
    /// `loc_added + loc_deleted` across window commits touching the path.
    churn: f64,
    /// Window commits touching the path (counts zero-LOC touches).
    revs: i64,
}

/// Run the `cycle-health` analysis. Returns one row per non-trivial SCC
/// of the resolved import graph, sorted by `heat_pct` descending, then
/// `size` descending, then `cycle_id` ascending — the live big tangles
/// first.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the import-graph build or the window-churn query).
#[tracing::instrument(name = "cycle-health", skip_all, fields(window_days = opts.window_days))]
pub fn run_cycle_health(db: &FactsDb, opts: &Options) -> Result<Vec<CycleHealthRow>> {
    let graph = build_import_graph(db)?;
    if graph.is_empty() {
        return Ok(Vec::new());
    }

    // Non-trivial SCCs are the cycles. Sort each cycle's members by path
    // so the preview, the ranking tie-break, and the extraction-candidate
    // tie-break are all lexicographic; rank largest tangle first.
    let mut cycles: Vec<Vec<usize>> = tarjan_scc(&graph.adj)
        .into_iter()
        .filter(|c| c.len() > 1)
        .collect();
    if cycles.is_empty() {
        return Ok(Vec::new());
    }
    for members in &mut cycles {
        members.sort_by(|&a, &b| graph.id_to_path[a].cmp(&graph.id_to_path[b]));
    }
    cycles.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| graph.id_to_path[a[0]].cmp(&graph.id_to_path[b[0]]))
    });

    let window = window_activity(db, opts)?;
    let total_churn: f64 = window.values().map(|w| w.churn).sum();
    let full_pc = graph_metrics(&graph).propagation_cost;

    let mut out: Vec<CycleHealthRow> = Vec::with_capacity(cycles.len());
    for (i, members) in cycles.iter().enumerate() {
        let paths: Vec<&str> = members
            .iter()
            .map(|&id| graph.id_to_path[id].as_str())
            .collect();

        let member_churn: f64 = paths
            .iter()
            .filter_map(|p| window.get(*p))
            .map(|w| w.churn)
            .sum();
        let member_revs: i64 = paths
            .iter()
            .filter_map(|p| window.get(*p))
            .map(|w| w.revs)
            .sum();
        let heat_pct = if total_churn > 0.0 {
            100.0 * member_churn / total_churn
        } else {
            0.0
        };

        let (candidate, predicted_pc_drop) = if members.len() <= TRIAL_REMOVAL_BOUND {
            let cand = extraction_candidate(&graph.adj, members);
            let without = graph_metrics(&graph_without_node(&graph, cand));
            (cand, Some(full_pc - without.propagation_cost))
        } else {
            (highest_degree_member(&graph.adj, members), None)
        };

        out.push(CycleHealthRow {
            cycle_id: u32::try_from(i).unwrap_or(u32::MAX),
            size: u32::try_from(members.len()).unwrap_or(u32::MAX),
            members_preview: members_preview(&paths),
            heat_pct,
            verdict: if member_revs > 0 { "live" } else { "fossil" }.to_owned(),
            extract_candidate: graph.id_to_path[candidate].clone(),
            predicted_pc_drop,
        });
    }

    out.sort_by(|a, b| {
        b.heat_pct
            .total_cmp(&a.heat_pct)
            .then_with(|| b.size.cmp(&a.size))
            .then_with(|| a.cycle_id.cmp(&b.cycle_id))
    });
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}

/// First 3 members (already path-sorted) joined with `", "`, plus a
/// `+N more` suffix when the cycle is larger.
fn members_preview(paths: &[&str]) -> String {
    let head = paths.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    if paths.len() > 3 {
        format!("{head} +{} more", paths.len() - 3)
    } else {
        head
    }
}

/// Fetch per-path window activity (LOC churn + touch count) in one query
/// over the lineage-aware changes source, anchored to the repo's last
/// commit date.
fn window_activity(db: &FactsDb, opts: &Options) -> Result<HashMap<String, WindowActivity>> {
    lineage::materialize_if_needed(db, opts)?;
    let src = lineage::source_table(opts);
    let sql = format!(
        "SELECT ch.path,
                CAST(COALESCE(SUM(ch.loc_added + ch.loc_deleted), 0) AS DOUBLE) AS churn,
                COUNT(ch.rev) AS revs
         FROM {src} ch
         JOIN commits co ON co.rev = ch.rev
         WHERE co.date >= (SELECT MAX(date) FROM commits) - INTERVAL (?) DAY
         GROUP BY ch.path"
    );
    let rows: Vec<(String, f64, i64)> = query_map_collect(
        db,
        &sql,
        duckdb::params![i64::from(opts.window_days)],
        "cycle-health window activity",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(rows
        .into_iter()
        .map(|(path, churn, revs)| (path, WindowActivity { churn, revs }))
        .collect())
}

/// The cycle member whose removal best dismantles the tangle.
///
/// For each member, runs Tarjan on the SCC-induced subgraph minus that
/// member and scores the remnant by `(largest surviving SCC size, total
/// surviving cyclic nodes)` — singleton components are not tangles, so
/// a fully dismantled remnant scores `(0, 0)`. The member minimising
/// that tuple wins; ties keep the earliest member in `scc` order.
/// Callers pass members sorted by path, making the final tie-break the
/// lexicographically smallest path.
pub(crate) fn extraction_candidate(adj: &[Vec<usize>], scc: &[usize]) -> usize {
    let index: HashMap<usize, usize> = scc.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut best = scc[0];
    let mut best_score = (usize::MAX, usize::MAX);
    for (ri, &removed) in scc.iter().enumerate() {
        // Induced subgraph on scc \ {removed}, re-indexed to dense local
        // ids that skip the removed member's slot.
        let local = |i: usize| if i < ri { i } else { i - 1 };
        let mut sub: Vec<Vec<usize>> = vec![Vec::new(); scc.len() - 1];
        for (ui, &u) in scc.iter().enumerate() {
            if ui == ri {
                continue;
            }
            for v in &adj[u] {
                if let Some(&vi) = index.get(v)
                    && vi != ri
                {
                    sub[local(ui)].push(local(vi));
                }
            }
        }
        let mut largest = 0usize;
        let mut cyclic = 0usize;
        for comp in tarjan_scc(&sub) {
            if comp.len() >= 2 {
                largest = largest.max(comp.len());
                cyclic += comp.len();
            }
        }
        if (largest, cyclic) < best_score {
            best_score = (largest, cyclic);
            best = removed;
        }
    }
    best
}

/// Fallback candidate for tangles above the trial-removal bound: the
/// member with the highest in-cycle degree (in + out edges within the
/// SCC). `members` is path-sorted, so ties resolve to the
/// lexicographically smallest member.
fn highest_degree_member(adj: &[Vec<usize>], members: &[usize]) -> usize {
    let index: HashMap<usize, usize> = members.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut deg = vec![0usize; members.len()];
    for (ui, &u) in members.iter().enumerate() {
        for v in &adj[u] {
            if let Some(&vi) = index.get(v) {
                deg[ui] += 1; // out-edge within the SCC
                deg[vi] += 1; // in-edge for the target
            }
        }
    }
    let mut best = 0;
    for i in 1..members.len() {
        if deg[i] > deg[best] {
            best = i;
        }
    }
    members[best]
}

/// Rebuild the import graph from its edge list minus every edge touching
/// `node` — node extraction, for the propagation-cost prediction.
fn graph_without_node(graph: &ImportGraph, node: usize) -> ImportGraph {
    let mut edges: Vec<(String, String)> = Vec::new();
    for (u, targets) in graph.adj.iter().enumerate() {
        if u == node {
            continue;
        }
        for &v in targets {
            if v == node {
                continue;
            }
            edges.push((graph.id_to_path[u].clone(), graph.id_to_path[v].clone()));
        }
    }
    build_import_graph_from_edges(&edges)
}

#[cfg(test)]
mod tests {
    use super::{extraction_candidate, highest_degree_member};

    #[test]
    fn extraction_candidate_picks_the_acyclic_cut() {
        // 3-cycle 0→1→2→0 plus back-edge 1→0. Removing 2 leaves the
        // 2-cycle 0↔1 alive (score (2, 2)); removing 0 or 1 leaves the
        // remnant acyclic (score (0, 0)). The 0/1 tie resolves to the
        // earliest member in the caller-supplied (path-sorted) order.
        let adj = vec![vec![1], vec![2, 0], vec![0]];
        assert_eq!(extraction_candidate(&adj, &[0, 1, 2]), 0);
    }

    #[test]
    fn extraction_candidate_finds_the_articulation_member() {
        // 2-cycle 0↔1 braided with 3-cycle 1→2→3→1: member 1 is the only
        // cut that dismantles every cycle at once (score (0, 0)); every
        // other removal leaves a surviving tangle.
        let adj = vec![vec![1], vec![0, 2], vec![3], vec![1]];
        assert_eq!(extraction_candidate(&adj, &[0, 1, 2, 3]), 1);
    }

    #[test]
    fn extraction_candidate_tie_breaks_by_member_order() {
        // Symmetric 2-cycle: both cuts are equivalent, so the first
        // member in the caller-supplied order (lexicographically smallest
        // path for real callers) wins.
        let adj = vec![vec![1], vec![0]];
        assert_eq!(extraction_candidate(&adj, &[0, 1]), 0);
        assert_eq!(extraction_candidate(&adj, &[1, 0]), 1);
    }

    #[test]
    fn degree_fallback_picks_the_hub() {
        // Star of 2-cycles: 0↔1, 0↔2, 0↔3 — node 0 has in-SCC degree 6,
        // every spoke has 2.
        let adj = vec![vec![1, 2, 3], vec![0], vec![0], vec![0]];
        assert_eq!(highest_degree_member(&adj, &[0, 1, 2, 3]), 0);
        // Symmetric 2-cycle tie → first member in order wins.
        let two = vec![vec![1], vec![0]];
        assert_eq!(highest_degree_member(&two, &[0, 1]), 0);
        assert_eq!(highest_degree_member(&two, &[1, 0]), 1);
    }
}
