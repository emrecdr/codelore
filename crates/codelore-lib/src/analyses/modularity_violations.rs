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
//! - **Structural:** the `imports` table (resolved file → file edges).
//!
//! A modularity violation is a temporal edge with no structural edge
//! in EITHER direction — the "co-change ∧ ¬import" cell of the
//! structure×history matrix that neither an import-only graph (it has
//! no history) nor a history-only tool (it has no structure) can
//! populate.
//!
//! ## Scope & limits
//!
//! - **Direct edges only.** A pair coupled solely through a transitive
//!   import chain (`a → b → c`, with `a` and `c` co-changing) is still
//!   reported here. Transitive-reachability filtering is a follow-up
//!   that consumes the structural reachability kernel.
//! - **Resolver language coverage.** "No structural edge" relies on the
//!   resolved `imports.target_path`, populated for Rust + Python + JS/TS.
//!   Languages whose resolver leaves `target_path` NULL (e.g. Java) make
//!   real import edges look absent, so such repos over-report. Same
//!   caveat [`god_classes`](crate::analyses::god_classes) documents for
//!   fan-in.

use std::collections::HashSet;

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

    // Resolved structural import edges, as a directed set. We probe both
    // orientations below, so a single orientation in the set is enough.
    let import_edges: HashSet<(String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT src_path, target_path FROM imports WHERE target_path IS NOT NULL",
        [],
        "modularity-violations imports",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?
    .into_iter()
    .collect();

    // Keep co-change pairs with no import edge in either direction.
    // `run_coupling` already orders by `(degree DESC, average_revs DESC,
    // entity_a, entity_b)`; the filter is stable, so the output stays
    // ranked by coupling strength. Apply the final row limit last.
    let mut out: Vec<ModularityViolationRow> = coupling_rows
        .into_iter()
        .filter(|p| {
            !import_edges.contains(&(p.entity_a.clone(), p.entity_b.clone()))
                && !import_edges.contains(&(p.entity_b.clone(), p.entity_a.clone()))
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
