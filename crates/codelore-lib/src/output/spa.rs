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

use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::coupling::CouplingRow;
use crate::analyses::dashboard::{
    CloneSummary, DailyCommit, ImportEdgeRow, KameiRiskRow, TrendPoint, XRayEntry,
};
use crate::analyses::entity_ownership::EntityOwnershipRow;
use crate::analyses::hotspots::HotspotRow;
use crate::analyses::knowledge_islands::KnowledgeIslandRow;
use crate::analyses::modularity_violations::ModularityViolationRow;
use crate::analyses::summary::SummaryRow;
use crate::analyses::unstable_interface::UnstableInterfaceRow;
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
    /// Modularity violations — Fisher-significant co-change pairs with
    /// NO structural import edge. Overlaid on the architecture graph as
    /// "temporal-only" edges: the implicit/hidden coupling that an
    /// import-only graph cannot show (the structure×history fusion).
    /// Empty when every co-change pair also imports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modularity_violations: Vec<ModularityViolationRow>,
    /// Unstable interfaces — heavily-imported files that change often
    /// and co-change with their dependents. Highlighted as warning
    /// nodes on the architecture graph. Empty when no interface is
    /// both widely-imported and churning.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unstable_interface: Vec<UnstableInterfaceRow>,
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
    /// Effective thresholds for THIS run, snapshotted at dispatch.
    /// Surfaced into the SPA's `data.options` block so per-metric
    /// tooltips can interpolate `${min_shared_revs}` /
    /// `${fisher_significance}` / `${min_revs}` and show the actual
    /// values used — V5: tooltips claimed to surface formula
    /// provenance but referenced parameter NAMES verbatim, hiding the
    /// effective gates. `Default` so callers (tests, step-summary)
    /// continue to use `..Default::default()` unchanged; production
    /// dispatch in `build_spa_dashboard` populates from real `Options`.
    #[serde(default)]
    pub options: SpaOptionsSnapshot,
}

/// Snapshot of the analysis-threshold subset of `Options` that drive
/// formula provenance in the SPA. Kept narrow (six numeric fields) so
/// the JSON payload doesn't balloon; the broader `Options` carries
/// path filters, AI-detection toggles, output routing flags, etc., which
/// have no place in a UI tooltip. Fields are public for `Default::default`
/// to populate them with reasonable code-maat parity values when a test
/// constructs `SpaDashboard` without going through `build_spa_dashboard`.
#[derive(Debug, Clone, Serialize)]
pub struct SpaOptionsSnapshot {
    pub min_revs: u32,
    pub min_shared_revs: u32,
    pub min_coupling_pct: u8,
    pub max_coupling_pct: u8,
    pub max_changeset_size: u32,
    pub fisher_significance: f64,
}

impl Default for SpaOptionsSnapshot {
    fn default() -> Self {
        // Mirrors `Options::default()` — code-maat parity baseline.
        // Kept in sync via `default_options_match_code_maat_thresholds`
        // in tests/types_test.rs (asserts the source-of-truth values
        // haven't drifted).
        Self {
            min_revs: 5,
            min_shared_revs: 5,
            min_coupling_pct: 30,
            max_coupling_pct: 100,
            max_changeset_size: 30,
            fisher_significance: 0.05,
        }
    }
}

impl SpaOptionsSnapshot {
    /// Snapshot the threshold subset of [`crate::Options`].
    #[must_use]
    pub fn from_options(opts: &crate::Options) -> Self {
        Self {
            min_revs: opts.min_revs,
            min_shared_revs: opts.min_shared_revs,
            min_coupling_pct: opts.min_coupling_pct,
            max_coupling_pct: opts.max_coupling_pct,
            max_changeset_size: opts.max_changeset_size,
            fisher_significance: opts.fisher_significance,
        }
    }
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
