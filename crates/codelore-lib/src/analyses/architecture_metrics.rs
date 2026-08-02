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
//! Four further rows disclose how much of the import surface the graph
//! above actually covers — *coverage* transparency, not defect scores, so
//! that a sparse graph can't read as a clean one. The structural metrics
//! only ever see the resolved edges (`target_path IS NOT NULL`); these
//! query the full `imports` table so a poor resolution rate is visible:
//!
//! - **`import_resolution_rate`** — fraction of all import statements whose
//!   target resolved to an in-repo file. External and standard-library
//!   imports (`numpy`, `java.util`, `std::fmt`, …) legitimately point
//!   outside the repo and count as unresolved, so a repo with many
//!   third-party dependencies naturally scores lower — this is expected,
//!   not a resolver bug.
//! - **`first_party_import_share`** — fraction of import statements that are
//!   first-party by intent: either already resolved, or a *relative* import
//!   (`use crate::…`, `from .mod import …`, `./foo`) naming an in-repo path
//!   even when the resolver missed it — including a first-party *glob*
//!   (`use crate::foo::*;`, bare `use super::*;`), which `imports.kind`
//!   tags `wildcard` rather than `relative` (see the "Definition caveat"
//!   below) but which is still unambiguously in-repo by its
//!   `crate::`/`self::`/`super::` prefix.
//! - **`resolution_rate_first_party`** — of those first-party imports, the
//!   fraction that resolved. It drops external imports from the denominator,
//!   isolating resolver coverage from third-party-dependency density: a low
//!   value points at a genuine resolver gap rather than many external deps.
//! - **`wildcard_import_share`** — fraction of all import statements that
//!   are glob imports (`imports.kind = 'wildcard'`), first-party or not.
//!   Purely informational (a glob names a module, not a symbol).
//!
//! When an active calibration artifact ([`crate::calibration::load_active_artifact`])
//! carries a `repo_metrics` section (corpus pools populated by `codelore
//! calibrate`), additional rows report where this repo's `propagation_cost`
//! and `cycle_file_share` sit against that corpus, each paired with a Wilson
//! 95% confidence interval (`…:ci_low` / `…:ci_high`) that reflects the finite
//! corpus pool's sampling uncertainty — see [`run_architecture_metrics`].
//! Absent artifact or absent section ⇒ those rows are simply not emitted (the
//! additivity contract this module's tests pin).

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
/// repo-level metric, in a fixed presentation order, plus the
/// import-resolution disclosure rows (see [`import_resolution_rows`]).
/// Returns just those disclosure rows when the repo has import statements
/// but none resolve into the graph (`n == 0`); empty only when the repo
/// has no imports at all.
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

    // Import-resolution disclosure: coverage of the import surface, split
    // three ways so a low headline rate can't be misread as resolver
    // weakness. See [`import_resolution_rows`]. Emitted whenever the repo has
    // any imports (even when none resolved, `n == 0` — precisely the
    // sparse-graph case worth surfacing); absent only when the repo has no
    // import statements at all.
    let resolution_rows = import_resolution_rows(db)?;

    if m.n == 0 {
        return Ok(resolution_rows);
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
    rows.extend(resolution_rows);

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
            let (lo, hi) = crate::stats::wilson_ci_from_proportion(p, pool_sample_size(pool));
            rows.push(row(
                "corpus_percentile:propagation_cost:ci_low",
                format!("{lo:.2}"),
            ));
            rows.push(row(
                "corpus_percentile:propagation_cost:ci_high",
                format!("{hi:.2}"),
            ));
            corpus_n = Some(pool.len());
        }
        if let Some(pool) = repo_metrics.values.get("cycle_file_share")
            && !pool.is_empty()
            && let Some(p) = raw_percentile(pool, cycle_file_share)
        {
            rows.push(row("corpus_percentile:cycle_file_share", format!("{p:.2}")));
            let (lo, hi) = crate::stats::wilson_ci_from_proportion(p, pool_sample_size(pool));
            rows.push(row(
                "corpus_percentile:cycle_file_share:ci_low",
                format!("{lo:.2}"),
            ));
            rows.push(row(
                "corpus_percentile:cycle_file_share:ci_high",
                format!("{hi:.2}"),
            ));
            corpus_n.get_or_insert(pool.len());
        }
        if let Some(n) = corpus_n {
            rows.push(row("corpus_n", n.to_string()));
        }
    }

    Ok(rows)
}

/// The import-resolution disclosure rows, over the already-ingested
/// `imports` fact (query-time, no graph rebuild). Empty when the repo has no
/// imports at all; otherwise up to three `{:.4}`-fraction rows that split one
/// coarse number into a resolver-strength signal and a repo-composition one,
/// so a low headline rate can't be misread as a weak resolver:
///
/// - **`import_resolution_rate`** — resolved ÷ *all* imports. Unchanged
///   semantics (additive-only contract). External crates / stdlib / npm
///   imports legitimately point outside the repo and stay unresolved, so a
///   repo with many third-party deps naturally reads low: this is *coverage
///   of the whole import surface*, NOT a defect score.
/// - **`first_party_import_share`** — first-party candidates ÷ all imports.
///   A *first-party candidate* is an import that could target in-repo code:
///   it either resolved to a tracked file, OR is syntactically repo-relative
///   (`imports.kind = 'relative'` — Rust `crate::`/`self::`/`super::`, Python
///   leading-dot, JS/TS `./`|`../`), the exact imports each per-language
///   resolver *attempts* to resolve in-repo (see `imports::resolver`). An
///   unresolved *absolute* import is presumed external. This is how much of
///   the surface even aims at the repo.
/// - **`resolution_rate_first_party`** — resolved ÷ first-party candidates:
///   the resolver's strength on the imports that actually point in-repo,
///   isolated from the third-party mix that drags the headline rate down.
///   Omitted when there are no first-party candidates (the rate is undefined,
///   and a `0.00` would misread as "resolved none of them").
/// - **`wildcard_import_share`** — glob imports (`kind = 'wildcard'`: Rust
///   `use foo::*`, Java `import foo.*;`, Python `from foo import *`) ÷ all
///   imports. Purely informational — a glob names a module, not a symbol, so
///   it is inherently harder for the resolver to check against an in-repo
///   path than a named import.
///
/// Definition caveat (documented, not hidden): an absolute import that fails
/// to resolve is counted as external, so a genuinely first-party absolute
/// import the resolver *missed* is under-counted; and for a language with no
/// syntactic relative marker (Java), first-party candidates reduce to the
/// resolved imports, making `resolution_rate_first_party` optimistic there.
/// A second, narrower instance of the same undercount: `classify` tests for a
/// trailing glob (`ends_with('*')`) BEFORE it tests for a relative root, so a
/// first-party glob (`use crate::foo::*;`, bare `use super::*;`) is tagged
/// `Wildcard`, not `Relative` — `imports.kind` keeps that distinction (Java's
/// `import foo.*;` has no relative form to fall back to, so blending the two
/// kinds would erase real signal there). `first_party_import_share` instead
/// widens its first-party predicate to recognise a `Wildcard` row whose raw
/// `target` carries the same `crate::`/`self::`/`super::` prefix `classify`
/// itself uses for Rust relative imports, counting it as first-party without
/// touching the stored `kind`.
fn import_resolution_rows(db: &FactsDb) -> Result<Vec<ArchitectureMetricRow>> {
    // One pass over `imports`. The `rate` expression is kept verbatim from the
    // original single-row query so `import_resolution_rate` stays byte-for-byte
    // identical; the other rows derive from the raw counts alongside it. The
    // first-party `COUNT(*) FILTER` widens the plain `kind = 'relative'` test
    // with a first-party-glob carve-out: a `Wildcard`-kind row whose `target`
    // starts with the same `crate::`/`self::`/`super::` prefixes `classify`
    // (`imports/extractor.rs`) uses to detect a Rust relative import. Those
    // prefixes are unambiguously in-repo regardless of the trailing `::*`, so
    // an unresolved first-party glob (e.g. bare `use super::*;`, which the
    // resolver can't map to a single file) no longer silently falls out of
    // the first-party count the way it did when only `kind = 'relative'` was
    // checked. Java's `import foo.*;` never matches this prefix set (Java has
    // no syntactic relative-import marker), so it stays correctly excluded.
    let counts: Vec<(f64, i64, i64, i64, i64)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT \
           COALESCE(COUNT(*) FILTER (WHERE target_path IS NOT NULL) * 1.0 \
                    / NULLIF(COUNT(*), 0), 0.0), \
           COUNT(*) FILTER (WHERE target_path IS NOT NULL), \
           COUNT(*) FILTER (WHERE target_path IS NOT NULL \
                              OR kind = 'relative' \
                              OR (kind = 'wildcard' AND ( \
                                     target LIKE 'crate::%' \
                                     OR target LIKE 'self::%' \
                                     OR target LIKE 'super::%' \
                                  ))), \
           COUNT(*), \
           COUNT(*) FILTER (WHERE kind = 'wildcard') \
         FROM imports",
        [],
        "import-resolution-rate",
        |r| {
            Ok((
                r.get::<_, f64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        },
    )?;
    let Some(&(rate, resolved, first_party, total, wildcard)) = counts.first() else {
        return Ok(Vec::new());
    };
    if total == 0 {
        return Ok(Vec::new());
    }
    // Counts are small non-negative import tallies; the `u32::try_from … f64`
    // idiom (as used for `n_f` above) keeps the division lossless and away
    // from the precision-loss lint without an `unwrap`.
    let as_f64 = |n: i64| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    let (resolved_f, first_party_f, total_f, wildcard_f) = (
        as_f64(resolved),
        as_f64(first_party),
        as_f64(total),
        as_f64(wildcard),
    );
    let mut rows = vec![
        row("import_resolution_rate", format!("{rate:.4}")),
        row(
            "first_party_import_share",
            format!("{:.4}", first_party_f / total_f),
        ),
    ];
    if first_party > 0 {
        rows.push(row(
            "resolution_rate_first_party",
            format!("{:.4}", resolved_f / first_party_f),
        ));
    }
    rows.push(row(
        "wildcard_import_share",
        format!("{:.4}", wildcard_f / total_f),
    ));
    Ok(rows)
}

/// The `n` for a repo-level metric's Wilson interval: the count of corpus repos
/// contributing to this pool. Saturates on the impossible >4-billion case
/// rather than wrapping.
fn pool_sample_size(pool: &[f64]) -> u32 {
    u32::try_from(pool.len()).unwrap_or(u32::MAX)
}

fn row(metric: &str, value: String) -> ArchitectureMetricRow {
    ArchitectureMetricRow {
        metric: metric.to_owned(),
        value,
    }
}
