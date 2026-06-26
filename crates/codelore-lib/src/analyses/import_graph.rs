//! Shared structural-import-graph kernel.
//!
//! Builds the directed file→file import graph from the `imports` table
//! and computes strongly-connected components. Reused by the
//! architecture analyses that reason over structure rather than
//! history (`dependency-cycles`, and the reachability-based metrics).
//!
//! The SCC routine is a hand-rolled **iterative** Tarjan (Tarjan 1972).
//! Iterative, not recursive, because a long import chain (a 50k-file
//! monorepo can have deep transitive `use` paths) would overflow the
//! call stack with the recursive formulation. The analysis crate
//! deliberately avoids `petgraph` (its optional 0.8 dep conflicts with
//! `leiden-rs`), and `unsafe` is forbidden workspace-wide — so this is
//! a plain `Vec`-based adjacency walk.

use std::collections::{HashMap, HashSet};

use crate::Result;
use crate::facts::FactsDb;

/// The directed structural import graph. Nodes are repo-relative file
/// paths that appear as a resolved import endpoint; edges are
/// `src → target` ("src imports target").
pub struct ImportGraph {
    /// Dense node id → path.
    pub id_to_path: Vec<String>,
    /// Path → dense node id.
    pub path_to_id: HashMap<String, usize>,
    /// Adjacency: `adj[u]` is the set of nodes `u` imports (deduped,
    /// self-loops removed).
    pub adj: Vec<Vec<usize>>,
}

impl ImportGraph {
    /// Number of nodes in the graph.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id_to_path.len()
    }

    /// Whether the graph has no nodes (no resolved import edges).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id_to_path.is_empty()
    }
}

/// Build the directed import graph from the resolved edges in the
/// `imports` table (`target_path IS NOT NULL`). Parallel edges are
/// deduped and self-loops dropped — neither affects reachability or
/// SCC membership, and removing them keeps the adjacency tight.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors.
pub fn build_import_graph(db: &FactsDb) -> Result<ImportGraph> {
    let edges: Vec<(String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT src_path, target_path FROM imports WHERE target_path IS NOT NULL",
        [],
        "import-graph edges",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;

    let mut path_to_id: HashMap<String, usize> = HashMap::new();
    let mut id_to_path: Vec<String> = Vec::new();

    // Dedup edges into a set first so parallel imports collapse.
    let mut edge_set: HashSet<(usize, usize)> = HashSet::with_capacity(edges.len());
    for (src, tgt) in &edges {
        if src == tgt {
            continue; // drop self-loops (resolver re-export artifacts)
        }
        let s = intern(src, &mut path_to_id, &mut id_to_path);
        let t = intern(tgt, &mut path_to_id, &mut id_to_path);
        edge_set.insert((s, t));
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); id_to_path.len()];
    for (s, t) in edge_set {
        adj[s].push(t);
    }

    Ok(ImportGraph {
        id_to_path,
        path_to_id,
        adj,
    })
}

/// Intern a path into the dense id space, returning its id.
fn intern(p: &str, path_to_id: &mut HashMap<String, usize>, id_to_path: &mut Vec<String>) -> usize {
    if let Some(&id) = path_to_id.get(p) {
        return id;
    }
    let id = id_to_path.len();
    path_to_id.insert(p.to_owned(), id);
    id_to_path.push(p.to_owned());
    id
}

/// Strongly-connected components of a directed graph via iterative
/// Tarjan. Each returned `Vec` is the node ids of one SCC. A singleton
/// component is a node on no cycle; a component of size ≥ 2 is a
/// dependency cycle (tangle).
///
/// Tarjan emits components in reverse-topological order; callers that
/// need a stable order should sort.
#[must_use]
pub fn tarjan_scc(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNSET: usize = usize::MAX;
    let n = adj.len();
    let mut indices = vec![UNSET; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut tstack: Vec<usize> = Vec::new();
    let mut sccs: Vec<Vec<usize>> = Vec::new();
    let mut idx = 0usize;
    // DFS work stack of (node, next-child-index-to-examine).
    let mut call: Vec<(usize, usize)> = Vec::new();

    for s in 0..n {
        if indices[s] != UNSET {
            continue;
        }
        call.push((s, 0));
        while let Some(&(node, start_child)) = call.last() {
            if start_child == 0 {
                // First visit of `node`.
                indices[node] = idx;
                low[node] = idx;
                idx += 1;
                tstack.push(node);
                on_stack[node] = true;
            }

            // Walk children from the saved resume point. On the first
            // unvisited child, save resume = j+1 and recurse.
            let mut recursed = false;
            let mut j = start_child;
            while j < adj[node].len() {
                let child = adj[node][j];
                if indices[child] == UNSET {
                    if let Some(top) = call.last_mut() {
                        top.1 = j + 1;
                    }
                    call.push((child, 0));
                    recursed = true;
                    break;
                } else if on_stack[child] && indices[child] < low[node] {
                    // Back/cross edge to a node still on the stack.
                    low[node] = indices[child];
                }
                j += 1;
            }
            if recursed {
                continue;
            }

            // All children processed: if `node` is an SCC root, pop it.
            if low[node] == indices[node] {
                let mut comp: Vec<usize> = Vec::new();
                while let Some(w) = tstack.pop() {
                    on_stack[w] = false;
                    comp.push(w);
                    if w == node {
                        break;
                    }
                }
                sccs.push(comp);
            }
            call.pop();
            // Propagate this node's lowlink up to its parent.
            if let Some(&(parent, _)) = call.last()
                && low[node] < low[parent]
            {
                low[parent] = low[node];
            }
        }
    }
    sccs
}

/// Per-node transitive reachability counts over the import graph.
pub struct Reach {
    /// SCC id of each node (dense, indexed by node id).
    pub scc_of: Vec<usize>,
    /// Member count of each SCC (indexed by SCC id).
    pub scc_size: Vec<usize>,
    /// Visibility fan-in: number of nodes that can reach this node
    /// (directly or transitively), including its own SCC. The column
    /// sum of the reflexive transitive-closure (visibility) matrix.
    pub vfi: Vec<u32>,
    /// Visibility fan-out: number of nodes reachable from this node,
    /// including its own SCC. The row sum of the visibility matrix.
    pub vfo: Vec<u32>,
}

/// Compute per-node visibility fan-in / fan-out over the directed
/// import graph, given its SCCs (from [`tarjan_scc`]).
///
/// Method (Baldwin, `MacCormack` & Rusnak 2014, "Hidden Structure"):
/// condense the graph to its SCC DAG, then propagate reach-sets in
/// Tarjan's emission order (which is reverse-topological — a component
/// is emitted only after every component it can reach). Each node's
/// `vfo` is the total size of the SCCs reachable from its SCC; `vfi` is
/// the total size of the SCCs that can reach it. Self is included
/// (the visibility matrix is reflexive). Propagation cost — the metric
/// "a change to a random file can reach X% of the system" — is
/// `sum(vfo) / n²` = `mean(vfi) / n`.
///
/// Reach-sets are kept on the *condensation* and stay sparse for real
/// import graphs; dense pathological graphs trade memory for the exact
/// count (no N×N matrix is ever materialised).
#[must_use]
pub fn reachability(adj: &[Vec<usize>], sccs: &[Vec<usize>]) -> Reach {
    let n = adj.len();
    let c = sccs.len();
    let mut scc_of = vec![0usize; n];
    let mut scc_size = vec![0usize; c];
    for (cid, comp) in sccs.iter().enumerate() {
        scc_size[cid] = comp.len();
        for &node in comp {
            scc_of[node] = cid;
        }
    }

    // Condensation edges (deduped), forward and reversed.
    let mut cond_fwd: Vec<HashSet<usize>> = vec![HashSet::new(); c];
    let mut cond_rev: Vec<HashSet<usize>> = vec![HashSet::new(); c];
    for (u, edges) in adj.iter().enumerate() {
        let cu = scc_of[u];
        for &v in edges {
            let cv = scc_of[v];
            if cu != cv {
                cond_fwd[cu].insert(cv);
                cond_rev[cv].insert(cu);
            }
        }
    }

    // VFO reach: forward closure. Emission order = reverse-topological,
    // so a component's successors are already computed when we reach it.
    let mut reach_fwd: Vec<HashSet<usize>> = vec![HashSet::new(); c];
    for cid in 0..c {
        let mut set = HashSet::new();
        set.insert(cid);
        for &succ in &cond_fwd[cid] {
            for &r in &reach_fwd[succ] {
                set.insert(r);
            }
        }
        reach_fwd[cid] = set;
    }
    // VFI reach: reverse closure. Process in reverse emission order so a
    // component's predecessors (ancestors) are computed first.
    let mut reach_rev: Vec<HashSet<usize>> = vec![HashSet::new(); c];
    for cid in (0..c).rev() {
        let mut set = HashSet::new();
        set.insert(cid);
        for &pred in &cond_rev[cid] {
            for &r in &reach_rev[pred] {
                set.insert(r);
            }
        }
        reach_rev[cid] = set;
    }

    let sum_sizes = |set: &HashSet<usize>| -> u32 {
        u32::try_from(set.iter().map(|&r| scc_size[r]).sum::<usize>()).unwrap_or(u32::MAX)
    };
    let mut vfi = vec![0u32; n];
    let mut vfo = vec![0u32; n];
    for node in 0..n {
        let cid = scc_of[node];
        vfo[node] = sum_sizes(&reach_fwd[cid]);
        vfi[node] = sum_sizes(&reach_rev[cid]);
    }

    Reach {
        scc_of,
        scc_size,
        vfi,
        vfo,
    }
}

#[cfg(test)]
mod tests {
    use super::{reachability, tarjan_scc};
    use std::collections::BTreeSet;

    /// Normalise SCC output to a comparable set-of-sorted-sets so tests
    /// don't depend on Tarjan's emission order or intra-component order.
    fn normalize(sccs: Vec<Vec<usize>>) -> BTreeSet<Vec<usize>> {
        sccs.into_iter()
            .map(|mut c| {
                c.sort_unstable();
                c
            })
            .collect()
    }

    #[test]
    fn empty_graph_has_no_components() {
        assert!(tarjan_scc(&[]).is_empty());
    }

    #[test]
    fn dag_yields_only_singletons() {
        // 0 → 1 → 2, plus 0 → 2.
        let adj = vec![vec![1, 2], vec![2], vec![]];
        let got = normalize(tarjan_scc(&adj));
        let want: BTreeSet<Vec<usize>> = [vec![0], vec![1], vec![2]].into_iter().collect();
        assert_eq!(got, want);
    }

    #[test]
    fn three_cycle_is_one_component() {
        // 0 → 1 → 2 → 0.
        let adj = vec![vec![1], vec![2], vec![0]];
        let got = normalize(tarjan_scc(&adj));
        let want: BTreeSet<Vec<usize>> = [vec![0, 1, 2]].into_iter().collect();
        assert_eq!(got, want);
    }

    #[test]
    fn two_cycles_joined_by_a_bridge_stay_separate() {
        // Cycle A {0,1}: 0↔1. Cycle B {3,4}: 3↔4. Bridge 1 → 2 → 3.
        let adj = vec![
            vec![1],    // 0
            vec![0, 2], // 1
            vec![3],    // 2
            vec![4],    // 3
            vec![3],    // 4
        ];
        let got = normalize(tarjan_scc(&adj));
        let want: BTreeSet<Vec<usize>> = [vec![0, 1], vec![2], vec![3, 4]].into_iter().collect();
        assert_eq!(got, want);
    }

    #[test]
    fn every_node_appears_in_exactly_one_component() {
        let adj = vec![vec![1], vec![2, 0], vec![3], vec![1], vec![]];
        let sccs = tarjan_scc(&adj);
        let mut seen = vec![false; adj.len()];
        let mut count = 0;
        for comp in &sccs {
            for &v in comp {
                assert!(!seen[v], "node {v} appeared in two components");
                seen[v] = true;
                count += 1;
            }
        }
        assert_eq!(count, adj.len(), "every node must be covered");
    }

    #[test]
    fn reachability_on_a_chain() {
        // 0 → 1 → 2. Each node is its own SCC.
        let adj = vec![vec![1], vec![2], vec![]];
        let r = reachability(&adj, &tarjan_scc(&adj));
        // vfo (reachable downstream, incl self): 3, 2, 1.
        assert_eq!(r.vfo, vec![3, 2, 1]);
        // vfi (who reaches me, incl self): 1, 2, 3.
        assert_eq!(r.vfi, vec![1, 2, 3]);
        // Propagation cost = sum(vfo) / n² = 6 / 9.
        assert_eq!(r.vfo.iter().sum::<u32>(), 6);
    }

    #[test]
    fn reachability_on_a_full_cycle_is_total() {
        // 0 → 1 → 2 → 0: one SCC of size 3, everything reaches everything.
        let adj = vec![vec![1], vec![2], vec![0]];
        let r = reachability(&adj, &tarjan_scc(&adj));
        assert_eq!(r.vfo, vec![3, 3, 3]);
        assert_eq!(r.vfi, vec![3, 3, 3]);
        // Propagation cost = 9 / 9 = 1.0 — a change touches everything.
        assert_eq!(r.vfo.iter().sum::<u32>(), 9);
    }
}
