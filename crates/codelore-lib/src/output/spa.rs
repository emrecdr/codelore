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
//! See `docs/ui-roadmap.md` for the v0.4.x widget plan and the
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

use crate::analyses::hotspots::HotspotRow;
use crate::{CodeLoreError, Result};

const TEMPLATE: &str = include_str!("spa/template.html");
const WIDGETS_JS: &str = include_str!("spa/widgets.js");
const ECHARTS_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/echarts.min.js"));
const D3_HIERARCHY_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/d3-hierarchy.min.js"));

/// Composite of all per-widget data the SPA dashboard renders. For
/// v0.4.0 only `hotspots` is wired; subsequent commits in the v0.4.x
/// series add `coupling`, `code_health`, `knowledge_islands`, and the
/// trends timeseries as separate fields. Adding a field here +
/// updating the JSON consumer in `widgets.js` is the v0.4.x growth
/// vector.
#[derive(Debug, Default, Serialize)]
pub struct SpaDashboard {
    pub hotspots: Vec<HotspotRow>,
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

    let html = TEMPLATE
        .replace("{{TITLE}}", &escape_html(title))
        .replace("{{REPO_PATH}}", &escape_html(repo_path))
        .replace("{{GENERATED_AT}}", &escape_html(generated_at))
        .replace("{{DATA_JSON}}", &data_json_safe)
        .replace("{{ECHARTS_JS}}", ECHARTS_JS)
        .replace("{{D3_HIERARCHY_JS}}", D3_HIERARCHY_JS)
        .replace("{{WIDGETS_JS}}", WIDGETS_JS);

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
            },
            HotspotRow {
                path: "src/lib/util.rs".into(),
                revisions: 8,
                cognitive: 28.0,
                code_health: 88.0,
                hotspot_score: 2.1,
            },
        ]
    }

    #[test]
    fn write_spa_embeds_all_expected_markers() {
        let dash = SpaDashboard {
            hotspots: sample_hotspots(),
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
            "hotspot widget mount point missing",
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
        let dash = SpaDashboard { hotspots: rows };

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
