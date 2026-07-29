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
use std::collections::HashMap;

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
/// 3. For every yellow/red file, compute the maximum `k_norm` among authors
///    who committed within `opts.window_days`, in a single set query against
///    a one-shot `active_authors` set (not one query per file).
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

    // Maximum k_norm among ACTIVE authors (authors with a commit within the
    // trailing window_days) for every unhealthy path, computed with one set
    // query rather than one query per file.
    let paths: Vec<String> = unhealthy.iter().map(|(path, _)| path.clone()).collect();
    let top_active_shares = top_active_shares_by_path(db, opts, &paths)?;

    // Build result rows.
    let note = "changes here historically run 45-93% slower (ownership\u{00d7}health interaction)";
    let mut rows: Vec<MarginalOwnerRiskRow> = Vec::new();

    for (path, band) in &unhealthy {
        let top_active_share = top_active_shares.get(path).copied().unwrap_or(0.0);

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

/// For every path in `paths`, compute the maximum `k_norm` among authors
/// active within `opts.window_days` of the repo's latest commit date, in a
/// single query.
///
/// Active-author set: any author with at least one commit in
/// `[MAX(date) - window_days, MAX(date)]` — the same window convention used
/// by every other windowed analysis — evaluated once regardless of how many
/// paths are requested. A path with no active-author share (or no share at
/// all) maps to `0.0`, matching `COALESCE(MAX(k_norm), 0.0)` on a per-path
/// query. Paths not present in `paths` are absent from the returned map.
/// Returns an empty map for an empty `paths` slice.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` prepare / query failure.
fn top_active_shares_by_path(
    db: &FactsDb,
    opts: &Options,
    paths: &[String],
) -> Result<HashMap<String, f64>, CodeLoreError> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }

    let src = lineage::source_table(opts);
    // Bind `paths` via a VALUES clause rather than string-interpolating
    // path literals into the SQL text.
    let placeholders = std::iter::repeat_n("(?)", paths.len())
        .collect::<Vec<_>>()
        .join(",");
    let now_anchor = crate::analyses::query::clamped_now_anchor("date");
    let sql = format!(
        "WITH repo_end AS (
             SELECT {now_anchor} AS end_date FROM commits
         ),
         active_authors AS (
             SELECT DISTINCT co.canonical_author
             FROM {src} ch
             JOIN commits co ON co.rev = ch.rev
             JOIN repo_end re ON TRUE
             WHERE co.date >= re.end_date - INTERVAL '{window}' DAY
         ),
         requested_paths(path) AS (
             VALUES {placeholders}
         ),
         path_shares AS (
             SELECT rp.path, ks.k_norm
             FROM requested_paths rp
             JOIN knowledge_shares ks ON ks.path = rp.path
             JOIN active_authors aa ON aa.canonical_author = ks.author
         )
         SELECT rp.path, COALESCE(MAX(ps.k_norm), 0.0) AS top_active_share
         FROM requested_paths rp
         LEFT JOIN path_shares ps ON ps.path = rp.path
         GROUP BY rp.path",
        src = src,
        window = opts.window_days,
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("marginal_owner_risk prepare: {e}")))?;
    let params: Vec<&dyn duckdb::ToSql> = paths.iter().map(|p| p as &dyn duckdb::ToSql).collect();
    stmt.query_map(params.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    })
    .map_err(|e| CodeLoreError::Analysis(format!("marginal_owner_risk query: {e}")))?
    .collect::<std::result::Result<HashMap<_, _>, _>>()
    .map_err(|e| CodeLoreError::Analysis(format!("marginal_owner_risk collect: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::FactsDb;

    /// Regression test for the single set query: each requested path's
    /// maximum active share must come from that path's OWN
    /// `knowledge_shares` rows, never another path's, and an inactive
    /// author's dominant share must not leak through even when it is the
    /// largest value on the path. Hand-crafts `commits` / `changes` /
    /// `knowledge_shares` directly so the expected shares are exact
    /// literals, independent of the decay/reviewer-credit formula that
    /// `materialize_knowledge_shares` applies.
    #[test]
    fn top_active_shares_by_path_does_not_cross_paths() {
        let db = FactsDb::new_in_memory().expect("in-memory db");

        // Alice is old/inactive; Bob and Carol are recent/active. The
        // default 90-day window anchors at MAX(date) = Bob's 2024-05-01
        // commit, so Alice's 2024-01-01 commit (120 days earlier) falls
        // outside it.
        db.conn()
            .execute(
                "INSERT INTO commits (rev, author_email, author_name, \
                 committer_email, canonical_author, date, committer_date, \
                 message, is_merge, parent_count) VALUES \
                 ('c1', 'alice@x.com', 'Alice', 'alice@x.com', 'Alice', \
                  TIMESTAMP '2024-01-01', TIMESTAMP '2024-01-01', 'a', false, 1), \
                 ('c2', 'bob@x.com', 'Bob', 'bob@x.com', 'Bob', \
                  TIMESTAMP '2024-05-01', TIMESTAMP '2024-05-01', 'b', false, 1), \
                 ('c3', 'carol@x.com', 'Carol', 'carol@x.com', 'Carol', \
                  TIMESTAMP '2024-04-20', TIMESTAMP '2024-04-20', 'c', false, 1)",
                [],
            )
            .expect("insert commits");

        // One change row per commit — the active_authors CTE joins
        // through the source table.
        db.conn()
            .execute(
                "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) \
                 VALUES ('c1', 'pathA.rs', 'modified', 1, 0), \
                        ('c2', 'pathB.rs', 'modified', 1, 0), \
                        ('c3', 'pathC.rs', 'modified', 1, 0)",
                [],
            )
            .expect("insert changes");

        // Hand-picked knowledge shares, deliberately not proportional to
        // the changes above — this test targets only the batched query.
        db.conn()
            .execute_batch(
                "CREATE TEMP TABLE knowledge_shares \
                 (path TEXT, author TEXT, k DOUBLE, k_norm DOUBLE)",
            )
            .expect("create knowledge_shares");
        db.conn()
            .execute(
                "INSERT INTO knowledge_shares (path, author, k, k_norm) VALUES \
                 ('pathA.rs', 'Alice', 1.0, 1.0), \
                 ('pathB.rs', 'Bob',   0.8, 0.8), \
                 ('pathB.rs', 'Carol', 0.2, 0.2), \
                 ('pathC.rs', 'Alice', 0.9, 0.9), \
                 ('pathC.rs', 'Carol', 0.1, 0.1), \
                 ('pathD.rs', 'Bob',   1.0, 1.0)",
                [],
            )
            .expect("insert knowledge_shares");

        // use_canonical_lineage=false so source_table() resolves to the
        // plain `changes` table this fixture populates, not `changes_lineage`
        // (which is only materialized by lineage::materialize_if_needed).
        let opts = Options {
            use_canonical_lineage: false,
            ..Options::default()
        };
        let paths = vec![
            "pathA.rs".to_owned(),
            "pathB.rs".to_owned(),
            "pathC.rs".to_owned(),
        ];
        let shares = top_active_shares_by_path(&db, &opts, &paths).expect("batched query");

        // pathA.rs: sole author Alice is inactive → no active author has
        // any share on this path → 0.0.
        assert_eq!(shares.get("pathA.rs").copied(), Some(0.0));
        // pathB.rs: both authors active → max(0.8, 0.2) = 0.8.
        assert_eq!(shares.get("pathB.rs").copied(), Some(0.8));
        // pathC.rs: Alice's dominant 0.9 share belongs to an INACTIVE
        // author and must be excluded; only Carol's active 0.1 counts.
        // A path-crossing or activity-filter bug would surface as 0.9,
        // or as pathB's 0.8, here.
        assert_eq!(shares.get("pathC.rs").copied(), Some(0.1));
        // pathD.rs was never requested — must not leak into the result.
        assert_eq!(shares.get("pathD.rs"), None);
        assert_eq!(shares.len(), 3);
    }

    #[test]
    fn top_active_shares_by_path_empty_paths_returns_empty_map_without_querying() {
        let db = FactsDb::new_in_memory().expect("in-memory db");
        let opts = Options::default();
        let shares = top_active_shares_by_path(&db, &opts, &[]).expect("empty paths");
        assert!(shares.is_empty());
    }
}
