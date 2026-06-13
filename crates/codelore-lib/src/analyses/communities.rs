//! `communities` — Leiden community detection on the behavioral coupling
//! graph.
//!
//! Builds an undirected weighted graph where each file with at least one
//! Fisher-significant coupling partner is a node and each pair from
//! [`coupling::run_coupling`] is an edge weighted by `-log10(fisher_p)`
//! (statistical strength of coupling, same enrichment vector used by the
//! `centrality` analysis). Runs the Leiden algorithm (Traag, Waltman &
//! van Eck 2019) to extract modularity-optimal communities, and reports
//! per-file community assignments plus the overall partition quality.
//!
//! ## Why Leiden, not Louvain
//!
//! Leiden is the strict successor to Louvain (Blondel et al. 2008) — same
//! modularity objective Q, but adds a refinement phase that guarantees
//! every community is internally connected. Louvain's well-documented
//! failure mode is producing "broken" communities (an internally
//! disconnected community appearing as one cluster), which on a behavioral
//! coupling graph would mistakenly group two unrelated subsystems that
//! happen to share a bridge file. The refinement phase costs a few
//! percent runtime to make that mistake impossible.
//!
//! Per `CodeLore`'s "modernize, don't migrate" principle, we ship Leiden
//! from day one even though the roadmap entry named Louvain.
//!
//! ## Edge weighting choice
//!
//! `-log10(fisher_p)` puts every weight in `(~1.3, ~308)` (the threshold
//! at `p = 0.05` yields ~1.3; values clamp at `-log10(f64::MIN_POSITIVE)`
//! on the high end). Weighted modularity then rewards densely-coupled
//! pairs over barely-significant ones, which is exactly what
//! community detection should care about — same enrichment vector as
//! `centrality::weighted_degree` so the two analyses tell a consistent
//! story.
//!
//! ## Determinism
//!
//! Leiden's local-move phase visits nodes in random order. We seed the
//! RNG explicitly so two `codelore analyze --analysis communities` runs
//! on the same `HEAD` produce byte-identical output (essential for the
//! cache key invariant and any provenance audit). The seed is currently
//! a fixed constant; if a future user wants statistical sensitivity
//! analysis they can vary it via a flag — out of scope for the initial
//! cut.
//!
//! Research basis: see `docs/research-foundations.md` entry "communities"
//! (Newman 2006 PRE for modularity; Blondel et al. 2008 J.Stat.Mech for
//! Louvain; Traag et al. 2019 Sci.Rep. for Leiden).

use std::collections::BTreeMap;

use leiden_rs::{GraphDataBuilder, Leiden, LeidenConfig};

use crate::analyses::coupling::run_coupling;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Per-file row in the `communities` analysis output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommunityRow {
    /// Canonical file path.
    pub entity: String,
    /// Zero-indexed community id (re-numbered to contiguous `[0, k)`).
    pub community_id: u32,
    /// Number of files in this entity's community.
    pub community_size: u32,
    /// Sum of edge weights connecting this file to other members of the
    /// same community (`-log10(fisher_p)` per partner).
    pub intra_strength: f64,
    /// Sum of edge weights connecting this file to files in *different*
    /// communities. A high `inter_strength / (intra_strength + inter_strength)`
    /// ratio flags a "bridge" file whose changes leak across module
    /// boundaries.
    pub inter_strength: f64,
}

/// Summary statistics for the whole partition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommunityStats {
    /// Modularity Q ∈ `[-0.5, 1.0]`. Newman 2006 reports Q > 0.3
    /// indicates a clearly modular structure on real-world graphs.
    pub modularity: f64,
    /// Number of distinct communities the partition produced.
    pub num_communities: u32,
    /// Number of files (graph nodes) that participated.
    pub num_nodes: u32,
    /// Number of Fisher-significant coupling pairs (graph edges).
    pub num_edges: u32,
}

/// Deterministic seed for the local-move RNG. See module-level docs.
const LEIDEN_SEED: u64 = 0xC0DE_105E_C0DE_105E;

/// Build the deterministic path ↔ node-id index. `BTreeMap` iteration is
/// sorted, so two runs against the same pair set produce identical node
/// numbering — important for cache-stable output.
fn assign_node_ids(pairs: &[crate::analyses::coupling::CouplingRow]) -> Vec<String> {
    let mut path_to_id: BTreeMap<String, usize> = BTreeMap::new();
    for p in pairs {
        let next_id = path_to_id.len();
        path_to_id.entry(p.entity_a.clone()).or_insert(next_id);
        let next_id = path_to_id.len();
        path_to_id.entry(p.entity_b.clone()).or_insert(next_id);
    }
    let mut id_to_path: Vec<String> = vec![String::new(); path_to_id.len()];
    for (path, id) in &path_to_id {
        id_to_path[*id].clone_from(path);
    }
    id_to_path
}

/// Defensive: the `fishers_exact` crate (called via
/// `analyses::coupling::fisher_two_tail`) can in theory return a p-value
/// of exactly 0.0 on a degenerate 2×2 contingency table — there's no
/// upstream clamp. `log10(0)` is `-inf` and would poison every downstream
/// modularity calculation. Clamp to `-log10(f64::MIN_POSITIVE) ≈ 307.6`
/// instead: the metric stays finite and the pathological pair ranks as
/// the maximum-strength contributor it morally is.
fn edge_weight(fisher_p: f64) -> f64 {
    if fisher_p > 0.0 {
        -fisher_p.log10()
    } else {
        -f64::MIN_POSITIVE.log10()
    }
}

/// Weighted edge in the graph: `(src_node_id, dst_node_id, weight)`.
type Edge = (usize, usize, f64);

fn build_graph(
    pairs: &[crate::analyses::coupling::CouplingRow],
    id_to_path: &[String],
) -> Result<(leiden_rs::GraphData, Vec<Edge>)> {
    let path_to_id: BTreeMap<&str, usize> = id_to_path
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    let edges: Vec<Edge> = pairs
        .iter()
        .map(|p| {
            (
                path_to_id[p.entity_a.as_str()],
                path_to_id[p.entity_b.as_str()],
                edge_weight(p.fisher_p),
            )
        })
        .collect();
    let mut builder = GraphDataBuilder::new(id_to_path.len());
    for (src, dst, w) in &edges {
        builder
            .add_edge(*src, *dst, *w)
            .map_err(|e| CodeLoreError::Analysis(format!("leiden add_edge: {e}")))?;
    }
    let graph = builder
        .build()
        .map_err(|e| CodeLoreError::Analysis(format!("leiden build: {e}")))?;
    Ok((graph, edges))
}

pub fn run_communities(
    db: &FactsDb,
    opts: &Options,
) -> Result<(Vec<CommunityRow>, CommunityStats)> {
    // `--rows N` truncation must happen AFTER aggregation; the partition
    // is meaningful only over the full Fisher-significant graph.
    let pairs = run_coupling(db, &opts.with_no_row_limit())?;
    if pairs.is_empty() {
        return Ok((
            Vec::new(),
            CommunityStats {
                modularity: 0.0,
                num_communities: 0,
                num_nodes: 0,
                num_edges: 0,
            },
        ));
    }

    let id_to_path = assign_node_ids(&pairs);
    let n = id_to_path.len();
    let (graph, edges) = build_graph(&pairs, &id_to_path)?;

    let config = LeidenConfig {
        seed: Some(LEIDEN_SEED),
        ..LeidenConfig::default()
    };
    let mut output = Leiden::new(config)
        .run(&graph)
        .map_err(|e| CodeLoreError::Analysis(format!("leiden run: {e}")))?;
    output.partition.renumber();

    // Walk each edge once more to split each node's incident weight
    // into intra-community vs inter-community. Sparse graph, linear pass.
    let mut intra = vec![0.0_f64; n];
    let mut inter = vec![0.0_f64; n];
    for (src, dst, w) in &edges {
        let same = output.partition.community_of(*src) == output.partition.community_of(*dst);
        let bucket = if same { &mut intra } else { &mut inter };
        bucket[*src] += *w;
        bucket[*dst] += *w;
    }

    let sizes = output.partition.community_sizes();
    let mut rows: Vec<CommunityRow> = (0..n)
        .map(|node| {
            let community = output.partition.community_of(node);
            CommunityRow {
                entity: id_to_path[node].clone(),
                community_id: u32::try_from(community).unwrap_or(u32::MAX),
                community_size: u32::try_from(*sizes.get(community).unwrap_or(&0))
                    .unwrap_or(u32::MAX),
                intra_strength: intra[node],
                inter_strength: inter[node],
            }
        })
        .collect();

    // Stable order: community asc, intra desc, inter desc, entity asc.
    // Groups community members contiguously and surfaces the strongest
    // in-group nodes first within each cluster.
    rows.sort_by(|a, b| {
        a.community_id
            .cmp(&b.community_id)
            .then_with(|| {
                b.intra_strength
                    .partial_cmp(&a.intra_strength)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                b.inter_strength
                    .partial_cmp(&a.inter_strength)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.entity.cmp(&b.entity))
    });

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }

    let stats = CommunityStats {
        modularity: output.quality,
        num_communities: u32::try_from(output.partition.num_communities()).unwrap_or(u32::MAX),
        num_nodes: u32::try_from(n).unwrap_or(u32::MAX),
        num_edges: u32::try_from(edges.len()).unwrap_or(u32::MAX),
    };

    Ok((rows, stats))
}
