//! `centrality` — graph-degree centrality on the behavioral coupling graph.
//!
//! Each Fisher-significant pair from [`coupling::run_coupling`] contributes
//! an undirected edge between its endpoints. For each path that appears in
//! at least one such edge, this analysis reports:
//!
//! - `degree`: count of Fisher-significant partners. Equivalent to the
//!   internal `coupling_centrality_v1` value that has historically backed
//!   the `code-health` composite score's `n_cp` term.
//! - `weighted_degree`: sum of `-log10(fisher_p)` across all partners.
//!   Pairs with stronger statistical evidence (smaller `fisher_p`)
//!   contribute more, so a file with three rock-solid partners ranks
//!   above a file with three borderline-significant ones. Modelled on
//!   Barrat et al., *The architecture of complex weighted networks*,
//!   PNAS 2004 §III.A (vertex strength in weighted networks).
//! - `revs`: total commits touching this path. Supplied as denominator
//!   context so a hyper-coupled but rarely-touched file isn't visually
//!   indistinguishable from a long-lived hub.
//!
//! ## Why no in-degree / out-degree
//!
//! The behavioral coupling graph is **undirected**. The Fisher exact test
//! on a 2×2 contingency table is symmetric, so each pair contributes a
//! single edge between two endpoints with no notion of direction.
//! Surfacing identical in/out columns would mislead. A directional
//! change-influence graph (e.g. "A's edits precede B's within commit
//! windows") would be a separate analysis — out of scope here.
//!
//! Research basis: see `docs/research-foundations.md` entry "centrality"
//! (Newman 2010 §7.1 for degree centrality on undirected graphs; Barrat
//! et al. 2004 for the weighted-degree formulation).

use std::collections::HashMap;

use crate::analyses::coupling::run_coupling;
use crate::facts::FactsDb;
use crate::{Options, Result};

/// A single per-file centrality row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CentralityRow {
    /// Canonical file path (rewritten through the lineage CTE when
    /// `--use-canonical-lineage` is on, raw `changes.path` otherwise).
    pub entity: String,
    /// Count of Fisher-significant coupling partners.
    pub degree: u32,
    /// Sum of `-log10(fisher_p)` across all Fisher-significant partners.
    /// Higher = the file is coupled with more pairs that carry stronger
    /// statistical evidence.
    pub weighted_degree: f64,
    /// Total commits touching this path. Same value regardless of which
    /// endpoint of a pair the path is, so accumulator overwrites are
    /// idempotent — see in-function note for the invariant.
    pub revs: u32,
}

pub fn run_centrality(db: &FactsDb, opts: &Options) -> Result<Vec<CentralityRow>> {
    // Per-path accumulator. `coupling.rs` computes `revs_a` / `revs_b`
    // via `COUNT(*) GROUP BY path`, so per path the value is identical
    // across every pair the path appears in. Overwriting `revs` on
    // every encounter is therefore idempotent rather than a race.
    //
    // Declared at the top of the function body so the items-after-
    // statements clippy pedantic lint doesn't fire (items must come
    // before any `let` statements in the same scope).
    struct Acc {
        degree: u32,
        weighted: f64,
        revs: u32,
    }

    // `--rows N` MUST NOT propagate into the inner coupling query — we
    // need every Fisher-significant pair to compute degree honestly.
    // Truncation is applied after aggregation. Same rationale as
    // `code_health::materialize_centrality` (which this analysis
    // generalises). See `Options::with_no_row_limit` for the prior
    // bug narrative.
    let pairs = run_coupling(db, &opts.with_no_row_limit())?;

    let mut acc: HashMap<String, Acc> = HashMap::new();
    for p in &pairs {
        // Defensive: the `fishers_exact` crate can in theory return a
        // p-value of exactly 0.0 on a degenerate 2×2 contingency table.
        // `log10(0)` is `-inf` and would propagate through `weighted_degree`
        // as a `-inf` accumulator (and a downstream NaN once any partner
        // contributes a finite weight). Clamp to `-log10(f64::MIN_POSITIVE)
        // ≈ 307.6` instead so the metric stays finite + ranks the
        // pathological pair as the maximum-strength contributor it
        // morally is.
        let weight = if p.fisher_p > 0.0 {
            -p.fisher_p.log10()
        } else {
            -f64::MIN_POSITIVE.log10()
        };

        let entry = acc.entry(p.entity_a.clone()).or_insert(Acc {
            degree: 0,
            weighted: 0.0,
            revs: p.revs_a,
        });
        entry.degree += 1;
        entry.weighted += weight;
        entry.revs = p.revs_a;

        let entry = acc.entry(p.entity_b.clone()).or_insert(Acc {
            degree: 0,
            weighted: 0.0,
            revs: p.revs_b,
        });
        entry.degree += 1;
        entry.weighted += weight;
        entry.revs = p.revs_b;
    }

    let mut rows: Vec<CentralityRow> = acc
        .into_iter()
        .map(|(entity, a)| CentralityRow {
            entity,
            degree: a.degree,
            weighted_degree: a.weighted,
            revs: a.revs,
        })
        .collect();

    // Stable order: degree desc, weighted desc, entity asc. The lex
    // tiebreaker keeps output byte-identical across runs even when
    // HashMap iteration order varies.
    rows.sort_by(|a, b| {
        b.degree.cmp(&a.degree).then_with(|| {
            b.weighted_degree
                .partial_cmp(&a.weighted_degree)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.entity.cmp(&b.entity))
        })
    });

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }

    Ok(rows)
}
