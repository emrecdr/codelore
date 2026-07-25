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
//!
//! When an active calibration artifact ([`crate::calibration::load_active_artifact`])
//! carries a `repo_metrics` section (corpus pools populated by `codelore
//! calibrate`), three additional rows report where this repo's
//! `propagation_cost` and `cycle_file_share` sit against that corpus — see
//! [`run_architecture_metrics`]. Absent artifact or absent section ⇒ those
//! rows are simply not emitted (the additivity contract this module's tests
//! pin).

use crate::analyses::import_graph::{build_import_graph, graph_metrics};
use crate::calibration::{load_active_artifact, raw_percentile};
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
/// repo-level metric, in a fixed presentation order, plus an
/// `import_resolution_rate` disclosure row. Returns just that row when the
/// repo has import statements but none resolve (`n == 0`); empty only when
/// the repo has no imports at all.
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

    // Import resolution coverage: the fraction of import statements whose
    // target resolved to an in-repo file. External / stdlib imports point
    // outside the repo and count as unresolved, so a repo with many
    // third-party deps naturally reads lower — this discloses how much of the
    // import surface the dependency graph below actually covers, NOT a defect
    // score. Emitted whenever the repo has any imports (even when none
    // resolved, `n == 0` — precisely the sparse-graph case worth surfacing);
    // absent only when the repo has no import statements at all.
    let resolution_row = import_resolution_row(db)?;

    if m.n == 0 {
        return Ok(resolution_row.into_iter().collect());
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

    let mut rows = vec![
        row("propagation_cost", format!("{:.4}", m.propagation_cost)),
        row("acd", format!("{acd:.2}")),
        row("nccd", format!("{nccd:.2}")),
        row("dependency_cycles", m.cycle_count.to_string()),
        row("largest_cycle", m.largest_cycle.to_string()),
        row("files", m.n.to_string()),
        row("architecture_type", arch_type.to_owned()),
    ];
    if let Some(r) = resolution_row {
        rows.push(r);
    }

    // Corpus-relative percentiles (additive; against the `repo_metrics` pools
    // populated by `codelore calibrate`).
    // Reuses `m` — the SAME `GraphMetrics` the seven rows above were built
    // from — so this never rebuilds the import graph. `cycle_file_share`
    // mirrors `codelore calibrate`'s `pool_repo_metrics` formula exactly
    // (`cyclic_nodes / n`, `n` already guaranteed non-zero by the early
    // return above) so this repo's value is directly comparable to the pool.
    if let Some(artifact) = load_active_artifact(opts)?
        && let Some(repo_metrics) = artifact.repo_metrics.as_ref()
    {
        let cycle_file_share = cyclic_f / n_f;
        // `corpus_n` documents the sample size behind whichever percentile
        // row(s) were actually emitted: the `propagation_cost` pool's length
        // when that row is present (the common case — both metrics are
        // pooled together by `calibrate`), else the `cycle_file_share`
        // pool's length when only that one is.
        let mut corpus_n: Option<usize> = None;

        if let Some(pool) = repo_metrics.values.get("propagation_cost")
            && !pool.is_empty()
            && let Some(p) = raw_percentile(pool, m.propagation_cost)
        {
            rows.push(row("corpus_percentile:propagation_cost", format!("{p:.2}")));
            corpus_n = Some(pool.len());
        }
        if let Some(pool) = repo_metrics.values.get("cycle_file_share")
            && !pool.is_empty()
            && let Some(p) = raw_percentile(pool, cycle_file_share)
        {
            rows.push(row("corpus_percentile:cycle_file_share", format!("{p:.2}")));
            corpus_n.get_or_insert(pool.len());
        }
        if let Some(n) = corpus_n {
            rows.push(row("corpus_n", n.to_string()));
        }
    }

    Ok(rows)
}

/// The `import_resolution_rate` disclosure row: resolved imports / total
/// imports over the `imports` table, as a `{:.4}` fraction. `None` when the
/// repo has no imports at all (the rate is undefined). Query-time over the
/// already-ingested `imports` fact — no graph rebuild.
fn import_resolution_row(db: &FactsDb) -> Result<Option<ArchitectureMetricRow>> {
    let counts: Vec<(f64, i64)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT \
           COALESCE(COUNT(*) FILTER (WHERE target_path IS NOT NULL) * 1.0 \
                    / NULLIF(COUNT(*), 0), 0.0), \
           COUNT(*) \
         FROM imports",
        [],
        "import-resolution-rate",
        |r| Ok((r.get::<_, f64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    Ok(counts.first().and_then(|&(rate, total)| {
        (total > 0).then(|| row("import_resolution_rate", format!("{rate:.4}")))
    }))
}

fn row(metric: &str, value: String) -> ArchitectureMetricRow {
    ArchitectureMetricRow {
        metric: metric.to_owned(),
        value,
    }
}
