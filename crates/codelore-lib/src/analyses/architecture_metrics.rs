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

use crate::analyses::import_graph::{build_import_graph, graph_metrics};
use crate::facts::FactsDb;
use crate::{Options, Result};

/// One repo-level architecture metric: `(metric, value)`. The value is a
/// string so the numeric metrics and the textual `architecture_type`
/// label can share one row shape; numeric values are written as bare,
/// parseable numbers (e.g. `0.0607`) so downstream tooling can read them.
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
    let m = graph_metrics(&graph);
    if m.n == 0 {
        return Ok(Vec::new());
    }

    let n_f = f64::from(u32::try_from(m.n).unwrap_or(u32::MAX));
    // ACD/NCCD layer on top of the shared kernel's CCD + propagation cost.
    let acd = m.ccd / n_f;
    // CCD of a balanced binary tree of n nodes = (n+1)·log2(n+1) − n.
    let ccd_btree = (n_f + 1.0) * (n_f + 1.0).log2() - n_f;
    let nccd = if ccd_btree > 0.0 {
        m.ccd / ccd_btree
    } else {
        0.0
    };

    let largest_f = f64::from(m.largest_cycle);
    let cyclic_f = f64::from(m.cyclic_nodes);
    let arch_type = if m.cycle_count == 0 {
        "hierarchical"
    } else if cyclic_f > 0.0 && largest_f / cyclic_f >= CORE_DOMINANCE {
        "core-periphery"
    } else {
        "multi-core"
    };

    Ok(vec![
        row("propagation_cost", format!("{:.4}", m.propagation_cost)),
        row("acd", format!("{acd:.2}")),
        row("nccd", format!("{nccd:.2}")),
        row("dependency_cycles", m.cycle_count.to_string()),
        row("largest_cycle", m.largest_cycle.to_string()),
        row("files", m.n.to_string()),
        row("architecture_type", arch_type.to_owned()),
    ])
}

fn row(metric: &str, value: String) -> ArchitectureMetricRow {
    ArchitectureMetricRow {
        metric: metric.to_owned(),
        value,
    }
}
