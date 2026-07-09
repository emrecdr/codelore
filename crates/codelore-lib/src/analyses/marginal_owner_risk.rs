//! Marginal-owner risk: ownership concentration × code-health fusion.
//!
//! For each file in the yellow or red health band, reports the maximum
//! knowledge share held by any author who committed within the trailing
//! `window_days`. A low top-active share on an already-unhealthy file
//! signals that the people most likely to fix regressions there have
//! shallow familiarity — the "marginal owner" who would have to step in
//! has little context to work with.
//!
//! Risk tier:
//! - `high`     — red band AND top active share < 0.10
//! - `elevated` — (red AND share < 0.30) OR (yellow AND share < 0.10)
//! - rows that do not meet either threshold are excluded (green band or
//!   sufficiently concentrated ownership)
//!
//! The ownership × code-quality interaction is correlational, not causal.
//! See: Palomba et al., "On the Interplay between Structural Smells and
//! Code Ownership" EASE 2023, arXiv 2304.11636.
use crate::CodeLoreError;
use crate::analyses::code_health::{HealthScanCtx, run_code_health_scoped};
use crate::analyses::knowledge::shares::materialize_knowledge_shares;
use crate::analyses::lineage;
use crate::facts::FactsDb;
use crate::options::Options;
use serde::Serialize;

/// A single file flagged with marginal-owner risk.
#[derive(Debug, Clone, Serialize)]
pub struct MarginalOwnerRiskRow {
    /// File path (canonical via lineage when enabled).
    pub path: String,
    /// Code-health band of the file at HEAD: `"yellow"` or `"red"`.
    pub band: String,
    /// Maximum `k_norm` knowledge share among authors active within
    /// `window_days`. In `[0.0, 1.0]`; 0.0 if no active author has any
    /// share in the `knowledge_shares` table.
    pub top_active_share: f64,
    /// Risk tier: `"high"` or `"elevated"`. Rows that do not satisfy
    /// either tier threshold are excluded from the output.
    pub risk: String,
    /// Fixed explanatory note for the row (correlational signal).
    pub note: String,
}

/// Classify a file's risk tier from its health band and the maximum
/// knowledge share held by any active author.
///
/// Returns `Some("high")`, `Some("elevated")`, or `None` (excluded).
/// This function is pure — no I/O — so it can be unit-tested directly
/// without a database fixture.
#[must_use]
pub fn classify_risk(band: &str, top_active_share: f64) -> Option<&'static str> {
    if band == "red" && top_active_share < 0.10 {
        return Some("high");
    }
    if (band == "red" && top_active_share < 0.30) || (band == "yellow" && top_active_share < 0.10) {
        return Some("elevated");
    }
    None
}

/// Run the marginal-owner-risk analysis over the already-ingested fact store.
///
/// Steps:
/// 1. Materialize `changes_lineage` (if canonical lineage is enabled) and
///    `knowledge_shares` (idempotent guard inside).
/// 2. Run code-health at HEAD to get per-file band labels.
/// 3. For each yellow/red file, query the maximum `k_norm` among authors
///    who committed within `opts.window_days`.
/// 4. Apply [`classify_risk`] and emit rows for `high` / `elevated` tiers
///    only.
pub fn run_marginal_owner_risk(
    db: &FactsDb,
    opts: &Options,
) -> Result<Vec<MarginalOwnerRiskRow>, CodeLoreError> {
    // Prerequisite: changes_lineage must exist before source_table() is used.
    lineage::materialize_if_needed(db, opts)?;

    // Prerequisite: knowledge_shares table (materialize is idempotent).
    materialize_knowledge_shares(db, opts)?;

    // Obtain code-health bands at HEAD.
    let cx = HealthScanCtx::head_default();
    let health_rows = run_code_health_scoped(db, opts, &cx)?;

    // Build a map path → band for yellow/red files only.
    // Green rows are unconditionally excluded — no risk signal.
    let unhealthy: Vec<(String, String)> = health_rows
        .into_iter()
        .filter(|r| r.band == "yellow" || r.band == "red")
        .map(|r| (r.path, r.band))
        .collect();

    if unhealthy.is_empty() {
        return Ok(Vec::new());
    }

    // For each unhealthy path, find the maximum k_norm among ACTIVE authors
    // (authors who have a commit within the trailing window_days).
    // We use a single parameterised query per path rather than one massive
    // IN-list query to stay within DuckDB's parameter binding limits and
    // keep the query plan simple.
    //
    // Active-author CTE: any author with at least one commit in
    //   [MAX(date) - window_days, MAX(date)]
    // per the same window convention used by every other windowed analysis.
    let src = lineage::source_table(opts);

    // Build result rows.
    let note = "changes here historically run 45-93% slower (ownership\u{00d7}health interaction)";
    let mut rows: Vec<MarginalOwnerRiskRow> = Vec::new();

    for (path, band) in &unhealthy {
        // Query: max k_norm for active authors on this path.
        // Active = committed to any file within window_days of repo HEAD date.
        // Inline the path literal with single-quote escaping (path values
        // come from the fact store — they are file paths, not user input).
        let path_escaped = path.replace('\'', "''");
        let sql = format!(
            "
            WITH repo_end AS (
                SELECT MAX(date) AS end_date FROM commits
            ),
            active_authors AS (
                SELECT DISTINCT co.canonical_author
                FROM {src} ch
                JOIN commits co ON co.rev = ch.rev
                JOIN repo_end re ON TRUE
                WHERE co.date >= re.end_date - INTERVAL '{window}' DAY
            ),
            path_shares AS (
                SELECT ks.k_norm
                FROM knowledge_shares ks
                JOIN active_authors aa ON aa.canonical_author = ks.author
                WHERE ks.path = '{path_escaped}'
            )
            SELECT COALESCE(MAX(k_norm), 0.0) AS top_active_share
            FROM path_shares
            ",
            src = src,
            window = opts.window_days,
            path_escaped = path_escaped,
        );

        let top_active_share: f64 = db.query_row(&sql, [], |row| row.get(0))?;

        if let Some(risk) = classify_risk(band, top_active_share) {
            rows.push(MarginalOwnerRiskRow {
                path: path.clone(),
                band: band.clone(),
                top_active_share,
                risk: risk.to_owned(),
                note: note.to_owned(),
            });
        }
    }

    Ok(rows)
}
