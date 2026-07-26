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
use codelore_lib::analyses::architecture_roles::ArchitectureRoleRow;
use codelore_lib::analyses::architecture_trend::ArchitectureTrendRow;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::analyses::coupling::{CouplingRow, run_coupling};
use codelore_lib::analyses::dashboard::ImportEdgeRow;
use codelore_lib::analyses::effort_exposure::EffortExposureRow;
use codelore_lib::analyses::factors::FactorTile;
use codelore_lib::analyses::function_xray::FunctionXrayRow;
use codelore_lib::analyses::health_trend::HealthTrendRow;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::analyses::knowledge_islands::run_knowledge_islands;
use codelore_lib::analyses::modularity_violations::ModularityViolationRow;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::analyses::unstable_interface::UnstableInterfaceRow;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::spa::{FileFunctionXray, SpaDashboard, write_spa};
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
        "widget-knowledge-surfaces",
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
        "renderKnowledgeSurfaces",
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

    // Alpine.js + persist plugin — interactivity layer wiring. Tested
    // via known minified-output substrings (Alpine ships under an
    // IIFE that exposes `Alpine.start`; persist plugin registers via
    // `Alpine.plugin`).
    assert!(
        html.contains("Alpine.start") || html.contains("alpinejs"),
        "Alpine.js payload missing from emitted HTML",
    );
    assert!(
        html.contains("Alpine.plugin") || html.contains("persist"),
        "Alpine persist plugin missing from emitted HTML",
    );

    // Tailwind v4 + DaisyUI 5 CSS asset — until the per-widget
    // conversion lands the asset is a stub, but `{{TAILWIND_DAISY_CSS}}`
    // must still substitute (i.e. the literal placeholder must NOT
    // appear in the final HTML).
    assert!(
        !html.contains("{{TAILWIND_DAISY_CSS}}"),
        "TAILWIND_DAISY_CSS placeholder leaked into emitted HTML — substitution wiring is broken",
    );
    assert!(
        !html.contains("{{ALPINE_JS}}"),
        "ALPINE_JS placeholder leaked into emitted HTML",
    );
    assert!(
        !html.contains("{{ALPINE_PERSIST_JS}}"),
        "ALPINE_PERSIST_JS placeholder leaked into emitted HTML",
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

    // Knowledge surface fields — present (possibly empty) after the round-trip.
    // The differential fixture has no complexity metrics at HEAD so these
    // will be empty arrays; what matters is that the keys exist in the JSON
    // (i.e. serialisation wiring is intact).
    for key in ["code_familiarity", "team_composition", "coordination_needs"] {
        // If the Vec is empty, serde skips the key (skip_serializing_if = "Vec::is_empty").
        // The assertion therefore only fires when a non-empty result was produced — which
        // is correct: we can't assert presence of an empty field that was serialised away.
        // We assert the field is EITHER absent (Vec was empty) OR is an array.
        if let Some(v) = data.get(key) {
            assert!(
                v.is_array(),
                "dashboard JSON field `{key}` must be an array when present",
            );
        }
    }

    // Hotspot rows should have the expected shape.
    let first_hot = &hotspots_arr[0];
    for field in [
        "path",
        "revisions",
        "cognitive",
        "cognitive_health",
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

/// The structure×history fusion overlay (`modularity-violations` +
/// `unstable-interface`) must round-trip into the embedded SPA JSON,
/// and the architecture-graph widget must be wired to consume all
/// three data arrays. Guards against the bundle field or the widget
/// call silently dropping the fusion data.
#[test]
#[allow(clippy::too_many_lines)] // one cohesive round-trip contract over many fields
fn spa_embeds_fusion_overlay_data() {
    use codelore_lib::analyses::health_trend::HealthTransitionRow;
    let dash = SpaDashboard {
        modularity_violations: vec![ModularityViolationRow {
            entity_a: "src/alpha.rs".into(),
            entity_b: "src/beta.rs".into(),
            shared: 9,
            degree: 81.5,
            fisher_p: 0.001,
        }],
        unstable_interface: vec![UnstableInterfaceRow {
            path: "src/hub.rs".into(),
            fan_in: 7,
            revisions: 30,
            coupled_dependents: 4,
            instability_score: 120.0,
        }],
        architecture_roles: vec![ArchitectureRoleRow {
            path: "src/hub.rs".into(),
            role: "core".into(),
            vfi: 12,
            vfo: 8,
            in_cycle: true,
            level: 2,
            reach_pct: 40.0,
        }],
        architecture_trend: vec![ArchitectureTrendRow {
            date: "2026-01-01".into(),
            rev: "abc123def456".into(),
            files: 10,
            propagation_cost: 0.25,
            cycle_count: 1,
            largest_cycle: 3,
        }],
        health_trend: vec![HealthTrendRow {
            date: "2026-01-01".into(),
            rev: "abc123def456".into(),
            files: 10,
            arch_health: 85.0,
            code_health: 72.0,
            combined_health: 78.5,
            arch_band: "green".into(),
            code_band: "green".into(),
            combined_band: "green".into(),
        }],
        ..SpaDashboard::default()
    };

    let mut buf = Vec::new();
    write_spa(
        &dash,
        "CodeLore Dashboard",
        "/tmp/x",
        "2026-06-26 00:00:00 UTC",
        &mut buf,
    )
    .expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8 html");

    // The arch-graph widget must receive all four data arrays.
    assert!(
        html.contains(
            "renderArchGraph(data.imports || [], data.modularity_violations || [], data.unstable_interface || [], data.architecture_roles || [])"
        ),
        "arch-graph widget must be wired to the fusion + roles data arrays",
    );

    let data = extract_data_json(&html).expect("parse data block");
    let mv = data
        .get("modularity_violations")
        .and_then(|v| v.as_array())
        .expect("modularity_violations array");
    assert_eq!(mv.len(), 1, "one modularity-violation row expected");
    assert_eq!(
        mv[0].get("entity_a").and_then(serde_json::Value::as_str),
        Some("src/alpha.rs"),
    );
    let ui = data
        .get("unstable_interface")
        .and_then(|v| v.as_array())
        .expect("unstable_interface array");
    assert_eq!(ui.len(), 1, "one unstable-interface row expected");
    assert_eq!(
        ui[0].get("path").and_then(serde_json::Value::as_str),
        Some("src/hub.rs"),
    );
    assert_eq!(
        ui[0]
            .get("coupled_dependents")
            .and_then(serde_json::Value::as_u64),
        Some(4),
    );
    let ar = data
        .get("architecture_roles")
        .and_then(|v| v.as_array())
        .expect("architecture_roles array");
    assert_eq!(ar.len(), 1, "one architecture-role row expected");
    assert_eq!(
        ar[0].get("role").and_then(serde_json::Value::as_str),
        Some("core"),
    );
    assert_eq!(
        ar[0].get("level").and_then(serde_json::Value::as_u64),
        Some(2),
    );

    // The arch-trend line chart must be wired + its data round-trip.
    assert!(
        html.contains("renderArchTrend(data.architecture_trend || [])"),
        "arch-trend widget must be wired to the architecture_trend array",
    );
    let at = data
        .get("architecture_trend")
        .and_then(|v| v.as_array())
        .expect("architecture_trend array");
    assert_eq!(at.len(), 1, "one architecture-trend point expected");
    assert_eq!(
        at[0].get("cycle_count").and_then(serde_json::Value::as_u64),
        Some(1),
    );
    assert!(
        (at[0]
            .get("propagation_cost")
            .and_then(serde_json::Value::as_f64)
            .expect("propagation_cost")
            - 0.25)
            .abs()
            < 1e-9,
    );

    // The health-trend timeline must round-trip into the embedded JSON.
    let ht = data
        .get("health_trend")
        .and_then(|v| v.as_array())
        .expect("health_trend array");
    assert_eq!(ht.len(), 1, "one health-trend point expected");
    assert_eq!(
        ht[0].get("arch_band").and_then(serde_json::Value::as_str),
        Some("green"),
    );
    let combined = ht[0]
        .get("combined_health")
        .and_then(serde_json::Value::as_f64)
        .expect("combined_health f64");
    assert!((combined - 78.5).abs() < 1e-9);

    // When health_transitions is non-empty it must appear as a JSON array
    // with the expected direction field.  The dashboard fixture above has no
    // transitions set, so the key is absent (skip_serializing_if).  Wire a
    // minimal dash with one transition and verify the round-trip.
    let dash_with_transitions = SpaDashboard {
        health_transitions: vec![HealthTransitionRow {
            path: "src/hot.rs".into(),
            date: "2026-01-01".into(),
            from_band: "yellow".into(),
            to_band: "red".into(),
            direction: "regressed".into(),
        }],
        ..SpaDashboard::default()
    };
    let mut buf2 = Vec::new();
    write_spa(
        &dash_with_transitions,
        "Transitions Test",
        "/tmp/y",
        "2026-06-26 00:00:00 UTC",
        &mut buf2,
    )
    .expect("write_spa transitions");
    let html2 = String::from_utf8(buf2).expect("utf8 html2");
    let data2 = extract_data_json(&html2).expect("parse data2 block");
    let tr = data2
        .get("health_transitions")
        .and_then(|v| v.as_array())
        .expect("health_transitions array present when non-empty");
    assert_eq!(tr.len(), 1, "one transition expected");
    assert_eq!(
        tr[0].get("direction").and_then(serde_json::Value::as_str),
        Some("regressed"),
    );
    assert_eq!(
        tr[0].get("from_band").and_then(serde_json::Value::as_str),
        Some("yellow"),
    );

    // When factors is non-empty it must appear as a JSON array and each
    // tile must carry the required fields.
    let dash_with_factors = SpaDashboard {
        factors: vec![FactorTile {
            name: "Code".into(),
            headline: Some(72.5),
            band: "green".into(),
            series: vec![65.0, 68.0, 72.5],
            attention: false,
            detail: "Code health 72.5 (green)".into(),
            numbers: Vec::new(),
        }],
        ..SpaDashboard::default()
    };
    let mut buf3 = Vec::new();
    write_spa(
        &dash_with_factors,
        "Factors Test",
        "/tmp/z",
        "2026-06-26 00:00:00 UTC",
        &mut buf3,
    )
    .expect("write_spa factors");
    let html3 = String::from_utf8(buf3).expect("utf8 html3");
    let data3 = extract_data_json(&html3).expect("parse data3 block");
    let fa = data3
        .get("factors")
        .and_then(|v| v.as_array())
        .expect("factors array present when non-empty");
    assert_eq!(fa.len(), 1, "one factor tile expected");
    assert_eq!(
        fa[0].get("name").and_then(serde_json::Value::as_str),
        Some("Code"),
    );
    assert_eq!(
        fa[0].get("headline").and_then(serde_json::Value::as_f64),
        Some(72.5),
    );
    assert_eq!(
        fa[0].get("band").and_then(serde_json::Value::as_str),
        Some("green"),
    );
    assert_eq!(
        fa[0].get("attention").and_then(serde_json::Value::as_bool),
        Some(false),
    );
    let series = fa[0]
        .get("series")
        .and_then(|v| v.as_array())
        .expect("series array");
    assert_eq!(series.len(), 3);
    // `numbers` is serde-skipped when empty — confirm it is absent from JSON
    // for a normal headline tile.
    assert!(
        fa[0].get("numbers").is_none(),
        "numbers absent from JSON when empty"
    );

    // Delivery tile: headline=None, numbers carries three proxy pairs.
    // The tile must round-trip correctly: headline absent from JSON,
    // numbers array present with the three entries.
    let dash_delivery = SpaDashboard {
        factors: vec![FactorTile {
            name: "Delivery".into(),
            headline: None,
            band: "green".into(),
            series: Vec::new(),
            attention: false,
            detail: "Git-only proxies".into(),
            numbers: vec![
                ("rework %".to_string(), "6.2".to_string()),
                ("branch p75 h".to_string(), "18".to_string()),
                ("cadence median d".to_string(), "14".to_string()),
            ],
        }],
        ..SpaDashboard::default()
    };
    let mut buf_del = Vec::new();
    write_spa(
        &dash_delivery,
        "Delivery Tile Test",
        "/tmp/z",
        "2026-06-26 00:00:00 UTC",
        &mut buf_del,
    )
    .expect("write_spa delivery tile");
    let html_del = String::from_utf8(buf_del).expect("utf8 html_del");
    let data_del = extract_data_json(&html_del).expect("parse data_del");
    let fd = data_del
        .get("factors")
        .and_then(|v| v.as_array())
        .expect("factors array for delivery tile");
    assert_eq!(fd.len(), 1, "one delivery tile expected");
    assert_eq!(
        fd[0].get("name").and_then(serde_json::Value::as_str),
        Some("Delivery"),
    );
    // headline is None → must be absent or null in JSON
    assert!(
        fd[0].get("headline").is_none_or(serde_json::Value::is_null),
        "headline must be null/absent for delivery tile"
    );
    assert_eq!(
        fd[0].get("band").and_then(serde_json::Value::as_str),
        Some("green"),
    );
    // numbers must be present and carry three entries
    let nums = fd[0]
        .get("numbers")
        .and_then(|v| v.as_array())
        .expect("numbers array present for delivery tile");
    assert_eq!(nums.len(), 3, "three numbers on delivery tile");
    assert_eq!(
        nums[0].as_array().and_then(|p| p[0].as_str()),
        Some("rework %"),
    );
    assert_eq!(nums[0].as_array().and_then(|p| p[1].as_str()), Some("6.2"),);

    // When effort_exposure is non-empty it must appear as a JSON array with
    // the required per-row fields. Build a minimal SpaDashboard with one
    // synthetic row and assert the round-trip carries all fields.
    let dash_with_ee = SpaDashboard {
        effort_exposure: vec![EffortExposureRow {
            band: "red".into(),
            files: 3,
            loc_share_pct: 25.0,
            commit_share_pct: 40.0,
            churn_share_pct: 35.0,
            commit_share_ci_low: 0.28,
            commit_share_ci_high: 0.54,
        }],
        ..SpaDashboard::default()
    };
    let mut buf_ee = Vec::new();
    write_spa(
        &dash_with_ee,
        "EE Test",
        "/tmp/z",
        "2026-06-26 00:00:00 UTC",
        &mut buf_ee,
    )
    .expect("write_spa effort_exposure");
    let html_ee = String::from_utf8(buf_ee).expect("utf8 html_ee");
    let data_ee = extract_data_json(&html_ee).expect("parse data_ee");
    let ee = data_ee
        .get("effort_exposure")
        .and_then(|v| v.as_array())
        .expect("effort_exposure array present when non-empty");
    assert_eq!(ee.len(), 1, "one effort_exposure row expected");
    assert_eq!(
        ee[0].get("band").and_then(serde_json::Value::as_str),
        Some("red"),
    );
    assert!(
        (ee[0]
            .get("loc_share_pct")
            .and_then(serde_json::Value::as_f64)
            .expect("loc_share_pct f64")
            - 25.0)
            .abs()
            < 1e-9,
        "loc_share_pct round-trip",
    );
    assert!(
        (ee[0]
            .get("churn_share_pct")
            .and_then(serde_json::Value::as_f64)
            .expect("churn_share_pct f64")
            - 35.0)
            .abs()
            < 1e-9,
        "churn_share_pct round-trip",
    );
    assert_eq!(
        ee[0].get("files").and_then(serde_json::Value::as_u64),
        Some(3),
    );

    // The centralized band thresholds must appear in the options block so the
    // SPA JS can read them from data.options instead of hardcoding them.
    let opts = data.get("options").expect("options block present in JSON");
    let green_min = opts
        .get("health_green_min")
        .and_then(serde_json::Value::as_f64)
        .expect("health_green_min in options");
    assert!(
        (green_min - codelore_lib::bands::HEALTH_GREEN_MIN).abs() < 1e-9,
        "health_green_min must equal HEALTH_GREEN_MIN ({}) but got {green_min}",
        codelore_lib::bands::HEALTH_GREEN_MIN,
    );
    let yellow_min = opts
        .get("health_yellow_min")
        .and_then(serde_json::Value::as_f64)
        .expect("health_yellow_min in options");
    assert!(
        (yellow_min - codelore_lib::bands::HEALTH_YELLOW_MIN).abs() < 1e-9,
        "health_yellow_min must equal HEALTH_YELLOW_MIN ({}) but got {yellow_min}",
        codelore_lib::bands::HEALTH_YELLOW_MIN,
    );
}

/// Asserts that the bivariate health×activity color helpers are inlined
/// into the emitted SPA bundle. The helpers are pure JS in `widgets.js`;
/// this test proves they survive the `include_str!` embedding.
#[test]
fn spa_embeds_bivariate_palette() {
    let dash = SpaDashboard::default();
    let mut buf: Vec<u8> = Vec::new();
    write_spa(&dash, "CodeLore Dashboard", ".", "now", &mut buf).expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8");
    assert!(
        html.contains("BIVARIATE_PALETTE"),
        "bivariate palette constant must ship in widgets.js",
    );
    assert!(
        html.contains("function bivariateColor"),
        "bivariateColor helper must ship in widgets.js",
    );
}

/// Asserts that the bivariate tab is the active default in the emitted
/// SPA and that the legend mount ships alongside the circle-pack widget.
#[test]
fn spa_bivariate_is_default_map_mode() {
    let dash = SpaDashboard::default();
    let mut buf: Vec<u8> = Vec::new();
    write_spa(&dash, "CodeLore Dashboard", ".", "now", &mut buf).expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8");
    // The bivariate tab exists and is the active default.
    assert!(
        html.contains("data-mode=\"bivariate\""),
        "bivariate tab must exist"
    );
    // The legend mount ships.
    assert!(
        html.contains("id=\"bivariate-legend\""),
        "bivariate legend mount must exist"
    );
    // The bivariate tab carries tab-active (cognitive must not be the default).
    // Capture the whole <button ...> open tag so whitespace alignment between
    // data-mode and class does not affect the assertion.
    let biv = html.find("data-mode=\"bivariate\"").unwrap();
    let tab_open = html[..biv].rfind("<button").unwrap();
    let tag_end = tab_open + html[tab_open..].find('>').unwrap();
    let tab_html = &html[tab_open..tag_end];
    assert!(
        tab_html.contains("tab-active"),
        "bivariate tab must be the default (tab-active); tag was: {tab_html}"
    );
}

/// `function_xray` round-trips through the embedded SPA JSON with the
/// expected shape: a `[{path, rows:[{function, change_freq, loc, …}]}]`
/// array. Confirms that `FileFunctionXray` serialises correctly and that
/// the SPA's `data.function_xray` key is present when the field is
/// non-empty. The X-Ray tab JS reads this array by path lookup.
#[test]
fn spa_embeds_function_xray_data() {
    let dash = SpaDashboard {
        function_xray: vec![FileFunctionXray {
            path: "src/hot.rs".into(),
            rows: vec![
                FunctionXrayRow {
                    function: "hot".into(),
                    change_freq: 4,
                    loc: 9,
                    cyclomatic: Some(1),
                    cognitive: Some(0),
                    last_changed: "2026-01-04".into(),
                },
                FunctionXrayRow {
                    function: "cold".into(),
                    change_freq: 0,
                    loc: 3,
                    cyclomatic: Some(1),
                    cognitive: Some(0),
                    last_changed: String::new(),
                },
            ],
        }],
        ..SpaDashboard::default()
    };

    let mut buf = Vec::new();
    write_spa(
        &dash,
        "X-Ray Test",
        "/tmp/xr",
        "2026-01-01 00:00:00 UTC",
        &mut buf,
    )
    .expect("write_spa function_xray");
    let html = String::from_utf8(buf).expect("utf8");

    // The X-Ray tab renderer function must be present in the bundle.
    assert!(
        html.contains("drawer-panel-xray") || html.contains("function_xray"),
        "function_xray wiring must be present in the emitted SPA bundle",
    );

    let data = extract_data_json(&html).expect("parse data block");
    let fx = data
        .get("function_xray")
        .and_then(|v| v.as_array())
        .expect("function_xray array present when non-empty");
    assert_eq!(fx.len(), 1, "one FileFunctionXray entry expected");
    assert_eq!(
        fx[0].get("path").and_then(serde_json::Value::as_str),
        Some("src/hot.rs"),
    );
    let rows = fx[0]
        .get("rows")
        .and_then(|v| v.as_array())
        .expect("rows array");
    assert_eq!(rows.len(), 2, "two FunctionXrayRow entries expected");
    assert_eq!(
        rows[0].get("function").and_then(serde_json::Value::as_str),
        Some("hot"),
    );
    assert_eq!(
        rows[0]
            .get("change_freq")
            .and_then(serde_json::Value::as_u64),
        Some(4),
    );
    assert_eq!(
        rows[0].get("loc").and_then(serde_json::Value::as_u64),
        Some(9),
    );
    assert_eq!(
        rows[0]
            .get("cyclomatic")
            .and_then(serde_json::Value::as_i64),
        Some(1),
    );
}

/// The DSM Fusion cell-mode (SPA-only widget logic in
/// `40_architecture.js`) aggregates `data.coupling` to module depth and
/// classifies cells against the SAME `data.imports` the structural
/// matrix already renders — both fields must round-trip into the
/// embedded payload, and the widget registry must pass `coupling` as
/// the third argument to `renderArchMatrix`. Guards against either the
/// field or the wiring silently dropping.
#[test]
fn spa_embeds_dsm_fusion_precondition_data() {
    let dash = SpaDashboard {
        imports: vec![ImportEdgeRow {
            src_path: "src/alpha/service.rs".into(),
            target_path: "src/beta/handler.rs".into(),
        }],
        coupling: vec![CouplingRow {
            entity_a: "src/alpha/service.rs".into(),
            entity_b: "src/gamma/handler.rs".into(),
            shared: 6,
            revs_a: 10,
            revs_b: 8,
            average_revs: 9,
            degree: 66.7,
            fisher_p: 0.01,
        }],
        ..SpaDashboard::default()
    };

    let mut buf = Vec::new();
    write_spa(
        &dash,
        "CodeLore Dashboard",
        "/tmp/dsm-fusion",
        "2026-07-14 00:00:00 UTC",
        &mut buf,
    )
    .expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8 html");

    assert!(
        html.contains(
            "renderArchMatrix(data.imports || [], data.architecture_roles || [], data.coupling || [])"
        ),
        "arch-matrix widget must be wired to pass coupling as the third argument \
         (the DSM Fusion precondition) — the registry call may have dropped it",
    );

    let data = extract_data_json(&html).expect("parse data block");
    let imports = data
        .get("imports")
        .and_then(|v| v.as_array())
        .expect("imports array");
    assert_eq!(imports.len(), 1, "one import edge expected");

    let coupling = data
        .get("coupling")
        .and_then(|v| v.as_array())
        .expect("coupling array present when non-empty");
    assert_eq!(coupling.len(), 1, "one coupling row expected");
    assert_eq!(
        coupling[0]
            .get("entity_a")
            .and_then(serde_json::Value::as_str),
        Some("src/alpha/service.rs"),
    );
    let degree = coupling[0]
        .get("degree")
        .and_then(serde_json::Value::as_f64)
        .expect("degree f64");
    assert!((degree - 66.7).abs() < 1e-9);
}

/// Honest absence (spec: "no coupling data → Fusion mode falls back with
/// a hint"): when `coupling` is empty, `data.coupling` must be absent
/// from the embedded JSON (the same `skip_serializing_if` contract every
/// other optional widget field relies on), and the shipped JS bundle
/// must carry the exact hint string Fusion mode shows instead of a
/// misleading empty classification.
#[test]
fn spa_dsm_fusion_honest_absence_when_coupling_empty() {
    let dash = SpaDashboard {
        imports: vec![ImportEdgeRow {
            src_path: "src/alpha/service.rs".into(),
            target_path: "src/beta/handler.rs".into(),
        }],
        // `coupling` deliberately left at its `Vec::default()` (empty) —
        // the coupling-free fixture the honest-absence hint must cover.
        ..SpaDashboard::default()
    };

    let mut buf = Vec::new();
    write_spa(
        &dash,
        "CodeLore Dashboard",
        "/tmp/dsm-fusion-absent",
        "2026-07-14 00:00:00 UTC",
        &mut buf,
    )
    .expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8 html");

    let data = extract_data_json(&html).expect("parse data block");
    assert!(
        data.get("coupling").is_none(),
        "coupling must be absent from the embedded JSON when empty \
         (skip_serializing_if contract) — got: {:?}",
        data.get("coupling"),
    );
    assert!(
        html.contains("No co-change data — showing structure only"),
        "Fusion mode's honest-absence hint string is missing from the shipped JS bundle",
    );
}

/// The dashboard's `<main>` region is six titled `.dash-group` sections
/// (Overview, Hotspots & Risk, Code Health, Architecture, Knowledge,
/// Delivery), each in a fixed order carrying its assigned widgets — a
/// pure template-structure contract, unaffected by dashboard data, so
/// an empty `SpaDashboard::default()` is enough to exercise it.
#[test]
fn dashboard_groups_exist_in_order_with_widgets_assigned() {
    let html = build_dashboard_html();
    let order = [
        "group-overview",
        "group-hotspots",
        "group-code-health",
        "group-architecture",
        "group-knowledge",
        "group-delivery",
    ];
    let mut last = 0;
    for id in order {
        let pos = html.find(&format!("id=\"{id}\"")).expect(id);
        assert!(pos > last, "{id} out of order");
        last = pos;
    }
    // Spot-check assignments: widget id appears AFTER its group id and
    // BEFORE the next group id.
    let idx = |needle: &str| html.find(needle).expect(needle);
    assert!(
        idx("id=\"widget-hotspot-table\"") > idx("id=\"group-hotspots\"")
            && idx("id=\"widget-hotspot-table\"") < idx("id=\"group-code-health\"")
    );
    assert!(
        idx("id=\"widget-arch-matrix\"") > idx("id=\"group-architecture\"")
            && idx("id=\"widget-arch-matrix\"") < idx("id=\"group-knowledge\"")
    );
    assert!(idx("id=\"widget-calendar-heatmap\"") > idx("id=\"group-delivery\""));
}

/// Group headings are the page's `<h2>` tier; widget titles demote to
/// `<h3>` (heading hierarchy: h1 page → h2 section → h3 widget).
#[test]
fn widget_titles_are_h3_under_h2_groups() {
    let html = build_dashboard_html();
    assert!(html.contains("<h2 id=\"group-overview-h\""));
    assert!(html.contains("<h3>Quality dimensions</h3>"));
    assert!(
        !html.contains("<h2>Quality dimensions</h2>"),
        "widget titles must no longer be h2",
    );
}

/// The sticky nav (`#dash-nav`, sibling of the header) carries one chip
/// per `.dash-group` section, in the same order as the sections
/// themselves — pure template structure, unaffected by dashboard data.
#[test]
fn dash_nav_has_six_chips_in_section_order() {
    let html = build_dashboard_html();
    let nav_start = html.find("id=\"dash-nav\"").expect("dash-nav present");
    let order = [
        "group-overview",
        "group-hotspots",
        "group-code-health",
        "group-architecture",
        "group-knowledge",
        "group-delivery",
    ];
    let mut last = nav_start;
    for id in order {
        let needle = format!("data-target=\"{id}\"");
        let pos = html.find(&needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(pos > last, "{id} chip out of order");
        last = pos;
    }
    let chip_count = html.matches("class=\"dash-nav-chip").count();
    assert_eq!(chip_count, 6, "expected exactly six nav chips");
}

/// Build the dashboard HTML from an empty (all-default) `SpaDashboard`.
/// The `<main>` structure — group sections, widget mount points, and
/// titles — is static template markup independent of any analysis
/// data, so this is enough to assert structural invariants without
/// standing up a full fixture repo + `FactsDb` ingest.
fn build_dashboard_html() -> String {
    let dash = SpaDashboard::default();
    let mut buf = Vec::new();
    write_spa(
        &dash,
        "CodeLore Dashboard",
        "/tmp/layout-overhaul",
        "2026-07-14 00:00:00 UTC",
        &mut buf,
    )
    .expect("write_spa");
    String::from_utf8(buf).expect("utf8 html")
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
