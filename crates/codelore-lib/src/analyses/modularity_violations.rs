//! `modularity-violations` analysis — file pairs that co-change
//! (Fisher-significant) yet have NO structural import edge between
//! them.
//!
//! This is the "implicit cross-module dependency" of Kazman & Cai's
//! DV8 hotspot patterns (Mo, Cai, Kazman, Xiao 2015 *Hotspot
//! Patterns*): two files that change together but don't import each
//! other are coupled through something invisible — a shared global, a
//! leaky abstraction, a contract honoured through a third party.
//! Empirically these pairs are more bug- and change-prone than
//! structurally-coupled ones.
//!
//! ## Fusion — the two graphs `CodeLore` already builds
//!
//! - **Temporal:** [`coupling::run_coupling`](crate::analyses::coupling::run_coupling)
//!   Fisher-significant co-change pairs.
//! - **Structural:** the import graph
//!   ([`import_graph`](crate::analyses::import_graph)) with transitive
//!   reachability.
//!
//! A modularity violation is a co-change pair with **no directed
//! dependency path** between the two files in either direction — neither
//! (transitively) imports the other. That is the "co-change ∧
//! ¬structurally-connected" cell of the structure×history matrix that
//! neither an import-only graph (it has no history) nor a history-only
//! tool (it has no structure) can populate.
//!
//! ## Scope & limits
//!
//! - **Transitive.** A pair coupled through an import chain
//!   (`a → b → c`, with `a` and `c` co-changing) is *not* a violation —
//!   `a` does depend on `c`, just not directly. Only pairs with no path
//!   between them are flagged. Files with no resolved imports aren't
//!   graph nodes, so any co-change with them is (correctly) a violation.
//! - **Resolver language coverage.** Connectivity relies on the resolved
//!   `imports.target_path`, populated for Rust + Python + JS/TS.
//!   Languages whose resolver leaves `target_path` NULL (e.g. Java) make
//!   real import edges look absent, so such repos over-report. Same
//!   caveat [`god_classes`](crate::analyses::god_classes) documents for
//!   fan-in.

use crate::facts::FactsDb;
use crate::{Options, Result};

/// A single modularity violation: a Fisher-significant co-change pair
/// with no structural import edge. Carries the co-change evidence so
/// callers can rank and explain the finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModularityViolationRow {
    /// Lexicographically smaller path of the pair (inherited from the
    /// coupling row's canonical ordering).
    pub entity_a: String,
    /// Lexicographically larger path of the pair.
    pub entity_b: String,
    /// Commits in which both files changed together.
    pub shared: u32,
    /// Coupling degree `100.0 * shared / average_revs` — the temporal
    /// coupling strength carried straight from the coupling pair.
    pub degree: f64,
    /// Two-tailed Fisher exact p-value for the co-change. Lower = the
    /// implicit coupling is less likely to be coincidence.
    pub fisher_p: f64,
}

/// Run the `modularity-violations` analysis. Returns the
/// Fisher-significant co-change pairs that have no structural import
/// edge, ranked by coupling degree (highest first — inherited from
/// `run_coupling`'s ordering).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the inner coupling run or the imports scan).
#[tracing::instrument(name = "modularity-violations", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_modularity_violations(
    db: &FactsDb,
    opts: &Options,
) -> Result<Vec<ModularityViolationRow>> {
    // Fisher-significant co-change pairs. Strip the row limit so the
    // anti-join sees the FULL candidate pool — `--rows N` caps the
    // final violations the user sees, not the coupling input (mirrors
    // `clone_coupling`'s reasoning). `run_coupling` is memoized, so a
    // sibling analysis having already run it in the same dispatch
    // makes this call free.
    let coupling_rows = crate::analyses::coupling::run_coupling(db, &opts.with_no_row_limit())?;
    if coupling_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Structural import graph + transitive reachability index. A
    // co-change pair is a violation unless a directed dependency path
    // connects the two files (either direction) — i.e. one transitively
    // imports the other.
    let graph = crate::analyses::import_graph::build_import_graph(db)?;
    let sccs = crate::analyses::import_graph::tarjan_scc(&graph.adj);
    let reach = crate::analyses::import_graph::reach_index(&graph.adj, &sccs);

    // Keep co-change pairs that are not structurally connected.
    // `run_coupling` already orders by `(degree DESC, average_revs DESC,
    // entity_a, entity_b)`; the filter is stable, so the output stays
    // ranked by coupling strength. Apply the final row limit last.
    let mut out: Vec<ModularityViolationRow> = coupling_rows
        .into_iter()
        .filter(|p| {
            // Both files must be import-graph nodes AND have a path
            // between them to count as structurally connected. A file
            // with no resolved imports isn't a node, so a co-change with
            // it is correctly kept as a violation.
            match (
                graph.path_to_id.get(&p.entity_a),
                graph.path_to_id.get(&p.entity_b),
            ) {
                (Some(&a), Some(&b)) => !reach.connected(a, b),
                _ => true,
            }
        })
        .map(|p| ModularityViolationRow {
            entity_a: p.entity_a,
            entity_b: p.entity_b,
            shared: p.shared,
            degree: p.degree,
            fisher_p: p.fisher_p,
        })
        .collect();

    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
