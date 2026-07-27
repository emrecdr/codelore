//! Dashboard-specific analyses: the parameterised SQL queries that feed
//! the single-file SPA dashboard's widgets.
//!
//! These mirror the per-analysis modules elsewhere in `analyses/` — each
//! is a parameterised SQL query over the already-ingested fact store that
//! returns a row struct. They live here rather than in the `output` layer
//! because a query failure is an analysis failure (exit code 4), not an
//! output/I/O failure. The row structs they return are serialised into the
//! SPA JSON payload by `output::spa`.

use serde::Deserialize;

use crate::{CodeLoreError, Result};

/// One function in the X-Ray sunburst.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct XRayEntry {
    pub path: String,
    pub function: String,
    pub cognitive: f64,
    pub start_line: u32,
    pub end_line: u32,
}

/// One day's commit count for the calendar heatmap.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct DailyCommit {
    pub date: String,
    pub count: u32,
}

/// One (month, path, score) point in the trends multi-line.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct TrendPoint {
    pub month: String,
    pub path: String,
    pub hotspot_score: f64,
}

/// Per-file clone overlay row: how many distinct clone groups touch the
/// path. Surfaced as a colour-mode toggle on the hotspot circle-pack so
/// users can see structural-duplication hotspots overlaid on the same
/// file layout they already know from the cognitive / author / AI modes.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct CloneSummary {
    pub path: String,
    /// Number of distinct `clone_group_id`s the path appears in. A file
    /// that's part of N independent clone families has `groups = N`.
    pub groups: u32,
}

/// One resolved import edge for the architecture force-graph widget.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct ImportEdgeRow {
    pub src_path: String,
    pub target_path: String,
}

/// Per-commit Kamei JIT-SDP feature row for the Delivery Risk
/// Sparkline widget. Drops the merge-commit subset (their Kamei
/// vectors are 0 by design — see
/// `gix_repo.rs::changed_files_for_commit`) and the date-null
/// fringe.
///
/// Feature definitions per Kamei et al. 2013 §3:
///   - `la` / `ld` — lines added / deleted (Size dimension)
///   - `nf` — files changed (Diffusion)
///   - `nd` — directories changed (Diffusion)
///   - `ndev` — distinct devs who touched the same files before this commit (History)
///   - `nuc` — unique changes per file before this commit (History)
///   - `exp` — author's general experience (History)
///   - `entropy` — distribution of changes across files (Diffusion)
///   - `fix` — is this a bug-fix commit (Purpose)
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct KameiRiskRow {
    pub rev: String,
    /// ISO date `YYYY-MM-DD` of the commit (committer date).
    pub date: String,
    pub la: u32,
    pub ld: u32,
    pub nf: u32,
    pub nd: u32,
    pub ndev: u32,
    pub nuc: u32,
    pub exp: u32,
    pub entropy: f64,
    pub fix: bool,
}

/// Pull every resolved import edge from the `imports` table. Only
/// resolved edges (where `target_path` is non-NULL) participate so
/// the SPA graph reflects the dependency surface `CodeLore` can
/// actually visualise.
///
/// # Errors
///
/// Propagates `DuckDB` prepare / query errors as
/// [`CodeLoreError::Analysis`].
#[tracing::instrument(name = "dashboard-imports", skip_all)]
pub fn run_imports_for_arch_graph(db: &crate::facts::FactsDb) -> Result<Vec<ImportEdgeRow>> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT src_path, target_path FROM imports \
             WHERE target_path IS NOT NULL \
             ORDER BY src_path ASC, target_path ASC",
        )
        .map_err(|e| CodeLoreError::Analysis(format!("arch-imports prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ImportEdgeRow {
                src_path: r.get(0)?,
                target_path: r.get(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("arch-imports query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("arch-imports collect: {e}")))
}

/// Aggregate function-level cognitive complexity per (path, function)
/// from the `complexity_metrics` table for the X-Ray sunburst (W8).
/// Returns at most `limit` rows ordered by `cognitive DESC`, since
/// the sunburst becomes unreadable past a few hundred functions and
/// the JSON payload would blow up on monorepos otherwise.
#[tracing::instrument(name = "dashboard-xray", skip_all)]
pub fn run_xray(db: &crate::facts::FactsDb, limit: i64) -> Result<Vec<XRayEntry>> {
    // Join `complexity_metrics` (cognitive score) with `entities`
    // (line range) on (path, name, rev). The entities table has the
    // start/end lines; complexity_metrics has the metrics. Both share
    // the same (path, name, rev) primary-key columns. The JOIN is
    // exact — every row in complexity_metrics has a matching row in
    // entities by construction (they're populated in the same
    // ingest pass).
    let mut stmt = db
        .conn()
        .prepare(
            // The `e.rev_last_seen = cm.rev` filter is the lockstep
            // invariant from `facts/ingest.rs`: append_entity_row and
            // append_metric_row both receive the same head_rev. Earlier
            // versions used `e.rev_introduced <= cm.rev AND e.rev_last_seen
            // >= cm.rev` (a lex SHA range) which only happened to work
            // when complexity_metrics has a single rev — random
            // failures the moment an incremental ingest ships. Equality
            // on the lockstep field is the correct semantic and matches
            // the file_mi CTE in `analyses/hotspots.rs`.
            "SELECT cm.path,
                    cm.name,
                    cm.cognitive,
                    CAST(e.start_line AS UINTEGER) AS s_line,
                    CAST(e.end_line AS UINTEGER) AS e_line
             FROM complexity_metrics cm
             INNER JOIN entities e
                ON e.path = cm.path
                AND e.name = cm.name
                AND e.rev_last_seen = cm.rev
             WHERE cm.cognitive > 0
             ORDER BY cm.cognitive DESC, cm.path ASC, cm.name ASC
             LIMIT ?",
        )
        .map_err(|e| CodeLoreError::Analysis(format!("xray prepare: {e}")))?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok(XRayEntry {
                path: r.get(0)?,
                function: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                cognitive: r.get(2)?,
                start_line: r.get(3)?,
                end_line: r.get(4)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("xray query: {e}")))?;
    let out: std::result::Result<Vec<_>, _> = rows.collect();
    out.map_err(|e| CodeLoreError::Analysis(format!("xray collect: {e}")))
}

/// Per-file clone-group counts from the `clones` table. Empty result
/// when no clone groups exist (small repo, no Tier-1 sources, or
/// `min_clone_node_count` filtered everything out at ingest time).
/// One row per path that appears in ≥ 1 clone family — files with
/// zero clone groups are dropped so the payload stays compact.
///
/// # Errors
/// Returns [`CodeLoreError::Analysis`] on any `DuckDB` failure.
#[tracing::instrument(name = "dashboard-clone-summary", skip_all)]
pub fn run_clone_summary(db: &crate::facts::FactsDb) -> Result<Vec<CloneSummary>> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT path, COUNT(DISTINCT clone_group_id)::UINTEGER AS groups
             FROM clones
             GROUP BY path
             ORDER BY groups DESC, path ASC",
        )
        .map_err(|e| CodeLoreError::Analysis(format!("clone_summary prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CloneSummary {
                path: r.get(0)?,
                groups: r.get(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("clone_summary query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("clone_summary collect: {e}")))
}

/// Build a per-(month, path) trend series restricted to `paths` for
/// the trends multi-line widget (W9). The score per (month, path) is
/// the count of revisions that touched the path during the month.
/// Empty `paths` returns an empty Vec — the widget renders nothing.
#[tracing::instrument(name = "dashboard-trends", skip_all)]
pub fn run_trends(
    db: &crate::facts::FactsDb,
    opts: &crate::Options,
    paths: &[String],
) -> Result<Vec<TrendPoint>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    // Aggregate over the canonical-lineage change source so a renamed file's
    // pre-rename revisions fold onto the head path this series is keyed to —
    // the `paths` list is the lineage-canonical hotspot set, so raw `changes`
    // would undercount every renamed file's history. Mirrors
    // `lineage::materialize_if_needed`'s own guard; the bucketed source is
    // deliberately excluded because the `ch.rev = c.rev` join below needs the
    // real per-commit revs that `changes_bucketed` collapses into a date key.
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let src = if opts.use_canonical_lineage && opts.time_bucket.is_none() {
        "changes_lineage"
    } else {
        "changes"
    };
    // Bind `paths` via UNNEST so we don't string-interpolate user data
    // into the SQL. DuckDB's list_value() / array binding accepts an
    // owned Vec<String>; we materialise the path list as a temp table
    // via VALUES instead since the duckdb crate's parameter binding
    // doesn't accept Vec<String> directly.
    //
    // Strategy: build a `VALUES (?), (?), ...` clause sized to paths.len().
    let placeholders = std::iter::repeat_n("(?)", paths.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "WITH paths(path) AS (VALUES {placeholders})
         SELECT strftime(date_trunc('month', c.date), '%Y-%m-%d') AS month,
                ch.path,
                CAST(COUNT(*) AS DOUBLE) AS score
         FROM commits c
         INNER JOIN {src} ch ON ch.rev = c.rev
         INNER JOIN paths USING (path)
         GROUP BY month, ch.path
         ORDER BY month ASC, ch.path ASC"
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("trends prepare: {e}")))?;
    let params: Vec<&dyn duckdb::ToSql> = paths.iter().map(|p| p as &dyn duckdb::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |r| {
            Ok(TrendPoint {
                month: r.get(0)?,
                path: r.get(1)?,
                hotspot_score: r.get(2)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("trends query: {e}")))?;
    let out: std::result::Result<Vec<_>, _> = rows.collect();
    out.map_err(|e| CodeLoreError::Analysis(format!("trends collect: {e}")))
}

/// Per-day commit counts for the calendar heatmap (W10). Returns one
/// row per day with at least one commit, sorted by date ascending.
#[tracing::instrument(name = "dashboard-daily-commits", skip_all)]
pub fn run_daily_commits(db: &crate::facts::FactsDb) -> Result<Vec<DailyCommit>> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT CAST(CAST(date AS DATE) AS TEXT) AS d,
                    CAST(COUNT(*) AS UINTEGER) AS n
             FROM commits
             GROUP BY CAST(date AS DATE)
             ORDER BY d ASC",
        )
        .map_err(|e| CodeLoreError::Analysis(format!("daily_commits prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DailyCommit {
                date: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("daily_commits query: {e}")))?;
    let out: std::result::Result<Vec<_>, _> = rows.collect();
    out.map_err(|e| CodeLoreError::Analysis(format!("daily_commits collect: {e}")))
}

/// Pull the last-N non-merge commits with their Kamei JIT-SDP
/// feature vector for the Delivery Risk Sparkline widget. Returns
/// rows in chronological order (oldest → newest) so the widget can
/// render left-to-right as a calendar-time bar series.
///
/// COALESCE(...0) on every Kamei feature guards against the
/// nullable schema columns — fresh fixtures or analyses where a
/// commit's Kamei vector wasn't populated render as zero-risk bars
/// rather than crashing the serialisation.
///
/// # Errors
///
/// Propagates `DuckDB` prepare / query errors as
/// [`CodeLoreError::Analysis`].
#[tracing::instrument(name = "dashboard-kamei-risk", skip_all)]
pub fn run_kamei_risk(db: &crate::facts::FactsDb, limit: i64) -> Result<Vec<KameiRiskRow>> {
    let sql = "
        WITH recent AS (
            SELECT rev, date, la, ld, nf, nd, ndev, nuc, exp, entropy, fix
            FROM commits
            WHERE is_merge = FALSE AND date IS NOT NULL
            ORDER BY date DESC, rowid DESC
            LIMIT ?
        )
        SELECT rev,
               strftime(date, '%Y-%m-%d') AS date,
               COALESCE(la, 0)::UINTEGER AS la,
               COALESCE(ld, 0)::UINTEGER AS ld,
               COALESCE(nf, 0)::UINTEGER AS nf,
               COALESCE(nd, 0)::UINTEGER AS nd,
               COALESCE(ndev, 0)::UINTEGER AS ndev,
               COALESCE(nuc, 0)::UINTEGER AS nuc,
               COALESCE(exp, 0)::UINTEGER AS exp,
               COALESCE(entropy, 0.0) AS entropy,
               COALESCE(fix, FALSE) AS fix
        FROM recent
        ORDER BY date ASC, rev ASC
    ";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei_risk prepare: {e}")))?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok(KameiRiskRow {
                rev: r.get(0)?,
                date: r.get(1)?,
                la: r.get(2)?,
                ld: r.get(3)?,
                nf: r.get(4)?,
                nd: r.get(5)?,
                ndev: r.get(6)?,
                nuc: r.get(7)?,
                exp: r.get(8)?,
                entropy: r.get(9)?,
                fix: r.get(10)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("kamei_risk query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("kamei_risk collect: {e}")))
}
