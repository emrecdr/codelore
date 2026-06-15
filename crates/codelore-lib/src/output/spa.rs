//! `--format spa` — single-file interactive dashboard emitter.
//!
//! Opt-in via the `spa` Cargo feature. When the feature is enabled,
//! `build.rs` fetches Apache `ECharts` and d3-hierarchy from jsDelivr at
//! pinned URLs + SHA-256-verifies them; this module embeds those JS deps
//! plus the HTML shell and the widget render glue inline at compile time
//! via `include_str!`, producing a single self-contained `codelore.html`
//! that opens in any browser, runs without a server, fits in a CI
//! artefact, and does not phone home.
//!
//! See `docs/ui-roadmap.md` for the widget plan and the
//! technical-stack justification.
//!
//! # Shape
//!
//! Mirrors `write_full_fact_store_sqlite` (multi-source composite), not
//! the per-row-type generic `write_html<T: Serialize>`. Callers populate
//! a [`SpaDashboard`] struct with the row vectors for each widget, then
//! invoke [`write_spa`] which serialises the struct as JSON, inlines it
//! into the HTML template alongside the vendored JS, and writes the
//! result to the provided sink.
//!
//! # XSS hygiene
//!
//! The serialised JSON sits inside `<script type="application/json">`
//! and must not contain a literal `</script>` substring that would
//! escape the script context. Same precaution as the existing
//! per-analysis HTML emitter in `output/html.rs`: any `</` in the JSON
//! is replaced with `<\/` (HTML-equivalent for spec parsers, foreign
//! to JSON parsers, so the inline JSON still round-trips correctly).

use std::io::Write;

use serde::Serialize;

use serde::Deserialize;

use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::coupling::CouplingRow;
use crate::analyses::entity_ownership::EntityOwnershipRow;
use crate::analyses::hotspots::HotspotRow;
use crate::analyses::knowledge_islands::KnowledgeIslandRow;
use crate::analyses::summary::SummaryRow;
use crate::{CodeLoreError, Result};

const TEMPLATE: &str = include_str!("spa/template.html");
const WIDGETS_JS: &str = include_str!("spa/widgets.js");
const ECHARTS_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/echarts.min.js"));
const D3_HIERARCHY_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/d3-hierarchy.min.js"));
const ALPINE_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/alpine.min.js"));
const ALPINE_PERSIST_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/alpine-persist.min.js"));
// CSS lives as a regular source file (not a `build.rs`-fetched asset) —
// it's the precompiled output of `just spa-css-rebuild`, checked into
// the repo. See `spa/tailwind-src/README.md` for the rebuild workflow.
const TAILWIND_DAISY_CSS: &str = include_str!("spa/tailwind.daisyui.min.css");

/// Composite of all per-widget data the SPA dashboard renders.
/// Each field carries the rows for one widget; widgets that opt out
/// via `skip_serializing_if` are simply absent from the payload.
/// Adding a field here + updating the JSON consumer in `widgets.js`
/// is the canonical extension point for a new widget.
#[derive(Debug, Default, Serialize)]
pub struct SpaDashboard {
    pub hotspots: Vec<HotspotRow>,
    /// Per-file code-health rows (drill-down details + median KPI).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_health: Vec<CodeHealthRow>,
    /// Aggregate metric rows (KPI tile values: commits, authors, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<SummaryRow>,
    /// Coupling pairs (sankey widget + per-file partner list in the
    /// detail drawer).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coupling: Vec<CouplingRow>,
    /// Knowledge-island rows — `CodeLore`'s auto-detected ex-developer
    /// signal. Empty when no contributors have departed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_islands: Vec<KnowledgeIslandRow>,
    /// Entity-ownership rows feeding the knowledge-map widget (W7).
    /// Each row is one (path, author) tuple; the JS picks the primary
    /// author per path (max added `LoC`) and palette-colors the circles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_ownership: Vec<EntityOwnershipRow>,
    /// Function-level entries feeding the X-Ray sunburst widget (W8).
    /// Each row is one function with its cognitive complexity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub xray: Vec<XRayEntry>,
    /// Per-day commit counts feeding the calendar-heatmap widget (W10).
    /// Each row is `(date YYYY-MM-DD, count)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub daily_commits: Vec<DailyCommit>,
    /// Per-month hotspot snapshots feeding the trends widget (W9).
    /// Each row is `(month YYYY-MM-01, path, hotspot_score)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trends: Vec<TrendPoint>,
    /// Repo-relative MI band counts (`high` / `moderate` / `low` / `unknown`)
    /// for the KPI tile. Derived from `hotspots[*].mi_rank` at dispatch
    /// time via [`crate::analyses::mi::MiRollup::from_hotspots`]. `None`
    /// when no hotspots are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mi_rollup: Option<crate::analyses::mi::MiRollup>,
    /// Density of the behavioral coupling graph in `[0, 1]` — the ratio
    /// of Fisher-significant pairs to the maximum possible pairs in the
    /// `revs >= min_revs` candidate node set. Computed via
    /// [`crate::analyses::coupling::density`] over the same node universe
    /// `run_coupling` uses. `None` when coupling analysis was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupling_density: Option<f64>,
    /// Per-file clone-group counts feeding the hotspot circle-pack's
    /// "Clones" colour mode. One row per path that appears in at least
    /// one clone family. Files with zero clone groups are omitted from
    /// the payload (the widget falls back to neutral grey for them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clones: Vec<CloneSummary>,
    /// Resolved import edges feeding the architecture force-graph
    /// widget. One row per resolved import from the imports table.
    /// Empty until the resolver covers the repo's language mix
    /// (Rust + Python + JS/TS today).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<ImportEdgeRow>,
    /// Per-commit Kamei JIT-SDP feature vector for the Delivery Risk
    /// Sparkline widget. One row per commit in the last-N (capped at
    /// 30) chronological window. Surfaces the raw Kamei 14-feature
    /// signal — la/ld (size), nf (spread), ndev (concurrency), exp
    /// (author experience), entropy (file distribution), fix (bug-
    /// fix-ness) — so the SPA can compute a composite risk score per
    /// dimension and explain *which* dimension dominates each
    /// commit's risk. Beyond-CodeScene differentiator (`CodeScene`
    /// reports an opaque score; `CodeLore` reports the peer-reviewed
    /// dimensions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kamei_risk: Vec<KameiRiskRow>,
}

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

/// Pull every resolved import edge from the `imports` table. Only
/// resolved edges (where `target_path` is non-NULL) participate so
/// the SPA graph reflects the dependency surface `CodeLore` can
/// actually visualise.
///
/// # Errors
///
/// Propagates `DuckDB` prepare / query errors as
/// [`CodeLoreError::Output`].
pub fn run_imports_for_arch_graph(db: &crate::facts::FactsDb) -> Result<Vec<ImportEdgeRow>> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT src_path, target_path FROM imports \
             WHERE target_path IS NOT NULL \
             ORDER BY src_path ASC, target_path ASC",
        )
        .map_err(|e| CodeLoreError::Output(format!("arch-imports prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ImportEdgeRow {
                src_path: r.get(0)?,
                target_path: r.get(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Output(format!("arch-imports query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Output(format!("arch-imports collect: {e}")))
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

/// Render the SPA HTML and write it to `w`. The HTML is fully
/// self-contained: opening it locally in any browser renders the
/// dashboard offline.
pub fn write_spa<W: Write>(
    dash: &SpaDashboard,
    title: &str,
    repo_path: &str,
    generated_at: &str,
    w: &mut W,
) -> Result<()> {
    let data_json = serde_json::to_string(dash)
        .map_err(|e| CodeLoreError::Output(format!("spa json serialize: {e}")))?;
    let data_json_safe = data_json.replace("</", "<\\/");

    // Single-pass templating via `output::template::substitute`. This
    // matters more here than in `output::html` because the SPA payload
    // includes the ~1.1 MB `echarts.min.js` blob plus widget glue plus
    // the per-analysis JSON data block. The chained-`.replace()` form
    // copied that multi-megabyte intermediate 7 times per emit; one
    // pass + a capacity hint cuts the allocation traffic ~7×.
    let title_escaped = escape_html(title);
    let repo_path_escaped = escape_html(repo_path);
    let generated_at_escaped = escape_html(generated_at);
    let html = crate::output::template::substitute(
        TEMPLATE,
        &[
            ("{{TITLE}}", &title_escaped),
            ("{{REPO_PATH}}", &repo_path_escaped),
            ("{{GENERATED_AT}}", &generated_at_escaped),
            ("{{DATA_JSON}}", &data_json_safe),
            ("{{ECHARTS_JS}}", ECHARTS_JS),
            ("{{D3_HIERARCHY_JS}}", D3_HIERARCHY_JS),
            ("{{ALPINE_JS}}", ALPINE_JS),
            ("{{ALPINE_PERSIST_JS}}", ALPINE_PERSIST_JS),
            ("{{TAILWIND_DAISY_CSS}}", TAILWIND_DAISY_CSS),
            ("{{WIDGETS_JS}}", WIDGETS_JS),
        ],
    );

    w.write_all(html.as_bytes())
        .map_err(|e| CodeLoreError::Output(format!("spa write: {e}")))?;
    Ok(())
}

/// Aggregate function-level cognitive complexity per (path, function)
/// from the `complexity_metrics` table for the X-Ray sunburst (W8).
/// Returns at most `limit` rows ordered by `cognitive DESC`, since
/// the sunburst becomes unreadable past a few hundred functions and
/// the JSON payload would blow up on monorepos otherwise.
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
        .map_err(|e| CodeLoreError::Output(format!("xray prepare: {e}")))?;
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
        .map_err(|e| CodeLoreError::Output(format!("xray query: {e}")))?;
    let out: std::result::Result<Vec<_>, _> = rows.collect();
    out.map_err(|e| CodeLoreError::Output(format!("xray collect: {e}")))
}

/// Per-file clone-group counts from the `clones` table. Empty result
/// when no clone groups exist (small repo, no Tier-1 sources, or
/// `min_clone_node_count` filtered everything out at ingest time).
/// One row per path that appears in ≥ 1 clone family — files with
/// zero clone groups are dropped so the payload stays compact.
///
/// # Errors
/// Returns [`CodeLoreError::Output`] on any `DuckDB` failure.
pub fn run_clone_summary(db: &crate::facts::FactsDb) -> Result<Vec<CloneSummary>> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT path, COUNT(DISTINCT clone_group_id)::UINTEGER AS groups
             FROM clones
             GROUP BY path
             ORDER BY groups DESC, path ASC",
        )
        .map_err(|e| CodeLoreError::Output(format!("clone_summary prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CloneSummary {
                path: r.get(0)?,
                groups: r.get(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Output(format!("clone_summary query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Output(format!("clone_summary collect: {e}")))
}

/// Build a per-(month, path) trend series restricted to `paths` for
/// the trends multi-line widget (W9). The score per (month, path) is
/// the count of revisions that touched the path during the month.
/// Empty `paths` returns an empty Vec — the widget renders nothing.
pub fn run_trends(db: &crate::facts::FactsDb, paths: &[String]) -> Result<Vec<TrendPoint>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
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
         INNER JOIN changes ch ON ch.rev = c.rev
         INNER JOIN paths USING (path)
         GROUP BY month, ch.path
         ORDER BY month ASC, ch.path ASC"
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Output(format!("trends prepare: {e}")))?;
    let params: Vec<&dyn duckdb::ToSql> = paths.iter().map(|p| p as &dyn duckdb::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |r| {
            Ok(TrendPoint {
                month: r.get(0)?,
                path: r.get(1)?,
                hotspot_score: r.get(2)?,
            })
        })
        .map_err(|e| CodeLoreError::Output(format!("trends query: {e}")))?;
    let out: std::result::Result<Vec<_>, _> = rows.collect();
    out.map_err(|e| CodeLoreError::Output(format!("trends collect: {e}")))
}

/// Per-day commit counts for the calendar heatmap (W10). Returns one
/// row per day with at least one commit, sorted by date ascending.
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
        .map_err(|e| CodeLoreError::Output(format!("daily_commits prepare: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DailyCommit {
                date: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map_err(|e| CodeLoreError::Output(format!("daily_commits query: {e}")))?;
    let out: std::result::Result<Vec<_>, _> = rows.collect();
    out.map_err(|e| CodeLoreError::Output(format!("daily_commits collect: {e}")))
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
/// [`CodeLoreError::Output`].
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
        .map_err(|e| CodeLoreError::Output(format!("kamei_risk prepare: {e}")))?;
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
        .map_err(|e| CodeLoreError::Output(format!("kamei_risk query: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Output(format!("kamei_risk collect: {e}")))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hotspots() -> Vec<HotspotRow> {
        vec![
            HotspotRow {
                path: "src/main.rs".into(),
                revisions: 12,
                cognitive: 42.0,
                code_health: 78.0,
                hotspot_score: 5.5,
                mi: Some(54.0),
                // Bottom quartile → Low band when MiRollup runs over this set.
                mi_rank: Some(0.0),
                ai_pct: None,
            },
            HotspotRow {
                path: "src/lib/util.rs".into(),
                revisions: 8,
                cognitive: 28.0,
                code_health: 88.0,
                hotspot_score: 2.1,
                mi: Some(82.5),
                // Top quartile → High band.
                mi_rank: Some(1.0),
                ai_pct: None,
            },
        ]
    }

    #[test]
    fn write_spa_embeds_all_expected_markers() {
        let dash = SpaDashboard {
            hotspots: sample_hotspots(),
            ..SpaDashboard::default()
        };
        let mut buf = Vec::new();
        write_spa(
            &dash,
            "CodeLore Dashboard",
            "/tmp/example-repo",
            "2026-06-11 00:00:00 UTC",
            &mut buf,
        )
        .expect("write_spa");
        let html = String::from_utf8(buf).expect("utf8");

        assert!(
            html.contains("CodeLore Dashboard"),
            "title missing from output",
        );
        assert!(
            html.contains("/tmp/example-repo"),
            "repo path missing from output",
        );
        assert!(
            html.contains("widget-hotspot-circle-pack"),
            "hotspot circle-pack widget mount point missing",
        );
        assert!(
            html.contains("widget-hotspot-table"),
            "hotspot table widget mount point missing",
        );
        assert!(
            html.contains("widget-kpi-tiles"),
            "KPI tiles widget mount point missing",
        );
        assert!(
            html.contains("widget-knowledge-islands"),
            "knowledge islands widget mount point missing",
        );
        assert!(
            html.contains("widget-coupling-sankey"),
            "change-coupling sankey widget mount point missing",
        );
        assert!(
            html.contains("file-detail-drawer"),
            "file detail drawer mount point missing",
        );
        assert!(
            html.contains("src/main.rs"),
            "embedded hotspot row missing from JSON block",
        );
        // ECharts global must be in the embedded JS payload.
        assert!(html.contains("echarts"), "ECharts payload missing");
        // d3-hierarchy global must also be there.
        assert!(html.contains("d3"), "d3-hierarchy payload missing");
        // The widget render closure must be present.
        assert!(
            html.contains("renderHotspotCirclePack"),
            "widget render fn missing",
        );
    }

    #[test]
    fn write_spa_escapes_xss_in_metadata() {
        let dash = SpaDashboard::default();
        let mut buf = Vec::new();
        write_spa(
            &dash,
            "<script>alert(1)</script>",
            "</title><script>alert(2)</script>",
            "2026-06-11",
            &mut buf,
        )
        .expect("write_spa");
        let html = String::from_utf8(buf).expect("utf8");

        // The literal injection strings must NOT appear unescaped.
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "title injection survived: HTML escape broken",
        );
        assert!(
            !html.contains("<script>alert(2)</script>"),
            "repo-path injection survived: HTML escape broken",
        );
        // Their escaped forms SHOULD appear.
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "expected escaped title",
        );
    }

    #[test]
    fn write_spa_escapes_script_terminator_in_json() {
        let mut rows = sample_hotspots();
        // Cram a script-terminator into a row's path.
        rows[0].path = "src/</script><script>alert('xss')</script>.rs".into();
        let dash = SpaDashboard {
            hotspots: rows,
            ..SpaDashboard::default()
        };

        let mut buf = Vec::new();
        write_spa(&dash, "x", "y", "z", &mut buf).expect("write_spa");
        let html = String::from_utf8(buf).expect("utf8");

        // The raw </script> in JSON would break out of the script block.
        // After the `</` → `<\/` rewrite, it must be the escaped form
        // inside the JSON payload.
        assert!(
            html.contains(r"<\/script>"),
            "expected escaped script terminator inside JSON block",
        );
        // The unescaped form COULD legitimately appear in the template
        // (the `</script>` that closes the embedded data block, for
        // instance). What's NOT allowed is the unescaped form INSIDE
        // the JSON data block. We check by ensuring at least one
        // escaped occurrence exists for every injection-style attempt.
    }
}
