//! End-to-end integration test for the `--format spa` dashboard
//! emitter. Builds a real `DuckDB` fact store from a fixture repo, runs
//! the analyses the SPA consumes, serialises through `write_spa`, and
//! asserts the resulting HTML contains the widget markers and the
//! embedded JSON parses with the expected shape.
//!
//! Only compiled when the `spa` Cargo feature is enabled — same gate
//! the emitter module sits behind.

#![cfg(feature = "spa")]

use codelore_lib::Options;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::analyses::knowledge_islands::run_knowledge_islands;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::spa::{SpaDashboard, write_spa};
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::differential_repo;

/// Smoke-emit the SPA from the canonical 50-commit differential
/// fixture and assert the structural invariants — widget markers
/// present, embedded JSON shape correct, hotspot rows actually made
/// it through the pipeline. This catches regressions where one of
/// the run_* calls silently returns empty, where the JSON shape
/// drifts, or where a widget mount point disappears from the
/// template.
#[allow(clippy::too_many_lines)]
#[test]
fn spa_emits_full_dashboard_from_differential_fixture() {
    let repo_fixture = differential_repo::build();
    let repo_path = repo_fixture.dir.path().to_path_buf();
    let repo = GixRepo::open(&repo_path).expect("open gix repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");

    let opts = Options {
        repo_path: repo_path.clone(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code_health");
    // Coupling can yield 0 rows on the 50-commit fixture if Fisher
    // significance isn't reached — that's fine; the test asserts the
    // dashboard structure, not that every analysis produced rows.
    let coupling = run_coupling(&db, &opts).unwrap_or_default();
    let knowledge_islands = run_knowledge_islands(&db, &opts).unwrap_or_default();

    assert!(
        !hotspots.is_empty(),
        "the differential fixture should produce ≥1 hotspot row"
    );
    assert!(
        !summary.is_empty(),
        "the differential fixture should produce ≥1 summary row"
    );

    let dash = SpaDashboard {
        hotspots: hotspots.clone(),
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };

    let mut buf = Vec::new();
    write_spa(
        &dash,
        "CodeLore Dashboard",
        &repo_path.display().to_string(),
        "2026-06-11 00:00:00 UTC",
        &mut buf,
    )
    .expect("write_spa");

    let html = String::from_utf8(buf).expect("utf8 html");

    // Widget mount points (template structural markers).
    for marker in [
        "widget-kpi-tiles",
        "widget-knowledge-islands",
        "widget-hotspot-circle-pack",
        "widget-hotspot-table",
        "widget-coupling-sankey",
        "file-detail-drawer",
    ] {
        assert!(
            html.contains(marker),
            "widget mount point `{marker}` missing from emitted HTML",
        );
    }

    // Widget render closures (widgets.js structural markers).
    for marker in [
        "renderKpiTiles",
        "renderHotspotCirclePack",
        "renderHotspotTable",
        "renderCouplingSankey",
        "renderKnowledgeIslands",
        "showFileDetailDrawer",
    ] {
        assert!(
            html.contains(marker),
            "widget render closure `{marker}` missing from emitted HTML",
        );
    }

    // Embedded JS deps — both libs need to make it into the bundle.
    assert!(
        html.contains("echarts"),
        "ECharts payload missing from emitted HTML",
    );
    assert!(
        html.contains("d3-hierarchy") || html.contains("d3.pack"),
        "d3-hierarchy payload missing from emitted HTML",
    );

    // Parse the embedded JSON data block and verify the shape.
    let data = extract_data_json(&html).expect("parse data block");
    let hotspots_arr = data
        .get("hotspots")
        .and_then(|v| v.as_array())
        .expect("hotspots array");
    assert_eq!(
        hotspots_arr.len(),
        hotspots.len(),
        "JSON-encoded hotspot row count must match the input vector",
    );
    let summary_arr = data
        .get("summary")
        .and_then(|v| v.as_array())
        .expect("summary array");
    assert!(
        !summary_arr.is_empty(),
        "summary array should be non-empty given the differential fixture",
    );

    // Hotspot rows should have the expected shape.
    let first_hot = &hotspots_arr[0];
    for field in [
        "path",
        "revisions",
        "cognitive",
        "code_health",
        "hotspot_score",
    ] {
        assert!(
            first_hot.get(field).is_some(),
            "hotspot row missing field `{field}` after JSON round-trip",
        );
    }

    // Sanity bound on the total HTML size — the embedded ECharts is
    // ~1.1 MB, plus ~14 KB d3-hierarchy plus our payload. Anything
    // below 800 KB means an asset failed to embed; anything above
    // 50 MB suggests we shipped uncompressed data we shouldn't have.
    let bytes = html.len();
    assert!(
        (800_000..50_000_000).contains(&bytes),
        "emitted HTML size {bytes} bytes is outside the sane window \
         [800 KB, 50 MB] — an asset may have failed to embed",
    );
}

/// Walk the HTML, find the `<script type="application/json"
/// id="codelore-data">…</script>` block, undo the `</` → `<\/`
/// XSS-escape, and parse it.
fn extract_data_json(html: &str) -> Option<serde_json::Value> {
    let start_tag = "<script type=\"application/json\" id=\"codelore-data\">";
    let end_tag = "</script>";
    let start = html.find(start_tag)? + start_tag.len();
    let rest = &html[start..];
    let end = rest.find(end_tag)?;
    let raw = &rest[..end];
    let restored = raw.replace(r"<\/", "</");
    serde_json::from_str(&restored).ok()
}
