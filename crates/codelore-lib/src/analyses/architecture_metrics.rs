//! `architecture-metrics` analysis — repo-level structural-health
//! numbers over the resolved import graph, the kind you trend over time.
//!
//! - **`propagation_cost`** (`MacCormack`, Rusnak & Baldwin 2006) — the
//!   density of the visibility (transitive-closure) matrix: "a change to
//!   a random file can, on average, reach this fraction of the system".
//! - **`acd`** — Lakos's Average Component Dependency: the mean number of
//!   files each file depends on directly *or transitively* (incl. self).
//! - **`nccd`** — Normalised Cumulative Component Dependency: `CCD`
//!   divided by the `CCD` of a balanced binary tree of the same size.
//!   `< 1` ≈ horizontal/flat, `> 1` ≈ vertical/layered, `> 2` ≈ likely
//!   cyclic (Lakos 1996, *Large-Scale C++ Software Design*).
//! - **`dependency_cycles`** / **`largest_cycle`** — count of non-trivial
//!   SCCs and the size of the biggest tangle.
//! - **`architecture_type`** — `hierarchical` (acyclic), `core-periphery`
//!   (one dominant cyclic group), or `multi-core` (several comparable
//!   ones) — Baldwin, `MacCormack` & Rusnak 2014.
//!
//! All derived in one pass from the shared import-graph kernel (SCC +
//! reachability), so this adds no new query cost beyond building the
//! graph. Accuracy follows the import resolver's language coverage.

use crate::analyses::import_graph::{build_import_graph, reachability, tarjan_scc};
use crate::facts::FactsDb;
use crate::{Options, Result};

/// One repo-level architecture metric: `(metric, value)`. The value is a
/// string so numeric metrics and the `architecture_type` label share one
/// row shape (mirrors [`SummaryRow`](crate::analyses::summary::SummaryRow));
/// numeric values are bare numbers so downstream tooling can parse them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchitectureMetricRow {
    pub metric: String,
    pub value: String,
}

/// Share of a cyclic node set the largest cycle must cover to call the
/// architecture "core-periphery" rather than "multi-core".
const CORE_DOMINANCE: f64 = 0.6;

/// Run the `architecture-metrics` analysis. Returns one row per
/// repo-level metric, in a fixed presentation order. Empty when no
/// imports resolve.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the import-graph build).
#[tracing::instrument(name = "architecture-metrics", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_architecture_metrics(
    db: &FactsDb,
    opts: &Options,
) -> Result<Vec<ArchitectureMetricRow>> {
    let graph = build_import_graph(db)?;
    let n = graph.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    let sccs = tarjan_scc(&graph.adj);
    let reach = reachability(&graph.adj, &sccs);

    let n_f = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    // Cumulative Component Dependency = Σ visibility-fan-out (each file's
    // transitive dependency set incl. self).
    let ccd: f64 = reach.vfo.iter().map(|&v| f64::from(v)).sum();
    let acd = ccd / n_f;
    let propagation_cost = ccd / (n_f * n_f);
    // CCD of a balanced binary tree of n nodes = (n+1)·log2(n+1) − n.
    let ccd_btree = (n_f + 1.0) * (n_f + 1.0).log2() - n_f;
    let nccd = if ccd_btree > 0.0 {
        ccd / ccd_btree
    } else {
        0.0
    };

    // Cycle structure.
    let mut cycle_count = 0u32;
    let mut largest_cycle = 0usize;
    let mut cyclic_nodes = 0usize;
    for comp in &sccs {
        if comp.len() >= 2 {
            cycle_count += 1;
            cyclic_nodes += comp.len();
            largest_cycle = largest_cycle.max(comp.len());
        }
    }
    let largest_f = f64::from(u32::try_from(largest_cycle).unwrap_or(u32::MAX));
    let cyclic_f = f64::from(u32::try_from(cyclic_nodes).unwrap_or(u32::MAX));
    let arch_type = if cycle_count == 0 {
        "hierarchical"
    } else if cyclic_f > 0.0 && largest_f / cyclic_f >= CORE_DOMINANCE {
        "core-periphery"
    } else {
        "multi-core"
    };

    Ok(vec![
        row("propagation_cost", format!("{propagation_cost:.4}")),
        row("acd", format!("{acd:.2}")),
        row("nccd", format!("{nccd:.2}")),
        row("dependency_cycles", cycle_count.to_string()),
        row("largest_cycle", largest_cycle.to_string()),
        row("files", n.to_string()),
        row("architecture_type", arch_type.to_owned()),
    ])
}

fn row(metric: &str, value: String) -> ArchitectureMetricRow {
    ArchitectureMetricRow {
        metric: metric.to_owned(),
        value,
    }
}
