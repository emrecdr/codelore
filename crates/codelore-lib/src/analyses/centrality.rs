//! Centrality analysis on the behavioural coupling graph.
//!
//! Promotes the SoC-style `coupling_centrality_v1` primitive (a bare
//! `COUNT(*)` of Fisher-significant partners, materialised in
//! `code_health::run_code_health`) to a first-class analysis with four
//! per-file scores:
//!
//! - **degree**: count of Fisher-significant coupling partners (the
//!   primitive `code_health` already used; kept as the network-science
//!   baseline so the new analysis subsumes the old behaviour).
//! - **weighted-degree**: sum of edge `degree` weights (the
//!   `100 * shared / average_revs` ratio from [`CouplingRow`]) over all
//!   Fisher-significant partners. Captures "this file is coupled to N
//!   partners strongly", not just "to N partners".
//! - **pagerank**: power-iteration `PageRank` with damping 0.85,
//!   weighted by the same edge degrees. Captures "this file is coupled
//!   to other well-coupled files" — the recursive influence signal.
//! - **eigenvector**: power-iteration eigenvector centrality, also
//!   weighted. Same family as `PageRank` but no damping factor —
//!   converges faster on the connected component and complements
//!   `PageRank`'s bias toward dangling nodes.
//!
//! All four are computed in one pass on a hash-adjacency
//! representation (no [`petgraph`] graph construction overhead — the
//! coupling graph is sparse, edges already de-duplicated by
//! `(entity_a, entity_b)` lexicographic ordering).
//!
//! # Why these four
//!
//! Skipped: **betweenness** (`O(V·E)` via Brandes — ~3.5B ops on a
//! linux-kernel-scale 70k-node coupling graph), **closeness** (`O(V³)`
//! — infeasible). `PageRank` + eigenvector together cover the
//! "influence in a recursive-trust sense" axis that betweenness
//! approximates more expensively.
//!
//! Power iteration is the chosen numeric kernel for both `PageRank` and
//! eigenvector centrality because:
//!
//! 1. It needs no external linear-algebra crate (no `nalgebra` /
//!    `ndarray-linalg` transitive bloat — both pull in BLAS-shaped
//!    dep trees for a single matrix-vector multiply).
//! 2. The coupling graph's spectral gap is large in practice
//!    (~10-15 iterations to converge to 1e-6 on the codelore-self
//!    ~600-edge graph; ~30 on the linux-kernel-scale one).
//! 3. The graph is undirected so the dominant eigenvector is
//!    guaranteed positive (Perron-Frobenius), no sign-flip handling.
//!
//! # Research basis
//!
//! - Degree centrality: classical network-analysis primitive (see e.g.
//!   Newman 2010 §7.1).
//! - Weighted degree: same primitive over the change-coupling weight
//!   space; `CodeLore`-specific instantiation.
//! - `PageRank`: Brin & Page 1998. Damping 0.85 is the canonical
//!   default.
//! - Eigenvector centrality: Bonacich 1972.
//!
//! See `docs/research-foundations.md` entry "centrality" for the
//! grounded write-up.
//!
//! [`petgraph`]: https://docs.rs/petgraph
//! [`CouplingRow`]: crate::analyses::coupling::CouplingRow

use std::collections::HashMap;

use crate::analyses::coupling::{CouplingRow, run_coupling};
use crate::facts::FactsDb;
use crate::{Options, Result};

/// `PageRank` damping factor (Brin & Page 1998 canonical default).
const PAGERANK_DAMPING: f64 = 0.85;
/// Iteration cap for both `PageRank` and eigenvector power iteration.
/// Empirically 10-15 iterations reach 1e-6 convergence on coupling
/// graphs we've measured; doubling that gives headroom for pathological
/// fixtures without runaway wall-clock.
const POWER_ITER_MAX: usize = 30;
/// Convergence threshold: L1 distance between successive iterates.
const POWER_ITER_EPSILON: f64 = 1e-6;

/// One row per file appearing in at least one Fisher-significant coupling
/// pair. Files with zero coupling partners are omitted (degree 0 across
/// every variant — would just be noise).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CentralityRow {
    pub path: String,
    pub degree: u32,
    pub weighted_degree: f64,
    pub pagerank: f64,
    pub eigenvector: f64,
}

/// Compute the four centrality scores per file on the Fisher-significant
/// coupling graph.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] if the underlying coupling query
/// fails. An empty coupling result (small repos, threshold filtered
/// everything out) yields an empty `Vec` — not an error.
#[tracing::instrument(name = "centrality", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_centrality(db: &FactsDb, opts: &Options) -> Result<Vec<CentralityRow>> {
    let pairs = run_coupling(db, opts)?;
    Ok(compute_centrality(&pairs))
}

/// Pure-function core of [`run_centrality`]: takes coupling pairs +
/// returns per-file centrality scores. Split out for direct unit-test
/// access without needing a `FactsDb`.
#[must_use]
pub fn compute_centrality(pairs: &[CouplingRow]) -> Vec<CentralityRow> {
    if pairs.is_empty() {
        return Vec::new();
    }

    // Index every distinct path → numeric id (0..n). Use a deterministic
    // insertion order (first-seen) so a given input always produces the
    // same id assignment — easier diffing across runs.
    let mut path_to_id: HashMap<String, usize> = HashMap::new();
    let mut id_to_path: Vec<String> = Vec::new();
    for pair in pairs {
        for path in [&pair.entity_a, &pair.entity_b] {
            if !path_to_id.contains_key(path) {
                path_to_id.insert(path.clone(), id_to_path.len());
                id_to_path.push(path.clone());
            }
        }
    }
    let n = id_to_path.len();
    // Hash-adjacency: for each node, vec of (neighbour_id, edge_weight).
    // Coupling is undirected so each pair populates both endpoints.
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for pair in pairs {
        let a = path_to_id[&pair.entity_a];
        let b = path_to_id[&pair.entity_b];
        let w = pair.degree; // 100 * shared / average_revs — already normalised
        adj[a].push((b, w));
        adj[b].push((a, w));
    }

    // Degree + weighted-degree are a single linear pass.
    let degree: Vec<u32> = adj
        .iter()
        .map(|edges| u32::try_from(edges.len()).unwrap_or(u32::MAX))
        .collect();
    let weighted_degree: Vec<f64> = adj
        .iter()
        .map(|edges| edges.iter().map(|(_, w)| *w).sum())
        .collect();

    let pagerank = pagerank_weighted(&adj, &weighted_degree);
    let eigenvector = eigenvector_weighted(&adj);

    let mut out: Vec<CentralityRow> = (0..n)
        .map(|i| CentralityRow {
            path: id_to_path[i].clone(),
            degree: degree[i],
            weighted_degree: weighted_degree[i],
            pagerank: pagerank[i],
            eigenvector: eigenvector[i],
        })
        .collect();
    // Sort by PageRank desc as the primary; secondary by path for
    // deterministic ordering when scores tie (common on tiny graphs).
    out.sort_by(|a, b| {
        b.pagerank
            .partial_cmp(&a.pagerank)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path))
    });
    out
}

/// Weighted `PageRank` via power iteration.
///
/// Standard Brin-Page recurrence adapted to weighted edges:
///
/// ```text
/// PR(v) = (1-d)/N + d · Σ_{u ∈ in(v)}  PR(u) · w(u→v) / W(u)
/// ```
///
/// where `W(u)` is the total out-weight of `u`. Initial vector is
/// uniform `1/N`. Dangling nodes (zero weighted out-degree, possible
/// on isolated pairs that don't share a partner) get their `PageRank`
/// mass redistributed uniformly — Brin-Page's standard sink handling.
fn pagerank_weighted(adj: &[Vec<(usize, f64)>], weighted_degree: &[f64]) -> Vec<f64> {
    let n = adj.len();
    if n == 0 {
        return Vec::new();
    }
    // `n as f64` would trip clippy::cast_precision_loss on the usize→f64
    // conversion. Past 2^53 nodes the precision drop is real; our
    // coupling graph caps in the tens of thousands so it's a
    // theoretical-only concern, but the lint forces us to be explicit.
    #[allow(clippy::cast_precision_loss)]
    let n_f = n as f64;
    let teleport = (1.0 - PAGERANK_DAMPING) / n_f;
    let mut rank = vec![1.0 / n_f; n];
    let mut next = vec![0.0_f64; n];
    for _ in 0..POWER_ITER_MAX {
        // Dangling mass: rank concentrated at zero-out-weight nodes.
        // Redistribute uniformly to preserve the L1 invariant (Σ rank = 1).
        let dangling: f64 = rank
            .iter()
            .enumerate()
            .filter(|(i, _)| weighted_degree[*i] == 0.0)
            .map(|(_, r)| *r)
            .sum();
        let dangling_share = PAGERANK_DAMPING * dangling / n_f;

        next.fill(teleport + dangling_share);
        for u in 0..n {
            if weighted_degree[u] == 0.0 {
                continue;
            }
            let share = PAGERANK_DAMPING * rank[u] / weighted_degree[u];
            for &(v, w) in &adj[u] {
                next[v] += share * w;
            }
        }
        // Convergence: L1 delta.
        let delta: f64 = next
            .iter()
            .zip(rank.iter())
            .map(|(n_, r)| (n_ - r).abs())
            .sum();
        std::mem::swap(&mut rank, &mut next);
        if delta < POWER_ITER_EPSILON {
            break;
        }
    }
    rank
}

/// Weighted eigenvector centrality via shifted power iteration on the
/// weighted adjacency matrix.
///
/// Vanilla power iteration on `A` can fail to converge on bipartite
/// graphs (notably the star — leaves form an independent set, the
/// dominant eigenvalue pairs with its negative, and the iterates
/// oscillate). The classical fix: iterate on `A + I` instead. This
/// shift makes every eigenvalue non-negative and strictly separates
/// the dominant direction, so power iteration converges in O(log n)
/// passes even on bipartite cases. The eigenvector of `A + I` is
/// identical to that of `A` (linear shift preserves eigenvectors), so
/// no post-processing is needed.
///
/// Initial vector uniform `1/√N` (L2-normalised); renormalise after
/// each iteration. Perron-Frobenius on the shifted matrix guarantees a
/// non-negative dominant eigenvector.
fn eigenvector_weighted(adj: &[Vec<(usize, f64)>]) -> Vec<f64> {
    let n = adj.len();
    if n == 0 {
        return Vec::new();
    }
    #[allow(clippy::cast_precision_loss)]
    let n_f = n as f64;
    let init = 1.0 / n_f.sqrt();
    let mut vec_curr = vec![init; n];
    let mut vec_next = vec![0.0_f64; n];
    for _ in 0..POWER_ITER_MAX {
        // v_{k+1} = (A + I) · v_k. Starting from the v_curr contribution
        // (the "I" part) means even a bipartite-pathology graph
        // accumulates a stable signal each pass.
        vec_next.copy_from_slice(&vec_curr);
        for u in 0..n {
            for &(v, w) in &adj[u] {
                vec_next[v] += w * vec_curr[u];
            }
        }
        // L2-normalise. Guard against the degenerate all-zero case
        // (would happen on a graph with no edges, which is already
        // ruled out by the empty-input early return — but defence in
        // depth never hurt).
        let norm: f64 = vec_next.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm == 0.0 {
            return vec_curr;
        }
        for cell in &mut vec_next {
            *cell /= norm;
        }
        // Convergence: L1 delta.
        let delta: f64 = vec_next
            .iter()
            .zip(vec_curr.iter())
            .map(|(n_, c)| (n_ - c).abs())
            .sum::<f64>();
        std::mem::swap(&mut vec_curr, &mut vec_next);
        if delta < POWER_ITER_EPSILON {
            break;
        }
    }
    vec_curr
}

/// Convenience: rebuild centrality directly from a [`CouplingRow`] slice
/// for in-memory orchestration (e.g. `code_health` consumes the same
/// pairs to compute coupling centrality alongside its other inputs;
/// without this helper it would either pay a double-`run_coupling` cost
/// or carry SQL-side state). The error path is unreachable on the
/// in-memory branch — the trait signature keeps callers honest.
///
/// # Errors
///
/// Never returns `Err` in practice; the `Result` shape is reserved for
/// future expansions (e.g. NaN propagation guards).
pub fn from_coupling_pairs(pairs: &[CouplingRow]) -> Result<Vec<CentralityRow>> {
    Ok(compute_centrality(pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(a: &str, b: &str, degree: f64) -> CouplingRow {
        CouplingRow {
            entity_a: a.into(),
            entity_b: b.into(),
            shared: 1,
            revs_a: 10,
            revs_b: 10,
            average_revs: 10,
            degree,
            fisher_p: 0.01,
        }
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(compute_centrality(&[]).is_empty());
    }

    #[test]
    fn star_graph_centrality_picks_hub() {
        // A star: hub--leaf1, hub--leaf2, hub--leaf3.
        let pairs = vec![
            pair("hub", "leaf1", 50.0),
            pair("hub", "leaf2", 50.0),
            pair("hub", "leaf3", 50.0),
        ];
        let result = compute_centrality(&pairs);
        // 4 files (hub + 3 leaves).
        assert_eq!(result.len(), 4);
        // Hub has the highest score across every variant.
        let hub = result.iter().find(|r| r.path == "hub").unwrap();
        assert_eq!(hub.degree, 3);
        assert!((hub.weighted_degree - 150.0).abs() < 1e-9);
        // PageRank: hub > leaves; verify the inequality holds.
        let leaf_pr = result
            .iter()
            .filter(|r| r.path.starts_with("leaf"))
            .map(|r| r.pagerank)
            .next()
            .unwrap();
        assert!(hub.pagerank > leaf_pr);
        // Eigenvector: hub eigenvector strictly greater than any leaf.
        let leaf_ev = result
            .iter()
            .filter(|r| r.path.starts_with("leaf"))
            .map(|r| r.eigenvector)
            .next()
            .unwrap();
        assert!(hub.eigenvector > leaf_ev);
    }

    #[test]
    fn pagerank_sums_to_one_within_epsilon() {
        let pairs = vec![
            pair("a", "b", 30.0),
            pair("b", "c", 40.0),
            pair("c", "a", 50.0),
        ];
        let result = compute_centrality(&pairs);
        let sum: f64 = result.iter().map(|r| r.pagerank).sum();
        // Power-iteration PageRank conserves L1 mass (sum=1) by
        // construction (teleport + dangling redistribute on each pass).
        assert!((sum - 1.0).abs() < 1e-6, "pagerank sum = {sum}");
    }

    #[test]
    fn eigenvector_is_l2_normalised() {
        let pairs = vec![
            pair("a", "b", 30.0),
            pair("b", "c", 40.0),
            pair("c", "a", 50.0),
            pair("a", "d", 20.0),
        ];
        let result = compute_centrality(&pairs);
        let norm_sq: f64 = result.iter().map(|r| r.eigenvector.powi(2)).sum();
        // Power iteration with per-step L2 renormalisation converges
        // to a unit-L2 vector.
        assert!(
            (norm_sq - 1.0).abs() < 1e-6,
            "eigenvector L2 norm² = {norm_sq}"
        );
    }

    #[test]
    fn weighted_degree_respects_edge_weights() {
        let pairs = vec![pair("a", "b", 10.0), pair("a", "c", 90.0)];
        let result = compute_centrality(&pairs);
        let a = result.iter().find(|r| r.path == "a").unwrap();
        assert!((a.weighted_degree - 100.0).abs() < 1e-9);
        // But its unweighted degree is 2.
        assert_eq!(a.degree, 2);
    }

    #[test]
    fn output_sorted_by_pagerank_descending() {
        let pairs = vec![
            pair("center", "a", 100.0),
            pair("center", "b", 100.0),
            pair("center", "c", 100.0),
            pair("center", "d", 100.0),
            pair("a", "b", 10.0),
        ];
        let result = compute_centrality(&pairs);
        // Verify monotone-descending PageRank.
        for w in result.windows(2) {
            assert!(
                w[0].pagerank >= w[1].pagerank,
                "{} ({}) >= {} ({})",
                w[0].path,
                w[0].pagerank,
                w[1].path,
                w[1].pagerank,
            );
        }
        // The hub `center` is the top.
        assert_eq!(result[0].path, "center");
    }
}
