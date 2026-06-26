//! `architecture-roles` analysis — per-file architectural role from the
//! structural import graph's "hidden structure" (Baldwin, `MacCormack` &
//! Rusnak 2014).
//!
//! Each file is classified by its transitive **visibility fan-in**
//! (`vfi`: how many files reach it) and **visibility fan-out** (`vfo`:
//! how many it reaches), relative to the system's Core:
//!
//! - **core** — a member of the largest cyclic group (the dominant
//!   SCC). The architectural "knot" everything routes through.
//! - **shared** — depended on as widely as the Core but depends on
//!   little (`vfi ≥ vfi_core`, `vfo < vfo_core`): utilities, libraries.
//! - **control** — depends on as much as the Core but little depends on
//!   it (`vfi < vfi_core`, `vfo ≥ vfo_core`): orchestrators, `main`.
//! - **periphery** — low on both axes: leaf features (the healthy bulk).
//!
//! When the graph is acyclic (no Core), roles are classified relative to
//! the median `vfi`/`vfo` instead — there is no dominant knot to anchor
//! on, so "as central as the Core" becomes "above the median".
//!
//! `reach_pct` = `vfo / file_count × 100` is the per-file downstream
//! blast radius ("a change here can reach X% of the system"); the
//! repo-level mean of `vfo / n` is `MacCormack`'s **propagation cost**.
//!
//! Accuracy follows the import resolver's language coverage, same caveat
//! as `god_classes` fan-in.

use crate::analyses::import_graph::{build_import_graph, reachability, tarjan_scc, topo_levels};
use crate::facts::FactsDb;
use crate::{Options, Result};

/// One file's architectural role + visibility reach.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchitectureRoleRow {
    pub path: String,
    /// `core` | `shared` | `control` | `periphery`.
    pub role: String,
    /// Visibility fan-in: files that reach this one (transitively).
    pub vfi: u32,
    /// Visibility fan-out: files this one reaches (transitively).
    pub vfo: u32,
    /// Whether this file sits in a dependency cycle (SCC of size ≥ 2).
    pub in_cycle: bool,
    /// Topological layer: longest dependency path from a file nothing
    /// imports. `0` = entry points; deeper = foundations.
    pub level: u32,
    /// Downstream blast radius `vfo / file_count × 100`.
    pub reach_pct: f64,
}

/// Median of a slice of counts (lower-middle for even lengths). `0` for
/// an empty slice.
fn median(v: &[u32]) -> u32 {
    if v.is_empty() {
        return 0;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    s[s.len() / 2]
}

/// Run the `architecture-roles` analysis. Returns one row per file in
/// the import graph, ranked by visibility fan-in (most depended-upon
/// first).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the import-graph build).
#[tracing::instrument(name = "architecture-roles", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_architecture_roles(db: &FactsDb, opts: &Options) -> Result<Vec<ArchitectureRoleRow>> {
    let graph = build_import_graph(db)?;
    let n = graph.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    let sccs = tarjan_scc(&graph.adj);
    let reach = reachability(&graph.adj, &sccs);
    let levels = topo_levels(&graph.adj, &sccs);

    // The Core is the largest cyclic group (SCC of size ≥ 2), if one
    // exists. Tie-break on the smallest member id for determinism.
    let core_scc: Option<usize> =
        (0..sccs.len())
            .filter(|&i| sccs[i].len() >= 2)
            .max_by_key(|&i| {
                (
                    sccs[i].len(),
                    std::cmp::Reverse(sccs[i].iter().copied().min().unwrap_or(0)),
                )
            });

    // Reference (vfi_core, vfo_core): the Core's shared reach, or the
    // medians when the graph is acyclic.
    let (ref_in, ref_out) = match core_scc {
        Some(cid) => {
            let member = sccs[cid].first().copied().unwrap_or(0);
            (reach.vfi[member], reach.vfo[member])
        }
        None => (median(&reach.vfi), median(&reach.vfo)),
    };

    let n_f = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    let mut out: Vec<ArchitectureRoleRow> = (0..n)
        .map(|node| {
            let cid = reach.scc_of[node];
            let in_cycle = reach.scc_size[cid] >= 2;
            let is_core = core_scc == Some(cid);
            let vfi = reach.vfi[node];
            let vfo = reach.vfo[node];
            let role = if is_core {
                "core"
            } else if vfi >= ref_in && vfo < ref_out {
                "shared"
            } else if vfi < ref_in && vfo >= ref_out {
                "control"
            } else if vfi < ref_in && vfo < ref_out {
                "periphery"
            } else {
                // High on both axes but not in the Core — a heavily
                // depended-upon connector. Group with shared.
                "shared"
            };
            ArchitectureRoleRow {
                path: graph.id_to_path[node].clone(),
                role: role.to_owned(),
                vfi,
                vfo,
                in_cycle,
                level: levels[node],
                reach_pct: 100.0 * f64::from(vfo) / n_f,
            }
        })
        .collect();

    // Most-depended-upon first; tie-break on path for determinism.
    out.sort_by(|a, b| b.vfi.cmp(&a.vfi).then_with(|| a.path.cmp(&b.path)));
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
