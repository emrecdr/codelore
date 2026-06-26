//! `dependency-cycles` analysis — non-trivial strongly-connected
//! components of the structural import graph.
//!
//! A set of files that (transitively) import each other forms a cycle /
//! tangle: none can be compiled, tested, understood, or replaced in
//! isolation, and a change to any one can ripple to all. This is
//! Arcan's "Cyclic Dependency" architectural smell (Fontana et al.
//! 2017) and the red block a Dependency-Structure-Matrix shows on the
//! diagonal (Sangal et al. 2005). Cycles are exactly the SCCs of size
//! ≥ 2 (Tarjan 1972), computed by the shared
//! [`import_graph`](crate::analyses::import_graph) kernel.
//!
//! Accuracy follows the import resolver's language coverage (Rust +
//! Python + JS/TS resolve `target_path`; Java imports stay unresolved),
//! same caveat as `god_classes` fan-in.

use crate::analyses::import_graph::{build_import_graph, tarjan_scc};
use crate::facts::FactsDb;
use crate::{Options, Result};

/// One member of one dependency cycle. Rows sharing a `cycle_id` belong
/// to the same tangle; `size` is that tangle's member count.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DependencyCycleRow {
    /// Dense 0-indexed cycle id, assigned largest-tangle-first.
    pub cycle_id: u32,
    /// Number of files in this cycle.
    pub size: u32,
    /// A file participating in the cycle.
    pub path: String,
}

/// Run the `dependency-cycles` analysis. Returns one row per
/// (cycle, member file), cycles ranked by size (largest first), members
/// sorted by path.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the import-graph build).
#[tracing::instrument(name = "dependency-cycles", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_dependency_cycles(db: &FactsDb, opts: &Options) -> Result<Vec<DependencyCycleRow>> {
    let graph = build_import_graph(db)?;
    if graph.is_empty() {
        return Ok(Vec::new());
    }

    // Non-trivial SCCs are the cycles. Materialise each as sorted member
    // paths for deterministic output.
    let mut cycles: Vec<Vec<String>> = tarjan_scc(&graph.adj)
        .into_iter()
        .filter(|c| c.len() > 1)
        .map(|c| {
            let mut paths: Vec<String> = c
                .into_iter()
                .map(|id| graph.id_to_path[id].clone())
                .collect();
            paths.sort();
            paths
        })
        .collect();

    // Rank largest tangle first; ties broken by the lexicographically
    // smallest member so the ordering is stable across runs.
    cycles.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| a.first().cmp(&b.first()))
    });

    let mut out: Vec<DependencyCycleRow> = Vec::new();
    for (i, members) in cycles.iter().enumerate() {
        let cycle_id = u32::try_from(i).unwrap_or(u32::MAX);
        let size = u32::try_from(members.len()).unwrap_or(u32::MAX);
        for path in members {
            out.push(DependencyCycleRow {
                cycle_id,
                size,
                path: path.clone(),
            });
        }
    }
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
