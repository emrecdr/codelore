//! `code-familiarity` analysis — what fraction of SLOC is actively known by
//! the team's current contributors.
//!
//! Familiarity score = `Σ_f sloc_f` · (`Σ_{d ∈ active} k_norm(d,f)`) / `Σ_f sloc_f` × 100
//!
//! where "active" means any author with ≥1 commit in the trailing
//! `opts.window_days` window (anchored to MAX(date) for reproducibility).
//! Knowledge shares are computed by [`materialize_knowledge_shares`], which
//! applies exponential decay (Jabrayilzade et al., ICSE-SEIP 2022) and
//! reviewer credit (Rigby & Bird, ESEC/FSE 2013).
//!
//! Islands score = % of SLOC where the top `k_norm` author ≥ 0.8 AND
//! the second author's `k_norm` < 0.2 (or the file has only one contributor).
//! A file is an "island" when one person holds dominant, essentially
//! unchallenged knowledge of it.
//!
//! Verdict: `"good"` when `familiarity_pct ≥ threshold` (default 70.0;
//! overridden by `[gates] code_familiarity_min` in `.codelore-thresholds.toml`).

use crate::analyses::knowledge::shares::materialize_knowledge_shares;
use crate::facts::FactsDb;
use crate::quality_gates::Thresholds;
use crate::{CodeLoreError, Options, Result};

/// Default familiarity threshold when not configured in the thresholds file.
const DEFAULT_FAMILIARITY_THRESHOLD: f64 = 70.0;

/// One-row summary of code familiarity for the repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeFamiliarityRow {
    /// Always `"repo"` (v1 emits a single repo-scope row).
    pub scope: String,
    /// Percentage of SLOC covered by active-team knowledge.
    ///
    /// Computed as `Σ_f sloc_f` · (`Σ_{d ∈ active} k_norm(d,f)`) / `Σ_f sloc_f` × 100.
    pub familiarity_pct: f64,
    /// Authors with ≥1 commit in the trailing `window_days` window.
    pub active_authors: u32,
    /// All distinct authors in `knowledge_shares`.
    pub total_authors: u32,
    /// Percentage of SLOC in files where one author holds ≥0.8 knowledge
    /// share and the second-ranked author holds < 0.2 (or no second author).
    pub islands_pct: f64,
    /// `"good"` when `familiarity_pct ≥ threshold`, otherwise `"risky"`.
    pub verdict: String,
}

/// Run the code-familiarity analysis.
///
/// Returns a single-element `Vec` with the repo-scope familiarity row, or an
/// empty `Vec` when the `knowledge_shares` table has no rows (e.g. the repo
/// has no recognized source files and `complexity_metrics` is empty).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on SQL or row-mapping failures.
#[allow(clippy::too_many_lines)]
pub fn run_code_familiarity(db: &FactsDb, opts: &Options) -> Result<Vec<CodeFamiliarityRow>> {
    // Step 1: ensure knowledge_shares + doe_scores are materialised.
    materialize_knowledge_shares(db, opts)?;

    let wd = opts.window_days;

    // Step 2+3: single query computing familiarity, islands, and author counts.
    //
    // path_familiarity sums k_norm for active authors per file, weighted by SLOC.
    // path_rank assigns rank 1 (top) and rank 2 (second) by k_norm per path.
    // island_paths selects files where top ≥ 0.8 and second < 0.2 (or absent).
    let sql = format!(
        "
        WITH anchor AS (
            SELECT MAX(date) AS max_d FROM commits
        ),
        active_authors AS (
            SELECT DISTINCT canonical_author
            FROM commits
            WHERE date >= (SELECT max_d FROM anchor) - INTERVAL '{wd} days'
        ),
        sloc_per_path AS (
            SELECT path, GREATEST(CAST(SUM(sloc) AS BIGINT), 0) AS sloc
            FROM complexity_metrics
            GROUP BY path
        ),
        path_familiarity AS (
            SELECT ks.path,
                   COALESCE(spp.sloc, 0) AS sloc,
                   SUM(CASE WHEN aa.canonical_author IS NOT NULL
                            THEN ks.k_norm ELSE 0.0 END) AS active_k_sum
            FROM knowledge_shares ks
            LEFT JOIN sloc_per_path spp ON spp.path = ks.path
            LEFT JOIN active_authors  aa ON aa.canonical_author = ks.author
            GROUP BY ks.path, spp.sloc
        ),
        path_rank AS (
            SELECT path, k_norm,
                   ROW_NUMBER() OVER (PARTITION BY path ORDER BY k_norm DESC) AS rk
            FROM knowledge_shares
        ),
        path_top AS (
            SELECT path, k_norm AS top_k FROM path_rank WHERE rk = 1
        ),
        path_second AS (
            SELECT path, k_norm AS second_k FROM path_rank WHERE rk = 2
        ),
        island_paths AS (
            SELECT pt.path
            FROM path_top pt
            LEFT JOIN path_second ps ON ps.path = pt.path
            WHERE pt.top_k >= 0.8
              AND (ps.second_k IS NULL OR ps.second_k < 0.2)
        ),
        island_sloc AS (
            SELECT COALESCE(SUM(spp.sloc), 0) AS v
            FROM island_paths ip
            LEFT JOIN sloc_per_path spp ON spp.path = ip.path
        ),
        totals AS (
            SELECT COALESCE(SUM(CAST(sloc AS DOUBLE) * active_k_sum), 0) AS weighted_active,
                   COALESCE(SUM(sloc), 0)                                 AS total_sloc
            FROM path_familiarity
        ),
        author_counts AS (
            SELECT COUNT(DISTINCT author) AS total_a FROM knowledge_shares
        ),
        active_count AS (
            SELECT COUNT(*) AS active_a FROM active_authors
        )
        SELECT
            100.0 * (SELECT weighted_active FROM totals)
                  / NULLIF((SELECT total_sloc FROM totals), 0)  AS familiarity_pct,
            100.0 * (SELECT v FROM island_sloc)
                  / NULLIF((SELECT total_sloc FROM totals), 0)  AS islands_pct,
            (SELECT active_a FROM active_count)::INTEGER         AS active_authors,
            (SELECT total_a  FROM author_counts)::INTEGER        AS total_authors
        "
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-familiarity: {e}")))?;

    let row = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<f64>>(0)?,
                r.get::<_, Option<f64>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query code-familiarity: {e}")))?
        .next()
        .transpose()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-familiarity: {e}")))?;

    let Some((familiarity_pct_opt, islands_pct_opt, active_authors, total_authors)) = row else {
        return Ok(vec![]);
    };

    // NULL familiarity means knowledge_shares is empty (no complexity data).
    let Some(familiarity_pct) = familiarity_pct_opt else {
        return Ok(vec![]);
    };
    let islands_pct = islands_pct_opt.unwrap_or(0.0);

    // Step 4: load optional threshold from .codelore-thresholds.toml.
    let thresholds = Thresholds::discover(&opts.repo_path)?;
    let threshold = thresholds
        .gates
        .code_familiarity_min
        .unwrap_or(DEFAULT_FAMILIARITY_THRESHOLD);

    let verdict = if familiarity_pct >= threshold {
        "good"
    } else {
        "risky"
    };

    Ok(vec![CodeFamiliarityRow {
        scope: "repo".into(),
        familiarity_pct,
        active_authors: u32::try_from(active_authors).unwrap_or(0),
        total_authors: u32::try_from(total_authors).unwrap_or(0),
        islands_pct,
        verdict: verdict.into(),
    }])
}
