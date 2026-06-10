//! Clone-coupling intersection (Plan 8 §6). The strategic differentiator
//! vs every existing clone detector: only flag clones that ALSO change
//! together at Fisher-significant rates. `CodeScene` calls this "X-Ray"; we
//! ship the same analytical pattern with our published-formula transparency.
//!
//! Algorithm (locked from the Plan 8 research brief):
//!
//! 1. **Any-pair** intersection. A clone family is "live" if **any two
//!    members** of the same `clone_group_id` are themselves a
//!    coupling-significant file pair (Fisher exact p < threshold).
//! 2. **False-positive mitigations** (5 per research brief):
//!     - min fragment size: `node_count ≥ opts.min_clone_node_count` (default 30)
//!     - min shared revs: `shared_revs ≥ opts.min_clone_shared_revs` (default 3)
//!     - similarity floor: `similarity ≥ opts.clone_similarity_floor` (default 0.70)
//!     - exclude generated/vendored paths (handled by `--exclude` + `.codeloreignore`)
//!     - optional same-dir skip (`opts.clone_skip_same_dir`, default true).
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "clone-coupling" (Tornhill, *Software Design X-Rays*, 2018 —
//! X-Ray analysis; `CodeScene` productisation). `CodeLore` ships the
//! same analytical pattern with published-formula transparency.
//!
//! NOTE: This analysis assumes Plan 8 §4 (`extract_clones_at_head` populates
//! the `clones` table during `FactsDb::ingest`) has shipped. Before §4 lands,
//! the JOIN returns 0 rows because the `clones` table stays empty even though
//! the `--analysis clones` CLI path (which runs ad-hoc) produces rows.
//! Tests in `tests/clone_coupling_test.rs` populate the `clones` table
//! programmatically to validate the SQL independently.

use serde::Serialize;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// One clone-coupling finding. A pair of files from the same clone family
/// that also co-change at Fisher-significant rates. Sorted by `combined_score`
/// desc so PR-review tooling surfaces the most actionable findings first.
#[derive(Debug, Clone, Serialize)]
pub struct CloneCouplingRow {
    /// `clone_group_id` shared by `file_a` and `file_b`.
    pub clone_group_id: u32,
    /// AST structural digest (hex) of the shared clone fingerprint.
    pub fingerprint: String,
    pub file_a: String,
    pub file_b: String,
    /// Function name in `file_a` (may be empty for closures / anonymous fns).
    pub entity_a: String,
    pub entity_b: String,
    pub start_line_a: u32,
    pub end_line_a: u32,
    pub start_line_b: u32,
    pub end_line_b: u32,
    /// Number of AST nodes in each member's fingerprint (shared by both —
    /// they're in the same clone family).
    pub node_count: u32,
    /// 1.0 for T1+T2 exact matches; < 1.0 for Type-3 near-miss (Plan 7 §2 T4).
    pub similarity: f64,
    /// Commits where both files co-changed.
    pub shared_revs: u32,
    /// Total commits touching `file_a`.
    pub support_a: u32,
    /// Total commits touching `file_b`.
    pub support_b: u32,
    /// `shared_revs / max(support_a, support_b)` ∈ [0, 1].
    pub degree_pct: f64,
    /// Fisher exact test p-value. Lower = more confident.
    pub p_value: f64,
    /// `similarity × degree_pct × (1 − p_value)` ∈ [0, 1]. Ranks the row;
    /// SARIF `security-severity` is derived from this × 10.
    pub combined_score: f64,
    /// T9: `true` when either `file_a` or `file_b` is currently a
    /// knowledge-island file (departed primary author + no substantial
    /// other owners). Live clones AT-RISK files are the most actionable
    /// debt findings: the clone is already a refactoring liability and
    /// the people who could refactor it have already left.
    ///
    /// Computed by intersecting `clone-coupling` output with the
    /// `knowledge-islands` analysis result set. Honors the same
    /// `--departed-threshold-days` and `--age-time-now` flags as the
    /// standalone analysis. No `knowledge-islands` data → all rows have
    /// `at_risk = false` (graceful degradation when the repo has no
    /// departed contributors).
    pub at_risk: bool,
}

/// Run the clone-coupling analysis. Returns rows sorted by
/// `combined_score` descending.
///
/// Performance: the JOIN is `O(n × k²)` where `n` is the number of clone
/// families and `k` is the average family size (typically ≤ 10). With a
/// HashMap-based probe table built from `coupling` results, the inner loop
/// is `O(k²)` per family rather than `O(pairs²)` across the whole repo —
/// follows the `SourcererCC` index-then-probe pattern from the research brief.
#[allow(clippy::too_many_lines)]
pub fn run_clone_coupling(db: &FactsDb, opts: &Options) -> Result<Vec<CloneCouplingRow>> {
    use std::collections::HashMap;

    // Build the materialized coupling output once. The existing
    // `coupling::run_coupling` already applies the Fisher-significance filter
    // and the `max_changeset_size` pre-filter — we lean on that work and
    // just JOIN our clone-family pairs against the results.
    //
    // CRITICAL: strip `rows_limit` AND drop `min_shared_revs` to the
    // clone-coupling floor for the inner call.
    //
    // - `--rows N` is meant to cap the FINAL clone-coupling rows the user
    //   sees; if it propagated into `run_coupling`, the inner result would
    //   truncate to the top N global coupling pairs and we'd silently miss
    //   every clone whose partner sits outside that window.
    // - `--min-shared-revs` default (5) would silently drop clone pairs
    //   that co-changed exactly 3 or 4 times — even though
    //   `--min-clone-shared-revs` (default 3) explicitly allows them.
    //   F2 fix: `for_clone_coupling_inner_coupling` lowers the floor to
    //   the clone-coupling threshold so the candidate pool is correct
    //   BEFORE we filter by clone-coupling's own threshold below.
    //
    // See `Options::for_clone_coupling_inner_coupling`.
    let coupling_rows =
        crate::analyses::coupling::run_coupling(db, &opts.for_clone_coupling_inner_coupling())?;

    // Index coupling pairs by (file_a, file_b) — both orderings, since clone
    // pairs come from the `clones` self-join with `path_a < path_b` ordering
    // but coupling output uses its own ordering and we need to find both.
    let mut coupling_map: HashMap<(String, String), &crate::analyses::coupling::CouplingRow> =
        HashMap::with_capacity(coupling_rows.len() * 2);
    for row in &coupling_rows {
        coupling_map.insert((row.entity_a.clone(), row.entity_b.clone()), row);
        coupling_map.insert((row.entity_b.clone(), row.entity_a.clone()), row);
    }

    // Pull clone-family member pairs from the clones table.
    // For each family with ≥ 2 members, generate the self-join with
    // `c1.path < c2.path` so each pair appears once.
    #[allow(clippy::items_after_statements)]
    const CLONE_PAIRS_SQL: &str = "
        SELECT c1.clone_group_id,
               hex(c1.fingerprint) AS fingerprint,
               c1.path AS file_a,
               c2.path AS file_b,
               c1.function AS entity_a,
               c2.function AS entity_b,
               CAST(c1.start_line AS UINTEGER) AS start_line_a,
               CAST(c1.end_line AS UINTEGER) AS end_line_a,
               CAST(c2.start_line AS UINTEGER) AS start_line_b,
               CAST(c2.end_line AS UINTEGER) AS end_line_b,
               CAST(c1.node_count AS UINTEGER) AS node_count,
               c1.similarity
        FROM clones c1
        JOIN clones c2
          ON c1.clone_group_id = c2.clone_group_id
         AND c1.path < c2.path
        WHERE c1.node_count >= ?
          AND c1.similarity >= ?
    ";

    #[allow(clippy::items_after_statements)]
    struct ClonePair {
        clone_group_id: u32,
        fingerprint: String,
        file_a: String,
        file_b: String,
        entity_a: String,
        entity_b: String,
        start_line_a: u32,
        end_line_a: u32,
        start_line_b: u32,
        end_line_b: u32,
        node_count: u32,
        similarity: f64,
    }

    crate::analyses::query::explain_if_requested(db, CLONE_PAIRS_SQL, [], "clone-coupling", opts)?;
    let mut stmt = db
        .conn()
        .prepare(CLONE_PAIRS_SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("clone-coupling: prepare: {e}")))?;
    let pairs = stmt
        .query_map(
            duckdb::params![opts.min_clone_node_count, opts.clone_similarity_floor],
            |r| {
                Ok(ClonePair {
                    clone_group_id: r.get::<_, u32>(0)?,
                    fingerprint: r.get::<_, String>(1)?,
                    file_a: r.get::<_, String>(2)?,
                    file_b: r.get::<_, String>(3)?,
                    entity_a: r.get::<_, String>(4)?,
                    entity_b: r.get::<_, String>(5)?,
                    start_line_a: r.get::<_, u32>(6)?,
                    end_line_a: r.get::<_, u32>(7)?,
                    start_line_b: r.get::<_, u32>(8)?,
                    end_line_b: r.get::<_, u32>(9)?,
                    node_count: r.get::<_, u32>(10)?,
                    similarity: r.get::<_, f64>(11)?,
                })
            },
        )
        .map_err(|e| CodeLoreError::Analysis(format!("clone-coupling: query: {e}")))?;

    // Probe each clone pair against the coupling map; emit a CloneCouplingRow
    // only when both files appear together as a Fisher-significant pair AND
    // pass the per-pair filters.
    let mut rows: Vec<CloneCouplingRow> = Vec::new();
    for pair in pairs {
        let p = pair.map_err(|e| CodeLoreError::Analysis(format!("clone-coupling: row: {e}")))?;

        // Optional: skip same-directory clone pairs (intentional mirroring).
        if opts.clone_skip_same_dir && same_parent_dir(&p.file_a, &p.file_b) {
            continue;
        }

        let Some(cp) = coupling_map.get(&(p.file_a.clone(), p.file_b.clone())) else {
            continue; // not a Fisher-significant coupling pair
        };

        // Additional mitigation: minimum shared_revs floor (default 3 per brief).
        if cp.shared < opts.min_clone_shared_revs {
            continue;
        }

        let degree_pct = cp.degree / 100.0; // CouplingRow.degree is 0–100 pct (already f64)
        // Carry the real Fisher exact p-value from the upstream coupling
        // row. Lower p ⇒ stronger co-change signal ⇒ higher combined_score.
        let fisher_p = cp.fisher_p;
        let combined_score = p.similarity * degree_pct * (1.0 - fisher_p);

        rows.push(CloneCouplingRow {
            clone_group_id: p.clone_group_id,
            fingerprint: p.fingerprint,
            file_a: p.file_a,
            file_b: p.file_b,
            entity_a: p.entity_a,
            entity_b: p.entity_b,
            start_line_a: p.start_line_a,
            end_line_a: p.end_line_a,
            start_line_b: p.start_line_b,
            end_line_b: p.end_line_b,
            node_count: p.node_count,
            similarity: p.similarity,
            shared_revs: cp.shared,
            support_a: cp.revs_a,
            support_b: cp.revs_b,
            degree_pct,
            p_value: fisher_p,
            combined_score,
            at_risk: false, // populated below from knowledge-islands intersection
        });
    }

    // T9: intersect with knowledge-islands. Any clone-coupling row whose
    // file_a or file_b is in the knowledge-islands result set gets
    // at_risk = true. Failures are non-fatal — we degrade gracefully to
    // at_risk = false if the sub-analysis errors (the clone-coupling
    // analysis itself is the primary product; knowledge-loss is an
    // enrichment signal).
    let islands_paths: std::collections::HashSet<String> =
        match crate::analyses::knowledge_islands::run_knowledge_islands(db, opts) {
            Ok(islands) => islands.into_iter().map(|r| r.entity).collect(),
            Err(e) => {
                tracing::debug!(
                    "clone-coupling: knowledge-islands sub-analysis errored ({e}); \
                     proceeding with at_risk = false for all rows",
                );
                std::collections::HashSet::new()
            }
        };
    for row in &mut rows {
        if islands_paths.contains(&row.file_a) || islands_paths.contains(&row.file_b) {
            row.at_risk = true;
        }
    }

    // Stable sort: at_risk DESC (knowledge-loss clones surface first),
    // then combined_score DESC, then clone_group_id + file pair for
    // deterministic CSV output across runs.
    rows.sort_by(|a, b| {
        b.at_risk
            .cmp(&a.at_risk)
            .then_with(|| {
                b.combined_score
                    .partial_cmp(&a.combined_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.clone_group_id.cmp(&b.clone_group_id))
            .then_with(|| a.file_a.cmp(&b.file_a))
            .then_with(|| a.file_b.cmp(&b.file_b))
    });

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }

    Ok(rows)
}

fn same_parent_dir(a: &str, b: &str) -> bool {
    let parent = |p: &str| p.rfind('/').map(|i| p[..i].to_string()).unwrap_or_default();
    parent(a) == parent(b)
}
