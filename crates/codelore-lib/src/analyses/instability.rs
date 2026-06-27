//! `instability` analysis — Robert C. Martin's package-coupling metrics
//! per file (Martin 1994, "OO Design Quality Metrics: An Analysis of
//! Dependencies"; canonised in *Clean Architecture* 2017).
//!
//! - **Afferent coupling `ca`** — how many files import this one (the
//!   in-degree of the resolved import graph). High `ca` = widely
//!   depended upon.
//! - **Efferent coupling `ce`** — how many files this one imports (the
//!   out-degree). High `ce` = depends on a lot.
//! - **Instability `I = ce / (ca + ce)`** ∈ `[0, 1]`. `I = 0` is
//!   maximally *stable* (much depends on it, it depends on nothing —
//!   hard and risky to change); `I = 1` is maximally *unstable* (it
//!   depends on much, nothing depends on it — cheap to change).
//!
//! The Stable-Dependencies Principle says a file should only depend on
//! files **more** stable than itself; a widely-depended-on file (low
//! `I`, high `ca`) that is nonetheless unstable is the dangerous shape.
//!
//! Abstractness `A` and Distance-from-the-Main-Sequence `D` (the "Zone
//! of Pain") need symbol-level abstract-type counts the file-level
//! import graph doesn't carry, so they are out of scope here.
//!
//! Computed on the resolved import graph, so accuracy follows the
//! resolver's language coverage (Rust + Python + JS/TS).

use crate::analyses::import_graph::build_import_graph;
use crate::facts::FactsDb;
use crate::{Options, Result};

/// One file's Martin coupling metrics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstabilityRow {
    pub path: String,
    /// Afferent coupling: files that import this one (in-degree).
    pub ca: u32,
    /// Efferent coupling: files this one imports (out-degree).
    pub ce: u32,
    /// `ce / (ca + ce)` ∈ [0, 1]. 0 = maximally stable, 1 = unstable.
    pub instability: f64,
}

/// Run the `instability` analysis. Returns one row per file in the
/// import graph, ranked by afferent coupling (most depended-upon first),
/// where instability is most consequential.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the import-graph build).
#[tracing::instrument(name = "instability", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_instability(db: &FactsDb, opts: &Options) -> Result<Vec<InstabilityRow>> {
    let graph = build_import_graph(db)?;
    let n = graph.len();
    if n == 0 {
        return Ok(Vec::new());
    }

    // Efferent = out-degree; afferent = in-degree over the resolved graph.
    let mut ca = vec![0u32; n];
    for targets in &graph.adj {
        for &v in targets {
            ca[v] = ca[v].saturating_add(1);
        }
    }

    let mut out: Vec<InstabilityRow> = (0..n)
        .map(|node| {
            let ce = u32::try_from(graph.adj[node].len()).unwrap_or(u32::MAX);
            let afferent = ca[node];
            let total = f64::from(afferent) + f64::from(ce);
            let instability = if total > 0.0 {
                f64::from(ce) / total
            } else {
                0.0
            };
            InstabilityRow {
                path: graph.id_to_path[node].clone(),
                ca: afferent,
                ce,
                instability,
            }
        })
        .collect();

    // Most depended-upon first; then most unstable; then path.
    out.sort_by(|a, b| {
        b.ca.cmp(&a.ca)
            .then_with(|| {
                b.instability
                    .partial_cmp(&a.instability)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.path.cmp(&b.path))
    });
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
