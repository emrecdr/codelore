//! `coordination-needs` analysis — per-file coordination overhead signal.
//!
//! Three complementary metrics quantify how much human coordination a file
//! demands:
//!
//! - **Fragmentation** `F = 1 − Σ k_norm²` (HHI complement): the probability
//!   that two randomly chosen knowledge-weighted commits to the file were made
//!   by *different* people.  Zero when one author holds all knowledge; near 1
//!   when knowledge is evenly split across many authors.  Computed from the
//!   decayed knowledge shares produced by [`materialize_knowledge_shares`].
//!   Note: `authors` counts *active-window* contributors (≥1 commit in the
//!   trailing window), while `fragmentation` is computed over *all* knowledge
//!   holders (including historical contributors whose knowledge has partially
//!   decayed), because the HHI sum reflects cumulative knowledge distribution,
//!   not just current activity.
//!
//! - **Interleave** `I = switches / (n_commits − 1)`: the fraction of
//!   chronologically adjacent commit pairs touching the file where the author
//!   switches.  A value near 1 means nearly every commit is by a different
//!   person than the previous one — classic ping-pong ownership churn.  Zero
//!   when fewer than 2 commits exist for the file (stable or untouched).
//!
//! - **Co-change entropy** `H'_a`: file *a*'s contribution to the co-change
//!   graph's structural entropy (co-change graph entropy, EASE 2025,
//!   arXiv 2504.18511).  Only commits touching ≤30 files are included (large
//!   "shotgun" commits bloat every file's degree without reflecting real
//!   coupling).  `p'_k = deg(k) / (2|E|)` is the probability that a random
//!   edge-endpoint is node *k*; `H'(S) = −Σ p'_k · ln(p'_k)` is the global
//!   entropy; `H'_a = p'_a · H'(S)` is the per-file contribution.  Log base
//!   is **ln** (natural); the paper leaves the base unspecified and ranks are
//!   invariant to base choice.  Files with no co-change edges receive `H'_a =
//!   0.0` and are still emitted (they appear in the path-aggregated result).
//!
//! **Tier classification** (for triage prioritisation):
//! - `single`: `authors == 1` — no coordination needed yet.
//! - `low`: `fragmentation < 0.25` — one author dominates, others are minor.
//! - `medium`: `fragmentation ∈ [0.25, 0.50)` OR `interleave < 0.50`.
//! - `high`: `fragmentation ≥ 0.50 AND interleave ≥ 0.50` — strong signal.
//!
//! **`health_band`** joins the file's current composite code-health band
//! (`red` / `yellow` / `green`) from [`run_code_health_scoped`].
//! Coordination overhead in a `red`-band file is the highest-leverage
//! refactoring signal: ownership fragmentation *and* structural debt.
//!
//! Future note: `cochange_entropy` would make a natural additional Kamei JIT-SDP
//! feature column (structural coupling load per change), but the Kamei table is
//! ingest-shaped and cannot be extended at analysis time.

use std::collections::HashMap;

use crate::analyses::code_health::{HealthScanCtx, run_code_health_scoped};
use crate::analyses::knowledge::shares::materialize_knowledge_shares;
use crate::analyses::lineage::source_table;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Per-file coordination-needs row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CoordinationNeedsRow {
    /// File path (rename-aware via the lineage CTE).
    pub path: String,
    /// Distinct active-window authors (≥1 commit in the trailing window).
    pub authors: u32,
    /// HHI complement over all knowledge holders: `1 − Σ k_norm²`.
    /// Zero = single owner; near 1 = evenly distributed knowledge.
    pub fragmentation: f64,
    /// Author-switch fraction between chronologically adjacent commits: 0 if
    /// fewer than 2 commits touch the file.
    pub interleave: f64,
    /// Per-file co-change graph entropy contribution (EASE 2025,
    /// arXiv 2504.18511); 0.0 for files with no co-change edges in the window.
    pub cochange_entropy: f64,
    /// Triage tier: `single` | `low` | `medium` | `high`.
    pub tier: String,
    /// Composite code-health band at HEAD: `red` | `yellow` | `green` |
    /// `unknown` (when no complexity metrics exist for the file).
    pub health_band: String,
}

/// Compute coordination-needs metrics for every path that appears in the
/// trailing `opts.window_days` window.
///
/// # Steps
///
/// 1. Materialise `knowledge_shares` + `doe_scores` (idempotent).
/// 2. Compute fragmentation and active-window author count per path from
///    `knowledge_shares`.
/// 3. Compute interleave per path from the chronological commit sequence
///    (using `LAG` over `changes JOIN commits`).
/// 4. Compute co-change entropy per path from the unweighted co-change graph
///    restricted to the trailing window (nf ≤ 30).
/// 5. Fetch code-health band per path (reuse `eh_bands_v1` pattern from
///    `effort_exposure`).
/// 6. Classify tier and assemble rows.
#[allow(clippy::too_many_lines)]
pub fn run_coordination_needs(db: &FactsDb, opts: &Options) -> Result<Vec<CoordinationNeedsRow>> {
    // Step 1: ensure knowledge_shares is materialised (idempotent).
    materialize_knowledge_shares(db, opts)?;

    let src = source_table(opts);

    // Step 2: fragmentation + active authors per path.
    // `fragmentation` = HHI complement over ALL knowledge holders (decayed
    // shares may be > 0 for historical authors, even if inactive this window).
    // `authors` = distinct active-window contributors only.
    let frag_sql = format!(
        "WITH window_cutoff AS (
             SELECT MAX(date) - INTERVAL ({wd}) DAY AS cutoff FROM commits
         ),
         active AS (
             SELECT DISTINCT ch.path, co.canonical_author
             FROM {src} ch
             JOIN commits co ON co.rev = ch.rev
             CROSS JOIN window_cutoff
             WHERE co.date >= window_cutoff.cutoff
         ),
         frag AS (
             SELECT
                 ks.path,
                 1.0 - SUM(ks.k_norm * ks.k_norm) AS fragmentation
             FROM knowledge_shares ks
             GROUP BY ks.path
         ),
         act_count AS (
             SELECT path, COUNT(DISTINCT canonical_author) AS active_authors
             FROM active
             GROUP BY path
         )
         SELECT frag.path,
                COALESCE(act_count.active_authors, 0) AS authors,
                frag.fragmentation
         FROM frag
         LEFT JOIN act_count ON act_count.path = frag.path
         WHERE frag.path IS NOT NULL
         ORDER BY frag.fragmentation DESC",
        src = src,
        wd = opts.window_days,
    );
    let mut frag_stmt = db
        .conn()
        .prepare(&frag_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare fragmentation query: {e}")))?;
    let frag_rows = frag_stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query fragmentation: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect fragmentation: {e}")))?;

    // Step 3: interleave — author-switch fraction between adjacent commits.
    // LAG() over (PARTITION BY path ORDER BY date, rowid) gives the previous
    // author for each commit on that path; switching = prev IS NOT NULL AND
    // prev != current.
    let interleave_sql = format!(
        "WITH ordered AS (
             SELECT
                 ch.path,
                 co.canonical_author                                        AS cur,
                 LAG(co.canonical_author) OVER (
                     PARTITION BY ch.path ORDER BY co.date, co.rowid
                 )                                                          AS prev
             FROM {src} ch
             JOIN commits co ON co.rev = ch.rev
         ),
         stats AS (
             SELECT
                 path,
                 COUNT(*)                                        AS n_commits,
                 COUNT(*) FILTER (WHERE prev IS NOT NULL AND prev != cur) AS switches
             FROM ordered
             GROUP BY path
         )
         SELECT path,
                CASE WHEN n_commits < 2 THEN 0.0
                     ELSE switches * 1.0 / (n_commits - 1)
                END AS interleave
         FROM stats",
    );
    let mut il_stmt = db
        .conn()
        .prepare(&interleave_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare interleave query: {e}")))?;
    let interleave_map: HashMap<String, f64> = il_stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query interleave: {e}")))?
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect interleave: {e}")))?;

    // Step 4: co-change entropy (EASE 2025, arXiv 2504.18511).
    // Restrict to commits touching ≤30 files in the trailing window.
    // p'_k = deg(k) / (2|E|); H'(S) = −Σ p'_k · ln(p'_k); H'_a = p'_a · H'.
    // Log base = ln (natural); ranks invariant to base choice.
    let entropy_sql = format!(
        "WITH win AS (
             SELECT co.rev FROM commits co
             WHERE co.nf <= 30
               AND co.date >= (SELECT MAX(date) FROM commits) - INTERVAL ({wd}) DAY
         ),
         edges AS (
             SELECT DISTINCT
                 LEAST(a.path, b.path)    AS p1,
                 GREATEST(a.path, b.path) AS p2
             FROM {src} a
             JOIN {src} b ON a.rev = b.rev AND a.path < b.path
             JOIN win w ON w.rev = a.rev
         ),
         deg AS (
             SELECT path, COUNT(*) AS d
             FROM (
                 SELECT p1 AS path FROM edges
                 UNION ALL
                 SELECT p2 AS path FROM edges
             )
             GROUP BY path
         ),
         tot AS (SELECT SUM(d) AS twoe FROM deg),
         p AS (
             SELECT path, d * 1.0 / twoe AS pk
             FROM deg, tot
             WHERE twoe > 0
         ),
         h AS (
             SELECT -SUM(pk * LN(pk)) AS hs FROM p WHERE pk > 0
         )
         SELECT p.path, p.pk * h.hs AS h_a FROM p, h",
        src = src,
        wd = opts.window_days,
    );
    let mut ent_stmt = db
        .conn()
        .prepare(&entropy_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare entropy query: {e}")))?;
    let entropy_map: HashMap<String, f64> = ent_stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query entropy: {e}")))?
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect entropy: {e}")))?;

    // Step 5: code-health band per path.
    let ctx = HealthScanCtx::head_default();
    let health_rows = run_code_health_scoped(db, opts, &ctx)?;
    let band_map: HashMap<String, String> =
        health_rows.into_iter().map(|r| (r.path, r.band)).collect();

    // Step 6: assemble rows, classify tier.
    let mut rows: Vec<CoordinationNeedsRow> = frag_rows
        .into_iter()
        .map(|(path, authors, fragmentation)| {
            let interleave = interleave_map.get(&path).copied().unwrap_or(0.0);
            let cochange_entropy = entropy_map.get(&path).copied().unwrap_or(0.0);
            let health_band = band_map
                .get(&path)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            let tier = classify_tier(authors, fragmentation, interleave);
            CoordinationNeedsRow {
                path,
                authors,
                fragmentation,
                interleave,
                cochange_entropy,
                tier,
                health_band,
            }
        })
        .collect();

    // Sort by fragmentation descending (highest coordination overhead first).
    rows.sort_by(|a, b| {
        b.fragmentation
            .partial_cmp(&a.fragmentation)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(rows)
}

/// Classify a coordination tier from author count, fragmentation, and interleave.
///
/// Rules (evaluated top-to-bottom, first match wins):
/// - `single`:  `authors == 1` — exactly one active-window contributor.
///   Authors=0 is impossible: the fragmentation CTE only emits paths that
///   appear in `knowledge_shares`, which requires ≥1 commit, so ≥1 author.
/// - `low`:     `fragmentation < 0.25` — one author dominates, others are minor.
/// - `high`:    `fragmentation ≥ 0.50 AND interleave ≥ 0.50` — strong signal.
/// - `medium`:  everything else.
fn classify_tier(authors: u32, fragmentation: f64, interleave: f64) -> String {
    if authors == 1 {
        return "single".to_string();
    }
    if fragmentation < 0.25 {
        return "low".to_string();
    }
    if fragmentation >= 0.50 && interleave >= 0.50 {
        return "high".to_string();
    }
    "medium".to_string()
}
