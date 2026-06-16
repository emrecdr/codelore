//! Leiden community detection on the behavioural coupling graph.
//!
//! Auto-detects Conway's-law communities from the coupling graph
//! (Fisher-significant pairs from [`coupling::run_coupling`]) using
//! the Leiden algorithm (Traag et al. 2019). Each file lands in
//! exactly one community; the per-community modularity contribution
//! and member list are exposed for downstream consumers.
//!
//! # Algorithm choice
//!
//! Leiden over Louvain (Blondel et al. 2008) — Leiden guarantees
//! well-connected communities and converges to a partition that's a
//! local optimum under both the modularity and CPM quality
//! functions, where Louvain can produce disconnected communities
//! that survive iteration. The Traag 2019 paper documents the
//! pathology Louvain has on real-world graphs. `CodeLore`'s coupling
//! graph is exactly the shape (sparse, weighted, modular) where the
//! Louvain pathology shows up most.
//!
//! # Crate choice
//!
//! `leiden-rs` (Apache-2.0/MIT). We use the no-default-features
//! surface — the upstream default pulls in `gryf` (an alternative
//! graph crate we don't need) and `cli` (a binary we don't ship).
//! Opted back into `rayon` for parallel iteration; meaningful
//! speedup on the linux-kernel-scale coupling graph.
//!
//! # Modularity vs CPM
//!
//! Default quality function: **modularity** (`QualityType::Modularity`,
//! γ = 1.0). The Newman-Girvan modularity has a known resolution
//! limit (small communities can be invisibly merged by the
//! modularity-optimal partition) but it's the most-cited interpretive
//! reference in software-engineering literature, which makes the
//! results comparable to other Conway's-law analyses. CPM is
//! available via the optional [`CommunitiesOptions::cpm_resolution`]
//! knob for users who want resolution-limit-free output.
//!
//! # Research basis
//!
//! - Traag, Waltman, van Eck 2019. "From Louvain to Leiden:
//!   guaranteeing well-connected communities." Scientific Reports.
//!   <https://doi.org/10.1038/s41598-019-41695-z>
//! - Newman & Girvan 2004. "Finding and evaluating community
//!   structure in networks." Phys. Rev. E.
//!
//! See `docs/research-foundations.md` entry "communities" for the
//! full grounded write-up.

use std::collections::HashMap;

use leiden_rs::{GraphDataBuilder, Leiden, LeidenConfig};

use crate::analyses::coupling::run_coupling;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Deterministic RNG seed for the Leiden community-detection pass. See
/// the call site for the rationale: `LeidenConfig::default()` leaves
/// `seed = None` and leiden-rs falls back to wall-clock entropy, which
/// breaks the module's "deterministic across runs" promise.
const LEIDEN_SEED: u64 = 0xC0DE_10E5_AED1_DEED;

/// One row per file mapped to its Leiden community ID. Files with no
/// Fisher-significant coupling partners are omitted (no node, no row).
/// Rows are sorted by `(community_id ASC, path ASC)` so the output is
/// deterministic across runs and friendly to a stable `git diff`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommunityRow {
    pub path: String,
    /// Dense 0-indexed community ID. Sequential after Leiden's
    /// internal label-renumbering pass.
    pub community_id: u32,
    /// Number of other files in this community.
    pub community_size: u32,
}

/// Aggregate output of [`run_communities`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommunitiesResult {
    /// Per-file rows.
    pub rows: Vec<CommunityRow>,
    /// Overall modularity score `Q ∈ [-1.0, 1.0]` of the partition.
    /// Higher = stronger community structure; Q > 0.3 is the
    /// conventional threshold for "meaningful" modularity (Newman 2004).
    pub modularity: f64,
    /// Number of distinct communities Leiden found.
    pub community_count: u32,
}

/// Run Leiden community detection on the Fisher-significant coupling
/// pairs.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] if the upstream coupling query
/// fails or if `leiden-rs` reports an internal error (malformed input,
/// numerical breakdown — neither expected on the well-formed inputs
/// `coupling::run_coupling` produces, but the error path stays typed).
pub fn run_communities(db: &FactsDb, opts: &Options) -> Result<CommunitiesResult> {
    let pairs = run_coupling(db, opts)?;
    if pairs.is_empty() {
        return Ok(CommunitiesResult {
            rows: Vec::new(),
            modularity: 0.0,
            community_count: 0,
        });
    }

    // Index every distinct path → numeric id. Same shape as
    // centrality::compute_centrality — first-seen ordering for
    // determinism.
    let mut path_to_id: HashMap<String, usize> = HashMap::new();
    let mut id_to_path: Vec<String> = Vec::new();
    for pair in &pairs {
        for path in [&pair.entity_a, &pair.entity_b] {
            if !path_to_id.contains_key(path) {
                path_to_id.insert(path.clone(), id_to_path.len());
                id_to_path.push(path.clone());
            }
        }
    }
    let n = id_to_path.len();

    // Build the leiden-rs graph. The builder API takes node count
    // upfront then accepts undirected edges via `add_edge(u, v, w)`.
    // Edge weights are the same `degree = 100 * shared / average_revs`
    // coupling-pair degree that the centrality analysis uses, so the
    // two analyses agree on what "strongly coupled" means.
    let mut builder = GraphDataBuilder::new(n);
    for pair in &pairs {
        let u = path_to_id[&pair.entity_a];
        let v = path_to_id[&pair.entity_b];
        builder.add_edge(u, v, pair.degree).map_err(|e| {
            CodeLoreError::Analysis(format!("leiden-rs add_edge({u}, {v}, …): {e}"))
        })?;
    }
    let graph = builder
        .build()
        .map_err(|e| CodeLoreError::Analysis(format!("leiden-rs build: {e}")))?;

    // Deterministic seed so two back-to-back runs against the same graph
    // produce identical `community_id` columns. `LeidenConfig::default()`
    // leaves `seed = None`, which the leiden-rs crate fills from
    // `rand::rng()` → wall-clock entropy → non-determinism. The module
    // docstring promises "deterministic across runs"; that promise was
    // broken on every cache miss. Constant is arbitrary but recognisable.
    let leiden = Leiden::new(LeidenConfig {
        seed: Some(LEIDEN_SEED),
        ..LeidenConfig::default()
    });
    let result = leiden
        .run(&graph)
        .map_err(|e| CodeLoreError::Analysis(format!("leiden-rs run: {e}")))?;

    // The partition maps each node id to a community id. We
    // renumber the communities into a dense 0..k range using
    // first-occurrence order so the output is stable across runs.
    let raw_assignments: Vec<usize> = (0..n).map(|i| result.partition.community_of(i)).collect();
    let mut dense_id: HashMap<usize, u32> = HashMap::new();
    let mut dense_assignments: Vec<u32> = Vec::with_capacity(n);
    for &raw in &raw_assignments {
        let next_id = u32::try_from(dense_id.len()).unwrap_or(u32::MAX);
        let id = *dense_id.entry(raw).or_insert(next_id);
        dense_assignments.push(id);
    }

    // Per-community sizes for the `community_size` field.
    let mut sizes: HashMap<u32, u32> = HashMap::new();
    for &cid in &dense_assignments {
        *sizes.entry(cid).or_insert(0) += 1;
    }

    let mut rows: Vec<CommunityRow> = (0..n)
        .map(|i| CommunityRow {
            path: id_to_path[i].clone(),
            community_id: dense_assignments[i],
            community_size: *sizes.get(&dense_assignments[i]).unwrap_or(&0),
        })
        .collect();
    rows.sort_by(|a, b| {
        a.community_id
            .cmp(&b.community_id)
            .then(a.path.cmp(&b.path))
    });

    Ok(CommunitiesResult {
        rows,
        modularity: result.quality,
        community_count: u32::try_from(dense_id.len()).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyses::coupling::CouplingRow;

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

    /// Direct test of the partition logic on a fixture: two cliques
    /// connected by a single weak edge. Leiden should put each clique
    /// in its own community.
    #[test]
    fn two_cliques_yield_two_communities() {
        // Clique A: a1, a2, a3 strongly coupled.
        // Clique B: b1, b2, b3 strongly coupled.
        // Bridge:  a1—b1 weakly.
        let pairs = vec![
            pair("a1", "a2", 100.0),
            pair("a1", "a3", 100.0),
            pair("a2", "a3", 100.0),
            pair("b1", "b2", 100.0),
            pair("b1", "b3", 100.0),
            pair("b2", "b3", 100.0),
            pair("a1", "b1", 1.0), // weak bridge
        ];
        // Run the partition logic directly (skipping FactsDb plumbing).
        // We replicate the body of run_communities here so the test
        // doesn't need a real database.
        let mut path_to_id: HashMap<String, usize> = HashMap::new();
        let mut id_to_path: Vec<String> = Vec::new();
        for p in &pairs {
            for x in [&p.entity_a, &p.entity_b] {
                if !path_to_id.contains_key(x) {
                    path_to_id.insert(x.clone(), id_to_path.len());
                    id_to_path.push(x.clone());
                }
            }
        }
        let mut builder = GraphDataBuilder::new(id_to_path.len());
        for p in &pairs {
            builder
                .add_edge(path_to_id[&p.entity_a], path_to_id[&p.entity_b], p.degree)
                .unwrap();
        }
        let graph = builder.build().unwrap();
        let result = Leiden::new(LeidenConfig::default()).run(&graph).unwrap();
        // Modularity Q > 0.3 is the conventional threshold for
        // "meaningful structure". Two weakly-bridged cliques should
        // clear that handily.
        assert!(
            result.quality > 0.3,
            "expected Q > 0.3 on two-clique fixture, got {}",
            result.quality
        );
        // a1, a2, a3 share one community; b1, b2, b3 share another.
        let ca = result.partition.community_of(path_to_id["a1"]);
        assert_eq!(ca, result.partition.community_of(path_to_id["a2"]));
        assert_eq!(ca, result.partition.community_of(path_to_id["a3"]));
        let cb = result.partition.community_of(path_to_id["b1"]);
        assert_eq!(cb, result.partition.community_of(path_to_id["b2"]));
        assert_eq!(cb, result.partition.community_of(path_to_id["b3"]));
        assert_ne!(ca, cb, "two cliques should produce two communities");
    }

    /// Two back-to-back Leiden runs over the same graph with the same
    /// seed must produce identical community assignments. Without a
    /// seed, `LeidenConfig::default()` drew from `rand::rng()` (wall-
    /// clock entropy) and the module's "deterministic across runs"
    /// promise was broken on every cache miss.
    #[test]
    fn leiden_partition_is_deterministic_across_runs() {
        let pairs = vec![
            pair("a1", "a2", 100.0),
            pair("a1", "a3", 100.0),
            pair("a2", "a3", 100.0),
            pair("b1", "b2", 100.0),
            pair("b1", "b3", 100.0),
            pair("b2", "b3", 100.0),
            pair("a1", "b1", 1.0),
        ];
        let mut path_to_id: HashMap<String, usize> = HashMap::new();
        let mut id_to_path: Vec<String> = Vec::new();
        for p in &pairs {
            for x in [&p.entity_a, &p.entity_b] {
                if !path_to_id.contains_key(x) {
                    path_to_id.insert(x.clone(), id_to_path.len());
                    id_to_path.push(x.clone());
                }
            }
        }
        let build = || {
            let mut b = GraphDataBuilder::new(id_to_path.len());
            for p in &pairs {
                b.add_edge(path_to_id[&p.entity_a], path_to_id[&p.entity_b], p.degree)
                    .unwrap();
            }
            b.build().unwrap()
        };
        let seeded = || LeidenConfig {
            seed: Some(LEIDEN_SEED),
            ..LeidenConfig::default()
        };
        let r1 = Leiden::new(seeded()).run(&build()).unwrap();
        let r2 = Leiden::new(seeded()).run(&build()).unwrap();
        let parts1: Vec<_> = (0..id_to_path.len())
            .map(|i| r1.partition.community_of(i))
            .collect();
        let parts2: Vec<_> = (0..id_to_path.len())
            .map(|i| r2.partition.community_of(i))
            .collect();
        assert_eq!(
            parts1, parts2,
            "seeded Leiden must produce identical partitions across runs"
        );
    }
}
