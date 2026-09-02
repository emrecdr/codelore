//! Headless-browser smoke test for the `--format spa` dashboard.
//!
//! Closes the runtime-defect blind spot the SPA-runtime post-mortem
//! flagged: the existing `spa_integration_test` greps the rendered
//! HTML for string presence but never *runs* the JS. Both the
//! `METRIC_DEFS` Temporal Dead Zone and the Alpine init-order defects
//! shipped through every SPA-touching PR because no JS executed at
//! CI time.
//!
//! This test renders the SPA via the real emitter, opens it in
//! headless Chrome, lets Alpine + widgets boot, then asserts:
//!
//! 1. **No console errors** — the runtime-init class of
//!    bug surfaces as a console error within milliseconds of load.
//! 2. **KPI tiles rendered** — proves `renderKpiTiles` actually ran
//!    against the embedded JSON without throwing.
//!
//! Gated behind the `browser-tests` Cargo feature (default OFF). CI's
//! ubuntu matrix opts in; contributor machines without Chrome skip
//! the test entirely (the launcher returns an error that we map to a
//! skip on the assumption that "no Chrome" is a local-dev condition,
//! not a regression).

#![cfg(all(feature = "browser-tests", feature = "spa", feature = "test-support"))]

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codelore_lib::Options;
use codelore_lib::analyses::code_health::{CodeHealthRow, run_code_health};
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::analyses::hotspots::{HotspotRow, run_hotspots};
use codelore_lib::analyses::knowledge_islands::run_knowledge_islands;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::analyses::team_composition::run_team_composition;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::spa::{SpaDashboard, write_spa};
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::{
    coupling_repo, delivery_repo, differential_repo, permissive_coupling_opts,
};
use headless_chrome::Browser;
use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::protocol::cdp::Page;
use headless_chrome::protocol::cdp::types::Event;

// Row types used to populate otherwise-dark SpaDashboard fields so their
// widget render branches execute under the browser gate.
use codelore_lib::analyses::architecture_roles::ArchitectureRoleRow;
use codelore_lib::analyses::architecture_trend::ArchitectureTrendRow;
use codelore_lib::analyses::dashboard::{
    CloneSummary, DailyCommit, ImportEdgeRow, KameiRiskRow, TrendPoint, XRayEntry,
};
use codelore_lib::analyses::effort_exposure::EffortExposureRow;
use codelore_lib::analyses::entity_ownership::EntityOwnershipRow;
use codelore_lib::analyses::factors::health_trend_factors;
use codelore_lib::analyses::health_trend::HealthTrendRow;
use codelore_lib::analyses::mi::MiRollup;
use codelore_lib::analyses::modularity_violations::ModularityViolationRow;
use codelore_lib::analyses::refactoring_targets::RefactoringTargetRow;
use codelore_lib::analyses::unstable_interface::UnstableInterfaceRow;

/// Render the SPA from the differential fixture and run it in a real
/// browser. Fails on any browser-console error within a short
/// post-boot window — which is the exact failure shape of the
/// runtime-init class of bug.
#[test]
#[allow(clippy::too_many_lines)] // mirror of the existing spa_integration_test shape
fn rendered_spa_boots_without_console_errors() {
    // -- Step 1: produce a real SPA HTML file from the fixture. -------
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");

    // Synthetic payloads for dark widget render-branch coverage.
    // Values are arbitrary-but-realistic; assertions below verify each
    // branch reached its chart-mount path, not the chart geometry.
    // Entity paths share the src/alpha and src/beta prefixes used by
    // write_smoke_spa so future fixtures can share synthetic helpers.
    let entity_ownership = vec![
        EntityOwnershipRow {
            entity: "src/alpha/service.rs".to_string(),
            author: "Alice".to_string(),
            added: 200,
            deleted: 40,
        },
        EntityOwnershipRow {
            entity: "src/beta/handler.rs".to_string(),
            author: "Bob".to_string(),
            added: 150,
            deleted: 30,
        },
    ];
    let clones = vec![
        CloneSummary {
            path: "src/alpha/service.rs".to_string(),
            groups: 2,
        },
        CloneSummary {
            path: "src/beta/handler.rs".to_string(),
            groups: 1,
        },
    ];
    let modularity_violations = vec![ModularityViolationRow {
        entity_a: "src/alpha/service.rs".to_string(),
        entity_b: "src/beta/handler.rs".to_string(),
        shared: 5,
        degree: 0.55,
        fisher_p: 0.02,
    }];
    let unstable_interface = vec![UnstableInterfaceRow {
        path: "src/alpha/service.rs".to_string(),
        fan_in: 4,
        revisions: 12,
        coupled_dependents: 3,
        instability_score: 36.0,
    }];
    let architecture_roles = vec![
        ArchitectureRoleRow {
            path: "src/alpha/service.rs".to_string(),
            role: "shared".to_string(),
            vfi: 8,
            vfo: 2,
            in_cycle: false,
            level: 1,
            reach_pct: 25.0,
        },
        ArchitectureRoleRow {
            path: "src/beta/handler.rs".to_string(),
            role: "periphery".to_string(),
            vfi: 1,
            vfo: 0,
            in_cycle: false,
            level: 0,
            reach_pct: 0.0,
        },
    ];
    let architecture_trend = vec![
        ArchitectureTrendRow {
            date: "2026-01-01".to_string(),
            rev: "abc123456789".to_string(),
            files: 8,
            propagation_cost: 0.12,
            cycle_count: 0,
            largest_cycle: 0,
        },
        ArchitectureTrendRow {
            date: "2026-02-01".to_string(),
            rev: "def234567890".to_string(),
            files: 10,
            propagation_cost: 0.18,
            cycle_count: 1,
            largest_cycle: 3,
        },
        ArchitectureTrendRow {
            date: "2026-03-01".to_string(),
            rev: "fad345678901".to_string(),
            files: 12,
            propagation_cost: 0.22,
            cycle_count: 2,
            largest_cycle: 4,
        },
    ];
    let mi_rollup = Some(MiRollup {
        low: 2,
        moderate: 5,
        high: 3,
        unknown: 1,
    });
    let coupling_density = Some(0.08_f64);
    let imports = vec![
        ImportEdgeRow {
            src_path: "src/alpha/service.rs".to_string(),
            target_path: "src/beta/handler.rs".to_string(),
        },
        ImportEdgeRow {
            src_path: "src/beta/handler.rs".to_string(),
            target_path: "src/alpha/mod_0.rs".to_string(),
        },
    ];
    let xray = vec![
        XRayEntry {
            path: "src/alpha/service.rs".to_string(),
            function: "run".to_string(),
            cognitive: 5.0,
            start_line: 10,
            end_line: 40,
        },
        XRayEntry {
            path: "src/beta/handler.rs".to_string(),
            function: "handle".to_string(),
            cognitive: 3.0,
            start_line: 5,
            end_line: 25,
        },
    ];

    // Synthetic effort-exposure rows so renderShareBars reaches its
    // bars-render path (not the empty-state branch) under the browser gate.
    let effort_exposure = vec![
        EffortExposureRow {
            band: "red".into(),
            files: 2,
            loc_share_pct: 18.0,
            commit_share_pct: 35.0,
            churn_share_pct: 30.0,
            commit_share_ci_low: 0.22,
            commit_share_ci_high: 0.50,
            churn_share_improving_pct: None,
            churn_share_degrading_pct: None,
        },
        EffortExposureRow {
            band: "yellow".into(),
            files: 3,
            loc_share_pct: 32.0,
            commit_share_pct: 25.0,
            churn_share_pct: 28.0,
            commit_share_ci_low: 0.16,
            commit_share_ci_high: 0.36,
            churn_share_improving_pct: None,
            churn_share_degrading_pct: None,
        },
        EffortExposureRow {
            band: "green".into(),
            files: 5,
            loc_share_pct: 50.0,
            commit_share_pct: 40.0,
            churn_share_pct: 42.0,
            commit_share_ci_low: 0.28,
            commit_share_ci_high: 0.54,
            churn_share_improving_pct: None,
            churn_share_degrading_pct: None,
        },
    ];

    // Refactoring-targets rows with paths deliberately DISTINCT from the
    // hotspot set, pre-sorted by priority DESC (as the builder emits them).
    // The guided tour's "targets" step must brush THESE paths, not the
    // top-hotspot proxy — so the step-3 assertion can prove the brush was
    // sourced from `data.refactoring_targets`.
    let refactoring_targets = vec![
        RefactoringTargetRow {
            path: "src/refac/first_target.rs".to_string(),
            priority: 9.9,
            combined_risk: 42.0,
            structural_risk: 0.8,
            hotspot_score: 6.0,
            revisions: 20,
            loc: 30,
            dominant_type: "complex-method".to_string(),
            band: "red".to_string(),
            manual_up_rank: 1,
        },
        RefactoringTargetRow {
            path: "src/refac/second_target.rs".to_string(),
            priority: 7.1,
            combined_risk: 21.0,
            structural_risk: 0.6,
            hotspot_score: 4.0,
            revisions: 12,
            loc: 40,
            dominant_type: "duplication".to_string(),
            band: "yellow".to_string(),
            manual_up_rank: 2,
        },
    ];
    let refactoring_target_paths = refactoring_targets
        .iter()
        .map(|r| r.path.clone())
        .collect::<Vec<_>>();

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        entity_ownership,
        clones,
        modularity_violations,
        unstable_interface,
        architecture_roles,
        architecture_trend,
        mi_rollup,
        coupling_density,
        imports,
        xray,
        effort_exposure,
        refactoring_targets,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Browser Smoke",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    // -- Step 2: launch headless Chrome. ------------------------------
    // `Browser::default()` resolves Chrome via the standard system
    // search path AND has a bundled-Chromium fetch fallback. On CI
    // (`ubuntu-latest`) Chrome is pre-installed; locally a dev
    // without Chrome will see the launcher Err with "could not
    // auto-detect", which we map to a skip so `cargo test` doesn't
    // panic on stock contributor machines.
    let browser = match Browser::default() {
        Ok(b) => b,
        Err(e) => {
            // On CI the skip must be a FAILURE: this suite is the only
            // place the dashboard's JS executes at all, and a broken
            // Chrome install used to turn the whole job into a silent
            // 2-minute green — the exact hole its own module doc blames
            // for shipped JS defects. The env var is set by the CI job;
            // contributor machines without Chrome still skip cleanly.
            assert!(
                std::env::var("CODELORE_REQUIRE_BROWSER").is_err(),
                "CODELORE_REQUIRE_BROWSER is set but Chrome failed to \
                 launch ({e}) — a browser-required environment must fail, \
                 not silently skip the only JS coverage"
            );
            println!(
                "spa_browser_test: skipping — could not launch Chrome ({e}). \
                 Install Chrome / Chromium and retry."
            );
            return;
        }
    };

    let tab = browser.new_tab().expect("new tab");
    // CDP `Runtime` domain must be enabled before we'll see
    // `RuntimeExceptionThrown` or `RuntimeConsoleAPICalled` events.
    // `Log` domain emits the higher-level `Browser` console messages
    // (e.g. uncaught exceptions filtered into the network panel).
    tab.enable_log().expect("enable log");
    tab.enable_runtime().expect("enable runtime");

    // -- Step 3: collect console errors + thrown exceptions. ----------
    // Both surfaces matter — the `METRIC_DEFS` TDZ surfaced as a `console.error`
    // (`Uncaught ReferenceError: Cannot access 'METRIC_DEFS' before
    // initialization`) while a future ECharts crash could surface as
    // `RuntimeExceptionThrown` (a top-level throw out of an async
    // listener). We capture both and fail on either being non-empty.
    let console_errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let exception_sink = Arc::clone(&console_errors);
    let listener = move |event: &Event| {
        if let Event::RuntimeExceptionThrown(thrown) = event {
            let d = &thrown.params.exception_details;
            // Pull every field we can — the bare `.text` is often
            // just "Uncaught" with the real signal in the
            // `exception.description` or in stack_trace frames.
            let mut msg = format!("RuntimeException: {}", d.text);
            if let Some(ex) = &d.exception
                && let Some(desc) = &ex.description
            {
                msg.push_str(" — ");
                msg.push_str(desc);
            }
            let _ = write!(
                msg,
                " @ {}:{}",
                d.url.clone().unwrap_or_else(|| "<inline>".into()),
                d.line_number,
            );
            if let Some(trace) = &d.stack_trace {
                let frames: Vec<String> = trace
                    .call_frames
                    .iter()
                    .take(5)
                    .map(|f| format!("{} @ {}:{}", f.function_name, f.url, f.line_number))
                    .collect();
                if !frames.is_empty() {
                    msg.push_str("\n      stack: ");
                    msg.push_str(&frames.join("\n             "));
                }
            }
            exception_sink.lock().expect("console mutex").push(msg);
        }
    };
    tab.add_event_listener(Arc::new(listener))
        .expect("add event listener");

    // -- Step 4: navigate to the rendered HTML. -----------------------
    let url = format!("file://{}", html_path.display());
    tab.navigate_to(&url).expect("navigate");
    tab.wait_until_navigated().expect("wait navigation");

    // Give Alpine + every widget a window to boot. ECharts widget
    // mounts are asynchronous; the existing rerender registry is
    // populated inside the IIFE's main body, so anything below ~1 sec
    // is too eager. 2 seconds is the sweet spot empirically — long
    // enough for the slowest widget (X-Ray sunburst), short enough
    // not to balloon the per-CI-run cost.
    std::thread::sleep(Duration::from_secs(2));

    // -- Step 5: assert no console errors fired. ----------------------
    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "SPA produced {} browser-console error(s) during boot:\n{}",
        errors.len(),
        errors.join("\n  "),
    );

    // -- Step 6: assert KPI tiles rendered with real values. ----------
    // The KPI tiles are the first widget the boot section invokes;
    // if `renderKpiTiles` threw (the `METRIC_DEFS` TDZ surface), they'd stay
    // empty. We pull the rendered text out of the widget container
    // and assert it's non-trivial. Choose a specific KPI selector to
    // avoid false-passes on whitespace-only nodes.
    let kpi_html = tab
        .find_element("#widget-kpi-tiles")
        .expect("kpi tiles container")
        .get_content()
        .expect("kpi tiles html");
    assert!(
        kpi_html.contains("stat-value") || kpi_html.contains("kpi-value"),
        "KPI tiles container had no rendered tile content; \
         renderKpiTiles probably threw silently. HTML: {}",
        &kpi_html[..kpi_html.len().min(500)]
    );

    // -- Step 7: assert the bivariate legend rendered. -----------------
    // `renderBivariateLegend` populates `#bivariate-legend` during the
    // same widget-boot pass as `renderKpiTiles`; an empty container
    // means the bivariate renderer threw or was not wired to boot.
    let legend_html = tab
        .find_element("#bivariate-legend")
        .expect("bivariate legend container")
        .get_content()
        .expect("bivariate legend html");
    assert!(
        legend_html.len() > 50,
        "bivariate legend container was empty; renderBivariateLegend \
         probably did not run. HTML: {}",
        &legend_html[..legend_html.len().min(200)]
    );

    // -- Step 8: assert bivariate tab is the default active selection. -
    // The hotspot colour-mode toggle bar must start with the bivariate
    // tab selected so the danger quadrant is visible on initial load
    // without swapping lenses.
    let bivariate_is_default: bool = eval_json(
        &tab,
        "(() => { \
             const bar = document.getElementById('hotspot-color-toggles'); \
             if (!bar) return false; \
             const sel = bar.querySelector('[aria-selected=\"true\"]'); \
             return !!sel && sel.getAttribute('data-mode') === 'bivariate'; \
         })()",
    );
    assert!(
        bivariate_is_default,
        "bivariate tab is not the default selected mode; the danger quadrant \
         is hidden on initial dashboard load"
    );

    // -- Step 9: assert cross-widget linked brushing highlights the table row. -
    // Publishing via the Alpine selection store fans out to all registered
    // subscribers on the next microtask. We split the store write and the
    // class read across two CDP round-trips so the microtask queue drains
    // between them — reading in the same JS turn as `set()` races and fails.
    // Scope both evals to #hotspot-tbody (not a bare tr[data-path] query)
    // because the knowledge-islands table emits tr[data-path] rows too, and
    // a KI-table path would not be found by the hotspot-table listener.
    let selected_path: String = eval_json(
        &tab,
        "(function () { \
             var tbody = document.getElementById('hotspot-tbody'); \
             if (!tbody) return ''; \
             var first = tbody.querySelector('tr[data-path]'); \
             if (!first) return ''; \
             var p = first.getAttribute('data-path'); \
             window.__codeloreTestSelPath = p; \
             window.Alpine.store('selection').set(p); \
             return p; \
         })()",
    );
    assert!(
        !selected_path.is_empty(),
        "no tr[data-path] row found in #hotspot-tbody; \
         the linked-brushing assertion requires at least one rendered hotspot row"
    );
    // One CDP round-trip is enough for Alpine's microtask effects to flush;
    // the class read is in a separate evaluate call so the queue has drained.
    std::thread::sleep(Duration::from_millis(100));
    let row_highlighted: bool = eval_json(
        &tab,
        "(function () { \
             var tbody = document.getElementById('hotspot-tbody'); \
             if (!tbody) return false; \
             var p = window.__codeloreTestSelPath; \
             if (!p) return false; \
             var row = tbody.querySelector('tr[data-path=\"' + p + '\"]'); \
             return !!row && row.classList.contains('!bg-base-300'); \
         })()",
    );
    assert!(
        row_highlighted,
        "setting selection via Alpine store did not highlight the matching \
         hotspot-table row — cross-widget linked brushing is not wired"
    );

    // -- Step 10: assert the coupling-sankey subscriber highlights on selection. -
    // Clear any prior selection FIRST (its own round-trip) so the publish below
    // is a guaranteed store CHANGE — re-setting the path the store already holds
    // is a no-op that never fans out, and an earlier step may have left the same
    // path selected. The sankey may be empty on a small fixture, so '' skips.
    let sankey_node: String = eval_json(
        &tab,
        "(function () { \
             var el = document.getElementById('widget-coupling-sankey-body'); \
             if (!el || !window.echarts) return ''; \
             var chart = window.echarts.getInstanceByDom(el); \
             if (!chart) return ''; \
             var opt = chart.getOption(); \
             var series = opt && opt.series && opt.series[0]; \
             var nodes = series && series.data; \
             if (!nodes || !nodes.length) return ''; \
             window.__codeloreSankeyTarget = nodes[0].name; \
             window.Alpine.store('selection').clear(); \
             return nodes[0].name; \
         })()",
    );
    if !sankey_node.is_empty() {
        std::thread::sleep(Duration::from_millis(100));
        // Spy on the sankey's dispatchAction, then publish the node name — now a
        // guaranteed change from the cleared (null) state — and read the captured
        // highlight target.
        let _: bool = eval_json(
            &tab,
            "(function () { \
                 var el = document.getElementById('widget-coupling-sankey-body'); \
                 var chart = el && window.echarts && window.echarts.getInstanceByDom(el); \
                 if (!chart) return false; \
                 window.__codeloreSankeyHi = null; \
                 var orig = chart.dispatchAction.bind(chart); \
                 chart.dispatchAction = function (p) { \
                     if (p && p.type === 'highlight') window.__codeloreSankeyHi = p.name || ''; \
                     return orig(p); \
                 }; \
                 window.Alpine.store('selection').set(window.__codeloreSankeyTarget); \
                 return true; \
             })()",
        );
        std::thread::sleep(Duration::from_millis(100));
        let captured: String = eval_json(
            &tab,
            "(function(){return window.__codeloreSankeyHi || '';})()",
        );
        assert_eq!(
            captured, sankey_node,
            "coupling-sankey selection listener did not dispatch a 'highlight' \
             for the published node name — the cross-widget subscriber is not wired"
        );
    }

    // -- Step 11: assert the sankey publish path lights the matching table row. -
    // A headless ECharts canvas click isn't reliably simulatable, so we invoke
    // the exact call the files-mode sankey click now makes — _codeloreShowDetail
    // on a node name that is also a hotspot-table path — and assert the broadcast
    // LIGHTS the row (clear first, so this proves the publish lit it, not residue).
    // This guards _codeloreShowDetail's publish + the hotspot-table subscriber,
    // not the sankey click wiring itself. With the differential fixture the
    // '.mailmap' node overlaps the table, so this exercises; '' => skip only if a
    // future fixture has an empty sankey or no sankey∩table path.
    let bridged_path: String = eval_json(
        &tab,
        "(function () { \
             var el = document.getElementById('widget-coupling-sankey-body'); \
             if (!el || !window.echarts) return ''; \
             var chart = window.echarts.getInstanceByDom(el); \
             if (!chart) return ''; \
             var opt = chart.getOption(); \
             var nodes = opt && opt.series && opt.series[0] && opt.series[0].data; \
             var tbody = document.getElementById('hotspot-tbody'); \
             if (!nodes || !tbody) return ''; \
             for (var i = 0; i < nodes.length; i++) { \
                 var n = nodes[i].name; \
                 if (tbody.querySelector('tr[data-path=\"' + n + '\"]')) { \
                     window.__codeloreBridge = n; \
                     window.Alpine.store('selection').clear(); \
                     return n; \
                 } \
             } \
             return ''; \
         })()",
    );
    if !bridged_path.is_empty() {
        std::thread::sleep(Duration::from_millis(100));
        // Publish via the exact call the files-mode sankey click now makes.
        let _: bool = eval_json(
            &tab,
            "(function(){ window._codeloreShowDetail(window.__codeloreBridge); return true; })()",
        );
        std::thread::sleep(Duration::from_millis(100));
        let row_lit: bool = eval_json(
            &tab,
            &format!(
                "(function () {{ var r = document.querySelector('#hotspot-tbody \
                  tr[data-path=\"{bridged_path}\"]'); return !!r && r.classList.contains('!bg-base-300'); }})()",
            ),
        );
        assert!(
            row_lit,
            "publishing a sankey node name via _codeloreShowDetail did not \
             highlight the matching hotspot-table row — the publish → \
             table-subscriber path is broken"
        );
    }

    // -- Step 12: bivariate legend set-brush emphasises a quadrant. -------
    // Click each legend cell until one selects a NON-EMPTY quadrant, then
    // assert the matching hotspot-table rows carry `.hotspot-row-brushed`.
    // Re-clicking the active cell clears. Return -1 = wiring missing (fail);
    // 0 = fixture has no populated quadrant (skip, guards empty data).
    let brushed_count: i64 = eval_json(
        &tab,
        "(function () { \
             var mount = document.getElementById('bivariate-legend'); \
             if (!mount) return -1; \
             var cells = mount.querySelectorAll('[data-biv-cell]'); \
             if (!cells.length) return -1; \
             var store = window.Alpine && window.Alpine.store && window.Alpine.store('brush'); \
             if (!store) return -1; \
             for (var i = 0; i < cells.length; i++) { \
                 store.clear(); \
                 cells[i].click(); \
                 if (store.paths && store.paths.length) { \
                     window.__codeloreBrushCellIdx = i; return store.paths.length; \
                 } \
             } \
             return 0; \
         })()",
    );
    assert!(
        brushed_count != -1,
        "bivariate brush store / legend cells not wired (missing #bivariate-legend \
         [data-biv-cell] cells or the `brush` Alpine store)"
    );
    if brushed_count > 0 {
        std::thread::sleep(Duration::from_millis(100));
        let rows_brushed: i64 = eval_json(
            &tab,
            "(function () { \
                 var t = document.getElementById('hotspot-tbody'); \
                 return t ? t.querySelectorAll('tr.hotspot-row-brushed').length : -1; \
             })()",
        );
        assert!(
            rows_brushed > 0,
            "legend set-brush selected a non-empty quadrant but no hotspot-table row \
             got `.hotspot-row-brushed` — the brush fan-out / table subscriber is not wired"
        );
        let _: bool = eval_json(
            &tab,
            "(function () { \
                 var cells = document.getElementById('bivariate-legend') \
                     .querySelectorAll('[data-biv-cell]'); \
                 cells[window.__codeloreBrushCellIdx].click(); return true; \
             })()",
        );
        std::thread::sleep(Duration::from_millis(100));
        let rows_after_clear: i64 = eval_json(
            &tab,
            "(function () { \
                 var t = document.getElementById('hotspot-tbody'); \
                 return t ? t.querySelectorAll('tr.hotspot-row-brushed').length : -1; \
             })()",
        );
        assert_eq!(
            rows_after_clear, 0,
            "re-clicking the active legend cell did not clear the quadrant brush"
        );
    } else {
        println!(
            "spa_browser_test: bivariate brush step skipped — fixture has no populated \
             health×activity quadrant (no code_health bands intersecting hotspots)"
        );
    }

    // -- Step 13: assert the arch-trend widget charted. --------------------
    // `renderArchTrend` calls `setChartAriaLabel` before mounting ECharts,
    // which stamps `role="img"` on the container. A bailed-at-empty renderer
    // leaves the attribute absent. Populated `architecture_trend` above
    // ensures the branch reaches the chart-mount path.
    let arch_trend_charted: bool = eval_json(
        &tab,
        "(function () { \
             var el = document.getElementById('widget-arch-trend-body'); \
             return !!el && el.getAttribute('role') === 'img'; \
         })()",
    );
    assert!(
        arch_trend_charted,
        "arch-trend container did not receive role=img; \
         renderArchTrend bailed at the empty-data guard despite populated payload"
    );

    // -- Step 14: assert the MI band KPI sub-tile appeared. ----------------
    // When `mi_rollup` has at least one file with a known band,
    // `renderKpiTiles` injects a tile whose sub-description text is
    // "top / mid / bottom quartile". Asserting that text confirms the
    // mi_rollup payload reached the renderer and the branch executed.
    let mi_tile_present: bool = eval_json(
        &tab,
        "(function () { \
             var kpi = document.getElementById('widget-kpi-tiles'); \
             return !!kpi && kpi.textContent.includes('bottom quartile'); \
         })()",
    );
    assert!(
        mi_tile_present,
        "MI band KPI sub-tile was absent from #widget-kpi-tiles; \
         mi_rollup payload may not have reached renderKpiTiles"
    );

    // -- Step 15: assert share-bars widget mounted without console errors. --
    // `renderShareBars` replaces the mount point's innerHTML with either the
    // bars container or the empty-state message. An empty inner-HTML means
    // the renderer threw before touching the DOM. The widget section must
    // exist in the DOM (widget-share-bars-body) and its body must be
    // non-empty after the boot window.
    let share_bars_mounted: bool = eval_json(
        &tab,
        "(function () { \
             var el = document.getElementById('widget-share-bars-body'); \
             return !!el && el.innerHTML.trim().length > 0; \
         })()",
    );
    assert!(
        share_bars_mounted,
        "share-bars widget body (#widget-share-bars-body) was empty after boot; \
         renderShareBars may have thrown or the widget mount point is missing"
    );

    // -- Step 16: assert guided-tour widget mounted with Start button. --
    // At boot the tour is inactive (tourStep = -1); renderGuidedTour writes
    // a "Start tour" button into the mount point. An empty body means the
    // renderer threw before touching the DOM.
    let tour_mounted: bool = eval_json(
        &tab,
        "(function () { \
             var el = document.getElementById('widget-guided-tour-body'); \
             if (!el || el.innerHTML.trim().length === 0) return false; \
             return el.querySelector('#tour-next') !== null; \
         })()",
    );
    assert!(
        tour_mounted,
        "guided-tour widget body (#widget-guided-tour-body) was empty or missing \
         the Start button after boot; renderGuidedTour may have thrown"
    );

    // -- Step 17: click-through the full guided tour. ----------------------
    // Drives the applyTourStep state machine: Start → step 0 (health) →
    // step 1 (cognitive) → step 2 (friction) → step 3 (health + brush) →
    // Exit (bivariate restored, brush cleared). After each step the color-mode
    // tab for that lens must carry aria-selected="true"; after Exit the
    // bivariate tab must be selected and the brush must be empty.
    //
    // Helper: returns the data-mode of the currently aria-selected color tab.
    let active_mode = || -> String {
        eval_json(
            &tab,
            "(function () { \
                 var bar = document.getElementById('hotspot-color-toggles'); \
                 if (!bar) return ''; \
                 var btns = bar.querySelectorAll('button[role=\"tab\"],button.toggle'); \
                 for (var i = 0; i < btns.length; i++) { \
                     if (btns[i].getAttribute('aria-selected') === 'true') \
                         return btns[i].getAttribute('data-mode') || ''; \
                 } \
                 return ''; \
             })()",
        )
    };
    // Helper: brush path count (0 = clear).
    let brush_count = || -> i64 {
        eval_json(
            &tab,
            "(function () { \
                 var s = window.Alpine && window.Alpine.store && \
                         window.Alpine.store('brush'); \
                 if (!s || !s.paths) return 0; \
                 return s.paths.length; \
             })()",
        )
    };
    // Helper: click #tour-next and wait for the DOM to settle.
    let click_next = || {
        let _: bool = eval_json(
            &tab,
            "(function () { \
                 var btn = document.getElementById('tour-next'); \
                 if (btn) btn.click(); \
                 return !!btn; \
             })()",
        );
        std::thread::sleep(Duration::from_millis(120));
    };

    // Before start: tour is inactive, color mode is unaffected by the tour.
    // Click Start → step 0 (health).
    click_next();
    assert_eq!(
        active_mode(),
        "health",
        "after Start the color-mode tab with data-mode='health' should be aria-selected; \
         applyTourStep(0) did not sync the tab bar"
    );
    assert_eq!(
        brush_count(),
        0,
        "step 0 should not set a brush (brushRefactoringTargets is false for step 0)"
    );

    // Next → step 1 (cognitive).
    click_next();
    assert_eq!(
        active_mode(),
        "cognitive",
        "after Next to step 1 the data-mode='cognitive' tab should be aria-selected"
    );

    // Next → step 2 (friction).
    click_next();
    assert_eq!(
        active_mode(),
        "friction",
        "after Next to step 2 the data-mode='friction' tab should be aria-selected"
    );

    // Next → step 3 (health + top-10 brush).
    click_next();
    assert_eq!(
        active_mode(),
        "health",
        "after Next to step 3 the data-mode='health' tab should be aria-selected"
    );
    let brushed_on_step3 = brush_count();
    assert!(
        brushed_on_step3 > 0,
        "step 3 (brushRefactoringTargets=true) should brush at least one path; \
         Alpine brush store is empty — bs.set(['targets','top10'],[...]) may not have fired"
    );
    // The brushed set must be the REAL refactoring targets (risk ÷ effort
    // ordering), not the top-hotspot proxy. The fixture's target paths are
    // disjoint from the hotspot set, so their presence proves the brush was
    // sourced from `data.refactoring_targets`. eval_json only reads primitives,
    // so stringify the array in-page and parse it here.
    let brushed_json: String = eval_json(
        &tab,
        "(function () { \
             var s = window.Alpine && window.Alpine.store && window.Alpine.store('brush'); \
             return JSON.stringify((s && s.paths) ? s.paths : []); \
         })()",
    );
    let brushed_paths: Vec<String> = serde_json::from_str(&brushed_json).expect("brush paths json");
    for want in &refactoring_target_paths {
        assert!(
            brushed_paths.iter().any(|p| p == want),
            "step 3 brush should contain refactoring-target path {want:?} \
             (sourced from data.refactoring_targets, not the hotspot proxy); got {brushed_paths:?}"
        );
    }

    // Next on the last step → Exit tour → bivariate restored, brush cleared.
    click_next();
    assert_eq!(
        active_mode(),
        "bivariate",
        "after Exit tour (Next on last step) the data-mode='bivariate' tab should be \
         aria-selected; exitTour() did not restore the bivariate tab"
    );
    assert_eq!(
        brush_count(),
        0,
        "after Exit tour the brush should be cleared; exitTour() called bs.clear() \
         but the Alpine store still reports paths"
    );
}

/// Boot the SPA in a browser with NO `scheduler.yield` — the state of every
/// Safari / iOS Safari, Chrome/Edge below 129, and Firefox below 142 — and
/// assert the full widget set still renders.
///
/// The cooperative boot loop yields between widgets via `yieldToMain`, whose
/// `MessageChannel` fallback path runs on exactly these browsers. That path
/// must not read any binding that is still in its temporal dead zone when the
/// loop makes its first `yieldToMain` call during the synchronous boot pass:
/// if it does, the un-awaited boot IIFE rejects and every widget after the
/// first stays blank. On the maintainers' own Chrome (>= 129) the
/// `scheduler.yield()` early return jumps clean over the fallback, so that
/// regression is invisible there — which is why this variant deletes
/// `scheduler` before any page script runs and drives the fallback directly.
#[test]
fn rendered_spa_boots_without_scheduler_yield() {
    // -- Step 1: produce a full-widget SPA (every body should populate). ---
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    write_smoke_spa(&html_path, "CodeLore No-Scheduler Boot");

    // -- Step 2: launch headless Chrome (skip cleanly if unavailable). -----
    let browser = match Browser::default() {
        Ok(b) => b,
        Err(e) => {
            // On CI the skip must be a FAILURE: this suite is the only
            // place the dashboard's JS executes at all, and a broken
            // Chrome install used to turn the whole job into a silent
            // 2-minute green — the exact hole its own module doc blames
            // for shipped JS defects. The env var is set by the CI job;
            // contributor machines without Chrome still skip cleanly.
            assert!(
                std::env::var("CODELORE_REQUIRE_BROWSER").is_err(),
                "CODELORE_REQUIRE_BROWSER is set but Chrome failed to \
                 launch ({e}) — a browser-required environment must fail, \
                 not silently skip the only JS coverage"
            );
            println!(
                "spa_browser_test: skipping — could not launch Chrome ({e}). \
                 Install Chrome / Chromium and retry."
            );
            return;
        }
    };
    let tab = browser.new_tab().expect("new tab");

    // -- Step 3: remove `scheduler` BEFORE the page's own scripts run. -----
    // `addScriptToEvaluateOnNewDocument` runs in the main world after the
    // document is created but before any of its scripts execute, so the
    // dashboard boots exactly as it would on a browser that never shipped
    // `scheduler.yield`. Redefine it to `undefined` (falling back to `delete`
    // if the global is non-configurable); Step 6 proves the override took.
    tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: "try { Object.defineProperty(window, 'scheduler', \
                 { value: undefined, configurable: true, writable: true }); } \
                 catch (e) { try { delete window.scheduler; } catch (e2) {} }"
            .to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    })
    .expect("register scheduler-removal script");

    // -- Step 4: collect uncaught exceptions / unhandled rejections. -------
    let console_errors = attach_exception_sink(&tab);

    // -- Step 5: navigate + let the cooperative boot loop run. -------------
    let url = format!("file://{}", html_path.display());
    tab.navigate_to(&url).expect("navigate");
    tab.wait_until_navigated().expect("wait navigation");
    std::thread::sleep(Duration::from_secs(2));

    // -- Step 6: vacuity guard — the no-scheduler path was actually taken. --
    // Without this, a browser that already lacks `scheduler` (or a silently
    // failed override) would let the test pass for the wrong reason.
    let scheduler_absent: bool = eval_json(&tab, "typeof scheduler === 'undefined'");
    assert!(
        scheduler_absent,
        "`scheduler` was still defined after the override — the no-scheduler \
         boot path was not exercised, so this test would pass vacuously"
    );

    // -- Step 7: every widget body rendered. -------------------------------
    // The loop renders the first widget, then yields before each of the rest.
    // If the first `yieldToMain` throws, the loop dies after widget 0 and only
    // `factor-header` has content. The selector matches every
    // `#widget-<name>-body` container regardless of its per-widget class.
    let total: i64 = eval_json(
        &tab,
        "document.querySelectorAll('[id^=\"widget-\"][id$=\"-body\"]').length",
    );
    assert_eq!(
        total, 23,
        "expected 23 widget-body containers in the template; got {total} \
         (template / WIDGETS drift — reconcile the count with the boot array)"
    );
    let rendered: i64 = eval_json(
        &tab,
        "(function () { \
             var bodies = document.querySelectorAll('[id^=\"widget-\"][id$=\"-body\"]'); \
             var n = 0; \
             for (var i = 0; i < bodies.length; i++) { \
                 if (bodies[i].innerHTML.trim().length > 0) n++; \
             } \
             return n; \
         })()",
    );
    assert_eq!(
        rendered, total,
        "only {rendered} of {total} widget bodies rendered with `scheduler.yield` \
         absent — the cooperative boot loop died on its first yield instead of \
         completing (a dead-zone read in the yieldToMain fallback)"
    );

    // -- Step 8: no uncaught exception / unhandled rejection fired. ---------
    // The dead-zone read surfaces as a rejection out of the un-awaited boot
    // IIFE; a clean fallback boot produces none.
    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "SPA produced {} browser error(s) during no-scheduler boot:\n  {}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// Click a Knowledge-Islands row and assert the file-detail drawer
/// opens POPULATED and then closes again. Guards the exact symptom
/// class the boot-only smoke test can't see: the drawer rendering as a
/// blank popup (no body, no close affordance) or refusing to close.
///
/// The boot test above only proves the page loads without console
/// errors; it never drives the row → drawer → close interaction, so a
/// regression in `showFileDetailDrawer` / the `detail` Alpine store /
/// the dialog close wiring would ship undetected.
#[test]
#[allow(clippy::too_many_lines)] // mirror of the boot test's shape + the interaction steps
fn knowledge_islands_row_opens_and_closes_detail_drawer() {
    // -- Step 1: produce a SPA that ACTUALLY has knowledge-island rows. --
    // The differential fixture's commits are all dated early-Jan 2026,
    // so a far-future anchor makes every author "departed" by tens of
    // thousands of days — every solo/dominant-owned live file surfaces
    // as a knowledge island regardless of the wall-clock date the test
    // runs on. Deterministic by construction.
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        // Far-future anchor → every contributor is "departed" relative to
        // their last commit, so the knowledge-islands table is populated.
        age_time_now: Some(time::macros::date!(2099 - 01 - 01)),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");

    // Fail loudly if the fixture stops producing KI rows — otherwise the
    // browser-side assertions would pass vacuously on an empty table.
    assert!(
        !knowledge_islands.is_empty(),
        "fixture produced no knowledge-island rows; the drawer-open \
         assertions below would be vacuous. Adjust the anchor / fixture."
    );
    let ki_row_count = knowledge_islands.len();
    println!("knowledge_islands_row_opens_and_closes_detail_drawer: {ki_row_count} KI rows");

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore KI Drawer Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    // -- Step 2: launch Chrome + let Alpine/widgets boot (skip if no Chrome). --
    // The KI table renders inside the cooperative widget-boot loop, and the
    // row click handlers attach during that render — so the helper's 2s
    // settle is what we wait on before driving the row.
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // -- Step 4: click a Knowledge-Islands row. --------------------------
    // `wait_for_element` polls until the KI render has produced rows, so
    // we don't race the cooperative boot scheduler.
    let row = tab
        .wait_for_element("tr.ki-row")
        .expect("at least one knowledge-islands row should render");
    row.click().expect("click KI row");
    // The drawer-show path mutates the DOM synchronously on click; a short
    // settle covers the radar ECharts mount + Alpine store propagation.
    std::thread::sleep(Duration::from_millis(300));

    // -- Step 5: assert the drawer OPENED and is POPULATED. --------------
    // `open === true`, no `[hidden]`, and computed `display !== 'none'`
    // is exactly the inverse of the "blank popup" symptom.
    let drawer_open: bool = eval_json(
        &tab,
        "document.getElementById('file-detail-drawer').open === true",
    );
    assert!(drawer_open, "detail drawer did not open on KI row click");

    let drawer_not_hidden: bool = eval_json(
        &tab,
        "!document.getElementById('file-detail-drawer').hasAttribute('hidden')",
    );
    assert!(
        drawer_not_hidden,
        "detail drawer kept the [hidden] attribute after open"
    );

    let drawer_displayed: bool = eval_json(
        &tab,
        "getComputedStyle(document.getElementById('file-detail-drawer')).display !== 'none'",
    );
    assert!(
        drawer_displayed,
        "detail drawer computed display:none after open (invisible popup)"
    );

    let title_len: i64 = eval_json(
        &tab,
        "document.getElementById('drawer-title').textContent.trim().length",
    );
    assert!(
        title_len > 0,
        "drawer title (clicked path) was empty; title_len={title_len}"
    );

    let body_len: i64 = eval_json(
        &tab,
        "document.getElementById('drawer-body').innerHTML.length",
    );
    assert!(body_len > 0, "drawer body was empty; body_len={body_len}");

    let has_ki_section: bool = eval_json(
        &tab,
        "document.getElementById('drawer-body').textContent.includes('Knowledge island')",
    );
    assert!(
        has_ki_section,
        "drawer body had no 'Knowledge island' section for a KI-row click"
    );

    // -- Step 6: assert the × close button actually closes the drawer. ---
    let close_btn = tab
        .find_element("#drawer-close")
        .expect("drawer close button should exist while drawer is open");
    close_btn.click().expect("click drawer close");
    // The dialog `close` event listener re-adds [hidden] + syncs the store
    // asynchronously; give it a turn before re-reading state.
    std::thread::sleep(Duration::from_millis(300));

    let drawer_closed: bool = eval_json(
        &tab,
        "(() => { const d = document.getElementById('file-detail-drawer'); \
             return d.open === false && (d.hasAttribute('hidden') || \
             getComputedStyle(d).display === 'none'); })()",
    );
    assert!(
        drawer_closed,
        "detail drawer did not close after clicking the × button"
    );
}

/// Open the file-detail drawer from a Knowledge-Islands row and assert
/// the a11y contract holds:
///
/// 1. The drawer carries an accessible NAME — `aria-labelledby` points
///    at the visible title — so assistive tech announces "<path>
///    dialog" instead of an unnamed group. A bare `<dialog>` has no
///    implicit name, so this is wired by hand and is the assertion that
///    fails on the un-fixed source.
/// 2. On open, focus is INSIDE the drawer (keyboard / screen-reader
///    users land on the freshly-revealed content, not the occluded
///    trigger).
/// 3. On close, focus RETURNS to the trigger row (the user resumes
///    where they left off).
///
/// The drawer is deliberately NON-MODAL (`dialog.show()`, not
/// `showModal()`) so a user can click another row while it is open. We
/// also wire the focus move-in / restore-out explicitly rather than
/// leaning on the platform's dialog-focusing steps, so the contract
/// holds even on engines whose non-modal focus handling diverges from
/// current Chrome.
#[test]
#[allow(clippy::too_many_lines)] // mirror of the KI-drawer test's shape + a11y assertions
fn detail_drawer_has_accessible_name_and_manages_focus() {
    // -- Step 1: produce a SPA that ACTUALLY has knowledge-island rows. --
    // Same far-future-anchor trick as the KI-drawer open/close test so
    // every solo-owned live file surfaces as a knowledge island.
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        age_time_now: Some(time::macros::date!(2099 - 01 - 01)),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");

    assert!(
        !knowledge_islands.is_empty(),
        "fixture produced no knowledge-island rows; the focus assertions \
         below would be vacuous. Adjust the anchor / fixture."
    );

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Drawer Focus Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    // -- Step 2: launch Chrome + let Alpine/widgets boot (skip if no Chrome). --
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // -- Step 4: focus the first KI row, then activate it. ---------------
    // The rows are keyboard-activable, so focusing first models the
    // keyboard-user path and gives `document.activeElement` a stable,
    // assertable trigger to restore to on close. We tag the row with a
    // marker attribute so we can identify "the same element" after the
    // drawer round-trips (DOM identity isn't queryable across evaluate
    // calls otherwise).
    tab.wait_for_element("tr.ki-row")
        .expect("at least one knowledge-islands row should render");
    tab.evaluate(
        "(() => { const r = document.querySelector('tr.ki-row'); \
         r.setAttribute('data-focus-trigger-marker', '1'); r.focus(); \
         return document.activeElement === r; })()",
        false,
    )
    .expect("focus the KI row");

    let row_focused_before: bool = eval_json(
        &tab,
        "document.activeElement === document.querySelector('tr[data-focus-trigger-marker]')",
    );
    assert!(
        row_focused_before,
        "could not focus the KI row before activation; test premise broken"
    );

    // Activate the row the way a keyboard user would (Enter). The row's
    // keydown handler routes to the same drawer-open path as a click.
    tab.evaluate(
        "(() => { const r = document.querySelector('tr[data-focus-trigger-marker]'); \
         r.dispatchEvent(new KeyboardEvent('keydown', \
         { key: 'Enter', bubbles: true })); })()",
        false,
    )
    .expect("activate KI row via Enter");
    std::thread::sleep(Duration::from_millis(300));

    // -- Step 5: assert the drawer opened and focus moved INTO it. -------
    let drawer_open: bool = eval_json(
        &tab,
        "document.getElementById('file-detail-drawer').open === true",
    );
    assert!(
        drawer_open,
        "detail drawer did not open on KI row keyboard activation"
    );

    // The drawer must expose an accessible name via aria-labelledby ->
    // the visible title. We resolve the reference end-to-end (the IDREF
    // must point at an element whose text is non-empty) rather than
    // just asserting the attribute string, so a dangling reference
    // can't pass. This is the assertion that fails on un-fixed source.
    let drawer_named: bool = eval_json(
        &tab,
        "(() => { const d = document.getElementById('file-detail-drawer'); \
             const ref = d.getAttribute('aria-labelledby'); \
             if (!ref) return false; \
             const labelEl = document.getElementById(ref); \
             return !!labelEl && labelEl.textContent.trim().length > 0; })()",
    );
    assert!(
        drawer_named,
        "detail drawer has no resolvable accessible name \
         (aria-labelledby -> non-empty title); screen readers announce \
         it as an unnamed dialog"
    );

    let focus_inside_drawer: bool = eval_json(
        &tab,
        "(() => { const d = document.getElementById('file-detail-drawer'); \
             const a = document.activeElement; \
             return !!a && (a === d || d.contains(a)); })()",
    );
    assert!(
        focus_inside_drawer,
        "focus did not move into the drawer on open — keyboard / screen-reader \
         users are stranded on the occluded trigger row"
    );

    // -- Step 6: close via the × button, assert focus RETURNS to row. ----
    let close_btn = tab
        .find_element("#drawer-close")
        .expect("drawer close button should exist while drawer is open");
    close_btn.click().expect("click drawer close");
    std::thread::sleep(Duration::from_millis(300));

    let focus_restored: bool = eval_json(
        &tab,
        "document.activeElement === document.querySelector('tr[data-focus-trigger-marker]')",
    );
    assert!(
        focus_restored,
        "focus did not return to the trigger row after the drawer closed"
    );
}

/// Team-composition Knowledge-surfaces widget renders the tenure mix from
/// real per-author rows — not the nonexistent `commit_share_pct` /
/// `active_authors` fields — and never leaks the `__summary__` carrier row
/// into the DOM. Guards the exact regression: reading fields absent from
/// `TeamCompositionRow` renders zero-width bars and the literal string
/// "undefined", and iterating the summary sentinel corrupts the bucket mix.
#[test]
fn team_composition_widget_renders_real_buckets_without_undefined() {
    let fixture = delivery_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        window_days: 90,
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let team_composition = run_team_composition(&db, &opts).expect("team-composition");

    // Fail loudly (not vacuously) if the fixture stops producing the shape
    // this test depends on: ≥2 real author rows spanning ≥2 distinct tenure
    // buckets, plus the __summary__ carrier row. Collected as owned `String`s
    // (rather than borrowing `team_composition`) so the checks below don't
    // keep a borrow alive across the later move into `SpaDashboard`.
    let real_author_count = team_composition
        .iter()
        .filter(|r| r.author != "__summary__")
        .count();
    assert!(
        real_author_count >= 2,
        "delivery_repo must produce ≥2 real author rows; got {real_author_count}"
    );
    let distinct_buckets: std::collections::HashSet<String> = team_composition
        .iter()
        .filter(|r| r.author != "__summary__")
        .map(|r| r.bucket.clone())
        .collect();
    assert!(
        distinct_buckets.len() >= 2,
        "delivery_repo must span ≥2 distinct tenure buckets; got {distinct_buckets:?}"
    );
    assert!(
        team_composition.iter().any(|r| r.author == "__summary__"),
        "delivery_repo team-composition must include the __summary__ carrier row"
    );

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        team_composition,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Team Composition Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    let widget_html = tab
        .find_element("#widget-knowledge-surfaces-body")
        .expect("knowledge-surfaces widget container")
        .get_content()
        .expect("widget html");

    assert!(
        !widget_html.contains("undefined"),
        "team-composition widget rendered the literal string 'undefined' — \
         the renderer is reading a field that does not exist on the row. HTML: {widget_html}"
    );
    assert!(
        !widget_html.contains("__summary__"),
        "team-composition widget rendered the __summary__ carrier row: {widget_html}"
    );

    // At least one bucket name from the real rows must appear in the legend.
    for bucket in &distinct_buckets {
        assert!(
            widget_html.contains(bucket.as_str()),
            "expected bucket name {bucket:?} in the rendered legend; HTML: {widget_html}"
        );
    }

    // At least one segment must have a non-zero rendered width (proves the
    // share is computed from real author counts, not a missing field
    // defaulting to 0 for every segment).
    let has_nonzero_segment: bool = eval_json(
        &tab,
        "(function () { \
             var segs = document.querySelectorAll('#widget-knowledge-surfaces-body .team-bar-segment'); \
             for (var i = 0; i < segs.length; i++) { \
                 var w = parseFloat(segs[i].style.width); \
                 if (!isNaN(w) && w > 0) return true; \
             } \
             return false; \
         })()",
    );
    assert!(
        has_nonzero_segment,
        "no .team-bar-segment had a non-zero rendered width; the widget is \
         still rendering zero-width bars"
    );
}

/// Render an SPA from the differential fixture to `html_path`, with
/// every chart-feeding payload field populated so the full widget set
/// actually mounts. Shared by the a11y-interaction tests below so each
/// one doesn't re-spell the ingest + emit boilerplate.
///
/// The differential fixture only ingests history (hotspots / coupling /
/// knowledge-islands), so the trends / calendar / X-Ray / arch-graph /
/// Kamei chart payloads would be empty and their renderers would bail to
/// the empty-state path — leaving too few charts mounted to exercise the
/// text-alternative contract. We synthesise small, well-formed rows for
/// those fields so each renderer reaches its chart-mount path. The values
/// are arbitrary-but-realistic; the assertions only inspect the a11y
/// attributes the renderers stamp, never the chart geometry.
// Long but linear: one synthetic-row block per dashboard field so each dark
// widget branch renders; splitting would scatter the fixture the assertions read.
#[allow(clippy::too_many_lines)]
fn write_smoke_spa(html_path: &std::path::Path, title: &str) {
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");

    // --- Synthetic chart payloads (see doc comment). -----------------
    // Trends: two files over three months.
    let trends: Vec<TrendPoint> = ["2026-01-01", "2026-02-01", "2026-03-01"]
        .iter()
        .enumerate()
        .flat_map(|(i, month)| {
            let step = f64::from(u32::try_from(i).unwrap_or(0));
            [
                ("src/alpha/service.rs", 0.1f64.mul_add(-step, 0.9)),
                ("src/beta/handler.rs", 0.1f64.mul_add(step, 0.4)),
            ]
            .into_iter()
            .map(move |(path, score)| TrendPoint {
                month: (*month).to_string(),
                path: path.to_string(),
                hotspot_score: score,
            })
        })
        .collect();
    // Calendar heatmap: a handful of active days.
    let daily_commits: Vec<DailyCommit> = (1..=8)
        .map(|d| DailyCommit {
            date: format!("2026-01-{d:02}"),
            count: d,
        })
        .collect();
    // X-Ray sunburst: functions across two top-level paths.
    let xray: Vec<XRayEntry> = (0..6)
        .map(|i| XRayEntry {
            path: if i % 2 == 0 {
                "src/alpha/service.rs".to_string()
            } else {
                "src/beta/handler.rs".to_string()
            },
            function: format!("fn_{i}"),
            cognitive: 3.0 + f64::from(i),
            start_line: 1 + i * 10,
            end_line: 9 + i * 10,
        })
        .collect();
    // Arch graph: cross-module import edges.
    let imports: Vec<ImportEdgeRow> = (0..4)
        .map(|i| ImportEdgeRow {
            src_path: format!("src/alpha/mod_{i}.rs"),
            target_path: format!("src/beta/mod_{i}.rs"),
        })
        .collect();
    // Kamei delivery-risk sparkline: a short commit window.
    let kamei_risk: Vec<KameiRiskRow> = (0..10)
        .map(|i| KameiRiskRow {
            rev: format!("{:040x}", i + 1),
            date: format!("2026-02-{:02}", i + 1),
            la: 20 + i,
            ld: 5 + i,
            nf: 1 + i % 4,
            nd: 1 + i % 2,
            ndev: 1 + i % 3,
            nuc: i,
            exp: 100 - i * 5,
            entropy: 0.1 * f64::from(i),
            fix: i % 3 == 0,
        })
        .collect();
    // Entity ownership: one (path, author) row per file.
    let entity_ownership: Vec<EntityOwnershipRow> = [
        ("src/alpha/service.rs", "Alice", 200u64, 40u64),
        ("src/beta/handler.rs", "Bob", 150, 30),
        ("src/alpha/mod_0.rs", "Alice", 80, 10),
        ("src/beta/mod_0.rs", "Bob", 60, 5),
    ]
    .iter()
    .map(|&(entity, author, added, deleted)| EntityOwnershipRow {
        entity: entity.to_string(),
        author: author.to_string(),
        added,
        deleted,
    })
    .collect();
    // Clone groups: two files with clone membership.
    let clones: Vec<CloneSummary> = vec![
        CloneSummary {
            path: "src/alpha/service.rs".to_string(),
            groups: 2,
        },
        CloneSummary {
            path: "src/beta/handler.rs".to_string(),
            groups: 1,
        },
    ];
    // Modularity violations: one co-change pair with no import edge.
    let modularity_violations: Vec<ModularityViolationRow> = vec![ModularityViolationRow {
        entity_a: "src/alpha/service.rs".to_string(),
        entity_b: "src/beta/handler.rs".to_string(),
        shared: 5,
        degree: 0.55,
        fisher_p: 0.02,
    }];
    // Unstable interface: one heavily-imported churning file.
    let unstable_interface: Vec<UnstableInterfaceRow> = vec![UnstableInterfaceRow {
        path: "src/alpha/service.rs".to_string(),
        fan_in: 4,
        revisions: 12,
        coupled_dependents: 3,
        instability_score: 36.0,
    }];
    // Architecture roles: two files classified by role.
    let architecture_roles: Vec<ArchitectureRoleRow> = vec![
        ArchitectureRoleRow {
            path: "src/alpha/service.rs".to_string(),
            role: "shared".to_string(),
            vfi: 8,
            vfo: 2,
            in_cycle: false,
            level: 1,
            reach_pct: 25.0,
        },
        ArchitectureRoleRow {
            path: "src/beta/handler.rs".to_string(),
            role: "periphery".to_string(),
            vfi: 1,
            vfo: 0,
            in_cycle: false,
            level: 0,
            reach_pct: 0.0,
        },
    ];
    // Architecture decay trend: three sampled revisions.
    let architecture_trend: Vec<ArchitectureTrendRow> = vec![
        ArchitectureTrendRow {
            date: "2026-01-01".to_string(),
            rev: "abc123456789".to_string(),
            files: 8,
            propagation_cost: 0.12,
            cycle_count: 0,
            largest_cycle: 0,
        },
        ArchitectureTrendRow {
            date: "2026-02-01".to_string(),
            rev: "def234567890".to_string(),
            files: 10,
            propagation_cost: 0.18,
            cycle_count: 1,
            largest_cycle: 3,
        },
        ArchitectureTrendRow {
            date: "2026-03-01".to_string(),
            rev: "fad345678901".to_string(),
            files: 12,
            propagation_cost: 0.22,
            cycle_count: 2,
            largest_cycle: 4,
        },
    ];
    // MI rollup: one file per band so the KPI tile renders.
    let mi_rollup = Some(MiRollup {
        low: 2,
        moderate: 5,
        high: 3,
        unknown: 1,
    });
    let coupling_density = Some(0.08_f64);
    // Health timeline: four sampled revisions with realistic health scores so
    // `renderHealthTrend` passes its `rows.length < 2` guard and mounts a chart.
    let health_trend: Vec<HealthTrendRow> = vec![
        HealthTrendRow {
            date: "2026-01-01".to_string(),
            rev: "abc123456789".to_string(),
            files: 8,
            arch_health: 72.0,
            code_health: 68.0,
            combined_health: 70.0,
            arch_band: "green".to_string(),
            code_band: "yellow".to_string(),
            combined_band: "yellow".to_string(),
        },
        HealthTrendRow {
            date: "2026-02-01".to_string(),
            rev: "def234567890".to_string(),
            files: 9,
            arch_health: 70.0,
            code_health: 65.0,
            combined_health: 67.5,
            arch_band: "green".to_string(),
            code_band: "yellow".to_string(),
            combined_band: "yellow".to_string(),
        },
        HealthTrendRow {
            date: "2026-03-01".to_string(),
            rev: "fad345678901".to_string(),
            files: 10,
            arch_health: 68.0,
            code_health: 63.0,
            combined_health: 65.5,
            arch_band: "yellow".to_string(),
            code_band: "yellow".to_string(),
            combined_band: "yellow".to_string(),
        },
        HealthTrendRow {
            date: "2026-04-01".to_string(),
            rev: "bce456789012".to_string(),
            files: 11,
            arch_health: 65.0,
            code_health: 60.0,
            combined_health: 62.5,
            arch_band: "yellow".to_string(),
            code_band: "yellow".to_string(),
            combined_band: "yellow".to_string(),
        },
    ];

    // Factor-header tiles: derived from the same `health_trend` sample
    // above via the real production function, so the Code/Architecture
    // tiles (and their jump-link `data-target`s) render exactly as they
    // would from a real CLI run rather than from hand-authored literals.
    let factors = health_trend_factors(&health_trend);

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        trends,
        daily_commits,
        xray,
        imports,
        kamei_risk,
        entity_ownership,
        clones,
        modularity_violations,
        unstable_interface,
        architecture_roles,
        architecture_trend,
        health_trend,
        mi_rollup,
        coupling_density,
        factors,
        effort_exposure: vec![
            EffortExposureRow {
                band: "red".into(),
                files: 2,
                loc_share_pct: 18.0,
                commit_share_pct: 35.0,
                churn_share_pct: 30.0,
                commit_share_ci_low: 0.22,
                commit_share_ci_high: 0.50,
                churn_share_improving_pct: None,
                churn_share_degrading_pct: None,
            },
            EffortExposureRow {
                band: "green".into(),
                files: 6,
                loc_share_pct: 82.0,
                commit_share_pct: 65.0,
                churn_share_pct: 70.0,
                commit_share_ci_low: 0.54,
                commit_share_ci_high: 0.74,
                churn_share_improving_pct: None,
                churn_share_degrading_pct: None,
            },
        ],
        ..SpaDashboard::default()
    };

    let mut f = std::fs::File::create(html_path).expect("create html");
    write_spa(
        &dash,
        title,
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);
}

/// Launch headless Chrome, open a tab on `file://<html_path>`, and give
/// Alpine + the cooperative widget-boot loop the standard 2-second settle
/// window every interaction test relies on. Returns `None` — after printing
/// the standard skip line — when Chrome can't be launched, so a contributor
/// machine without Chrome `return`s cleanly instead of panicking. The
/// `Browser` is handed back alongside the tab; callers must keep it bound
/// (dropping it would close the tab).
fn boot_spa_tab(html_path: &std::path::Path) -> Option<(Browser, Arc<headless_chrome::Tab>)> {
    let browser = match Browser::default() {
        Ok(b) => b,
        Err(e) => {
            // On CI the skip must be a FAILURE: this suite is the only
            // place the dashboard's JS executes at all, and a broken
            // Chrome install used to turn the whole job into a silent
            // 2-minute green — the exact hole its own module doc blames
            // for shipped JS defects. The env var is set by the CI job;
            // contributor machines without Chrome still skip cleanly.
            assert!(
                std::env::var("CODELORE_REQUIRE_BROWSER").is_err(),
                "CODELORE_REQUIRE_BROWSER is set but Chrome failed to \
                 launch ({e}) — a browser-required environment must fail, \
                 not silently skip the only JS coverage"
            );
            println!(
                "spa_browser_test: skipping — could not launch Chrome ({e}). \
                 Install Chrome / Chromium and retry."
            );
            return None;
        }
    };
    let tab = browser.new_tab().expect("new tab");
    let url = format!("file://{}", html_path.display());
    tab.navigate_to(&url).expect("navigate");
    tab.wait_until_navigated().expect("wait navigation");
    std::thread::sleep(Duration::from_secs(2));
    Some((browser, tab))
}

/// Evaluate `js` in the page and deserialize the returned JSON value into
/// `T`. Collapses the `evaluate → .value.expect → serde_json::from_value`
/// readback ladder every assertion repeats.
fn eval_json<T: serde::de::DeserializeOwned>(tab: &headless_chrome::Tab, js: &str) -> T {
    let value = tab
        .evaluate(js, false)
        .expect("evaluate js")
        .value
        .expect("js returned a value");
    serde_json::from_value(value).expect("deserialize js result")
}

/// Trimmed `textContent` of `#id`, or the empty string when the element is
/// absent. A thin, named specialization of [`eval_json`] over the
/// `getElementById(...).textContent` readback that interaction assertions
/// (toggle labels, legends) otherwise repeat inline.
fn element_text(tab: &headless_chrome::Tab, id: &str) -> String {
    eval_json(
        tab,
        &format!(
            "(function () {{ \
                 var el = document.getElementById('{id}'); \
                 return el ? el.textContent.trim() : ''; \
             }})()"
        ),
    )
}

/// Number of data points in the first `ECharts` series mounted into `#host_id`,
/// or `-1` when neither the chart nor `ECharts` itself has mounted. Lets a test
/// prove a re-render *changed* a matrix (cell count moved) rather than merely
/// re-painting it.
fn echarts_series_len(tab: &headless_chrome::Tab, host_id: &str) -> i64 {
    eval_json(
        tab,
        &format!(
            "(function () {{ \
                 var el = document.getElementById('{host_id}'); \
                 if (!el || !window.echarts) return -1; \
                 var chart = window.echarts.getInstanceByDom(el); \
                 if (!chart) return -1; \
                 var opt = chart.getOption(); \
                 var d = opt && opt.series && opt.series[0] && opt.series[0].data; \
                 return d ? d.length : -1; \
             }})()"
        ),
    )
}

/// Enable the CDP `Log`/`Runtime` domains and attach a listener that captures
/// every `RuntimeExceptionThrown` into the returned sink, so a test can assert
/// an interaction produced no uncaught browser-side exceptions.
fn attach_exception_sink(tab: &headless_chrome::Tab) -> Arc<Mutex<Vec<String>>> {
    let console_errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&console_errors);
    tab.enable_log().expect("enable log");
    tab.enable_runtime().expect("enable runtime");
    let listener = move |event: &Event| {
        if let Event::RuntimeExceptionThrown(thrown) = event {
            sink.lock()
                .expect("console mutex")
                .push(thrown.params.exception_details.text.clone());
        }
    };
    tab.add_event_listener(Arc::new(listener))
        .expect("add event listener");
    console_errors
}

/// `clientHeight` (layout-box height, px) of `#id`, or `-1` when the element
/// is absent. Lets a test prove a container GREW to contain its chart — the
/// widget-body-vs-chart-host sizing contract the DSM matrix must hold at any
/// module count.
fn client_height(tab: &headless_chrome::Tab, id: &str) -> i64 {
    eval_json(
        tab,
        &format!(
            "(function () {{ \
                 var el = document.getElementById('{id}'); \
                 return el ? el.clientHeight : -1; \
             }})()"
        ),
    )
}

/// Replicates `resizeAllEchartsIn`'s DOM sweep with `selector` and reports
/// whether it reaches a mounted `ECharts` instance on the element with id
/// `target_id`. The fullscreen resize path queries this exact selector shape
/// and calls `resize()` on each match's chart instance, so a chart on a
/// nested host is only reached when the selector descends to it.
fn sweep_reaches_chart(tab: &headless_chrome::Tab, selector: &str, target_id: &str) -> bool {
    eval_json(
        tab,
        &format!(
            "(function () {{ \
                 if (!window.echarts) return false; \
                 var els = document.querySelectorAll('{selector}'); \
                 for (var i = 0; i < els.length; i++) {{ \
                     if (els[i].id === '{target_id}' \
                         && window.echarts.getInstanceByDom(els[i])) return true; \
                 }} \
                 return false; \
             }})()"
        ),
    )
}

/// Overrides the tab's rendered viewport via CDP `Emulation.setDeviceMetricsOverride`
/// so `window.innerWidth`/`innerHeight` — and every CSS media query keyed
/// off them, including the `.dash-group-grid` 1280px breakpoint — see
/// exactly `w`×`h`, independent of the host machine's real Chrome window
/// size. Geometry tests use this to prove the responsive grid reflows at a
/// specific breakpoint from measured layout, not from asserting a class
/// name.
fn set_viewport(tab: &headless_chrome::Tab, w: u32, h: u32) {
    tab.call_method(Emulation::SetDeviceMetricsOverride {
        width: w,
        height: h,
        device_scale_factor: 1.0,
        mobile: false,
        scale: None,
        screen_width: None,
        screen_height: None,
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })
    .expect("set device metrics override");
}

/// `(top, left, width)` from `getBoundingClientRect()` of the first element
/// matching `selector`, or all `-1.0` when nothing matches. Backs the
/// responsive-geometry proofs below: they measure real rendered geometry —
/// not class names — to confirm the section grid actually reflows at a
/// given viewport width. Marshals the rect through `JSON.stringify` because
/// CDP `Runtime.evaluate` only inlines a `value` for primitive results
/// (`eval_json`'s contract); an array comes back as an object reference
/// without one, so a plain `[top, left, width]` return would leave
/// `eval_json` with nothing to deserialize.
fn bounding_rect(tab: &headless_chrome::Tab, selector: &str) -> (f64, f64, f64) {
    let json: String = eval_json(
        tab,
        &format!(
            "(function () {{ \
                 var el = document.querySelector('{selector}'); \
                 if (!el) return JSON.stringify([-1, -1, -1]); \
                 var r = el.getBoundingClientRect(); \
                 return JSON.stringify([r.top, r.left, r.width]); \
             }})()"
        ),
    );
    serde_json::from_str(&json).expect("parse bounding rect JSON")
}

/// A `role="tablist"` must support arrow-key navigation per the
/// WAI-ARIA Tabs pattern: focus a tab, press `ArrowRight`, and focus +
/// the selected state move to the next tab. On the un-fixed source the
/// tabs carry `role="tab"` + `aria-selected` but no keyboard handler,
/// so focus stays put and the selection never advances — this test
/// fails until `wireTablistArrows` is wired at boot.
#[test]
fn tablist_arrow_keys_move_focus_and_selection() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    write_smoke_spa(&html_path, "CodeLore Tablist Keyboard Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // Use the hotspot color-mode tablist — it always renders (static
    // markup, no data dependency) and its first tab starts selected.
    // Mark the first two tabs so we can assert identity across evaluate
    // calls, focus the first, then press ArrowRight.
    let setup_ok: bool = eval_json(
        &tab,
        "(() => { const bar = document.getElementById('hotspot-color-toggles'); \
             if (!bar) return false; \
             const tabs = bar.querySelectorAll('[role=\"tab\"]'); \
             if (tabs.length < 2) return false; \
             tabs[0].setAttribute('data-kb-first', '1'); \
             tabs[1].setAttribute('data-kb-second', '1'); \
             tabs[0].focus(); \
             return document.activeElement === tabs[0] && \
                    tabs[0].getAttribute('tabindex') === '0'; })()",
    );
    assert!(
        setup_ok,
        "could not focus the first tab with roving tabindex=0; \
         either the tablist is missing or wireTablistArrows did not run"
    );

    // ArrowRight: focus + selection must advance to the second tab.
    tab.evaluate(
        "(() => { const t = document.querySelector('[data-kb-first]'); \
         t.dispatchEvent(new KeyboardEvent('keydown', \
         { key: 'ArrowRight', bubbles: true })); })()",
        false,
    )
    .expect("dispatch ArrowRight");
    // Selection is driven through the tab's click handler (imperative on
    // this tablist, Alpine-reactive on the others); a short settle covers
    // either propagation path.
    std::thread::sleep(Duration::from_millis(200));

    let focus_moved: bool = eval_json(
        &tab,
        "document.activeElement === document.querySelector('[data-kb-second]')",
    );
    assert!(
        focus_moved,
        "ArrowRight did not move focus to the next tab — tablist has no \
         arrow-key navigation"
    );

    let selection_moved: bool = eval_json(
        &tab,
        "(() => { const first = document.querySelector('[data-kb-first]'); \
             const second = document.querySelector('[data-kb-second]'); \
             return second.getAttribute('aria-selected') === 'true' && \
                    first.getAttribute('aria-selected') === 'false' && \
                    second.getAttribute('tabindex') === '0' && \
                    first.getAttribute('tabindex') === '-1'; })()",
    );
    assert!(
        selection_moved,
        "ArrowRight moved focus but not the aria-selected / roving-tabindex \
         state to the next tab"
    );
}

/// The hotspot file list (`role="tree"`) is a parallel DOM structure next
/// to the circle-pack canvas, offered as a keyboard/screen-reader
/// alternative. Confirms `wireTreeArrows` (`00_setup_boot.js`) gives it real
/// WAI-ARIA treeview keyboard semantics:
///   - `ArrowDown` moves focus AND the roving tabindex to the next
///     treeitem, but — unlike the tablist pattern above — does NOT also
///     activate it (arrow keys only move focus per the APG treeview
///     pattern; there is no wraparound either).
///   - `Enter` on the focused treeitem still opens the file-detail drawer
///     through the existing inline `_codeloreShowDetail` binding.
///
/// On the un-fixed source every treeitem is `tabindex="0"` with no
/// keydown handler on the tree, so `ArrowDown` does nothing and this test
/// fails.
#[test]
fn hotspot_tree_arrow_keys_move_focus_and_enter_opens_drawer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-tree-keyboard.html");
    write_smoke_spa(&html_path, "CodeLore Tree Keyboard Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    // The keyboard-accessible file list lives inside a closed-by-default
    // `<details>` — native `<details>` hides its content subtree
    // (including from focus) while collapsed, so force it open before
    // driving the tree with focus/keydown. Opening and focusing are two
    // separate round trips (with a real settle in between) because
    // DaisyUI's collapse reveal is a CSS transition — focusing in the
    // same synchronous script as the `open = true` write races the
    // browser's style recalc and silently no-ops.
    tab.evaluate(
        "(() => { const menu = document.querySelector('[role=\"tree\"]'); \
             const details = menu && menu.closest('details'); \
             if (details) details.open = true; \
             const items = menu ? menu.querySelectorAll('[role=\"treeitem\"]') : []; \
             if (items[0]) items[0].setAttribute('data-kb-first', '1'); \
             if (items[1]) items[1].setAttribute('data-kb-second', '1'); \
         })()",
        false,
    )
    .expect("open tree details + mark rows");
    std::thread::sleep(Duration::from_millis(200));

    let setup_ok: bool = eval_json(
        &tab,
        "(() => { const first = document.querySelector('[data-kb-first]'); \
             const second = document.querySelector('[data-kb-second]'); \
             if (!first || !second) return false; \
             first.focus(); \
             return document.activeElement === first && \
                    first.getAttribute('tabindex') === '0' && \
                    second.getAttribute('tabindex') === '-1'; })()",
    );
    assert!(
        setup_ok,
        "could not focus the first treeitem with roving tabindex=0; \
         either the hotspot tree has fewer than 2 rows or the initial \
         roving-tabindex binding is wrong"
    );

    // ArrowDown: focus + roving tabindex move to the second item, but the
    // row must NOT activate (no drawer open) — arrow keys only move focus
    // per the WAI-ARIA treeview pattern.
    tab.evaluate(
        "document.querySelector('[data-kb-first]').dispatchEvent(\
             new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))",
        false,
    )
    .expect("dispatch ArrowDown");
    std::thread::sleep(Duration::from_millis(200));

    let focus_moved: bool = eval_json(
        &tab,
        "(() => { const first = document.querySelector('[data-kb-first]'); \
             const second = document.querySelector('[data-kb-second]'); \
             return document.activeElement === second && \
                    second.getAttribute('tabindex') === '0' && \
                    first.getAttribute('tabindex') === '-1'; })()",
    );
    assert!(
        focus_moved,
        "ArrowDown did not move focus + roving tabindex to the next \
         treeitem — the tree has no arrow-key navigation"
    );

    let drawer_still_closed: bool = eval_json(
        &tab,
        "document.getElementById('file-detail-drawer').open !== true",
    );
    assert!(
        drawer_still_closed,
        "ArrowDown must only move focus (WAI-ARIA treeview pattern) — it \
         must not also activate the row and open the drawer"
    );

    // Enter on the now-focused (second) item activates it via the
    // existing inline `_codeloreShowDetail` binding.
    tab.evaluate(
        "document.querySelector('[data-kb-second]').dispatchEvent(\
             new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))",
        false,
    )
    .expect("dispatch Enter");
    std::thread::sleep(Duration::from_millis(300));

    let drawer_opened: bool = eval_json(
        &tab,
        "document.getElementById('file-detail-drawer').open === true",
    );
    assert!(
        drawer_opened,
        "Enter on the focused treeitem did not open the file-detail drawer"
    );

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "tree keyboard nav produced {} browser-console error(s):\n{}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// Minimal `HotspotRow` for the badge/tree tests: only the fields the keyboard
/// list and circle-pack read. `cognitive_health` is the inline `[60, 100]`
/// proxy that the badge must NOT be sourced from.
fn synth_hotspot(path: &str, cognitive_health: f64, hotspot_score: f64) -> HotspotRow {
    HotspotRow {
        path: path.to_string(),
        revisions: 5,
        cognitive: 10.0,
        cognitive_health,
        hotspot_score,
        mi: None,
        mi_rank: None,
        ai_pct: None,
        hotspot_score_anchored: None,
    }
}

/// Minimal `CodeHealthRow` carrying the composite `band` + `score` the keyboard
/// list must badge from. `band` is the authoritative composite signal; `score`
/// is the 0–100 health number shown as the badge text.
fn synth_code_health(path: &str, band: &str, score: f64, structural_risk: f64) -> CodeHealthRow {
    CodeHealthRow {
        path: path.to_string(),
        cognitive: 10.0,
        score,
        structural_risk,
        percentile: 0.5,
        band: band.to_string(),
        corpus_percentile: None,
        beyond_corpus: false,
        corpus_percentile_ci_low: None,
        corpus_percentile_ci_high: None,
    }
}

/// The keyboard-accessible file list is the declared a11y alternative to the
/// canvas health lens, which colours by the COMPOSITE `code_health` band. The
/// list must badge from that same band — not the `cognitive_health` proxy,
/// which is arithmetically bounded to `[60, 100]`, so a proxy `≤ 40 → red`
/// branch is unreachable and screen-reader users would never see a red file
/// the sighted lens shows. This payload is built so the composite band and the
/// proxy DISAGREE: every file's `cognitive_health` sits in `[60, 100]` (the old
/// cut could only ever emit warning/success, never error), while the composite
/// bands include a red file, two yellow, two green, and one hotspot with no
/// composite row at all. The rendered badge distribution must follow the
/// composite, and the red file's badge text must be the composite score, not
/// the proxy — the exact reading the old code structurally could not produce.
#[test]
#[allow(clippy::too_many_lines)] // mirror of the other browser tests' payload + assertion shape
fn hotspot_tree_badges_composite_code_health_band_not_cognitive_proxy() {
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");

    // Real summary / coupling / knowledge-islands give a clean-booting baseline
    // (four other browser tests boot this same fixture without console errors);
    // only hotspots + code_health are overridden so the badge source is under
    // full control.
    let summary = run_summary(&db, &opts).expect("summary");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");

    // Every cognitive_health in [60, 100]: the OLD proxy cut
    // (≤40 error / ≤70 warning / else success) can only emit warning or
    // success here — never error. Any badge-error the list shows is
    // structurally impossible under the old code.
    let hotspots = vec![
        synth_hotspot("src/pkg/red_file.rs", 62.0, 9.0),
        synth_hotspot("src/pkg/yellow_one.rs", 75.0, 8.0),
        synth_hotspot("src/pkg/yellow_two.rs", 80.0, 7.0),
        synth_hotspot("src/pkg/green_one.rs", 95.0, 6.0),
        synth_hotspot("src/pkg/green_two.rs", 90.0, 5.0),
        synth_hotspot("src/pkg/no_composite.rs", 70.0, 4.0),
    ];
    // Composite bands DISAGREE with the proxy: one red, two yellow, two green.
    // src/pkg/no_composite.rs is deliberately ABSENT here → "no data" badge.
    let code_health = vec![
        synth_code_health("src/pkg/red_file.rs", "red", 30.0, 0.80),
        synth_code_health("src/pkg/yellow_one.rs", "yellow", 55.0, 0.40),
        synth_code_health("src/pkg/yellow_two.rs", "yellow", 60.0, 0.35),
        synth_code_health("src/pkg/green_one.rs", "green", 85.0, 0.10),
        synth_code_health("src/pkg/green_two.rs", "green", 90.0, 0.05),
    ];

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-tree-badge.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Tree Badge Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    // Open the collapsed <details> so the tree content is live, then settle.
    tab.evaluate(
        "(() => { const menu = document.querySelector('[role=\"tree\"]'); \
             const d = menu && menu.closest('details'); if (d) d.open = true; })()",
        false,
    )
    .expect("open tree details");
    std::thread::sleep(Duration::from_millis(200));

    let counts_json: String = eval_json(
        &tab,
        "(function () { \
             var menu = document.querySelector('[role=\"tree\"]'); \
             if (!menu) return JSON.stringify({ rows: -1 }); \
             var badges = menu.querySelectorAll('[role=\"treeitem\"] span.badge'); \
             var c = { error: 0, warning: 0, success: 0, ghost: 0, other: 0, rows: badges.length }; \
             for (var i = 0; i < badges.length; i++) { \
                 var cl = badges[i].classList; \
                 if (cl.contains('badge-error')) c.error++; \
                 else if (cl.contains('badge-warning')) c.warning++; \
                 else if (cl.contains('badge-success')) c.success++; \
                 else if (cl.contains('badge-ghost')) c.ghost++; \
                 else c.other++; \
             } \
             return JSON.stringify(c); \
         })()",
    );
    let counts: serde_json::Value = serde_json::from_str(&counts_json).expect("counts json");

    // Distribution must match the COMPOSITE bands embedded above, not the proxy.
    assert_eq!(
        counts["rows"], 6,
        "expected 6 keyboard-list rows (top-50 by hotspot_score); counts={counts}"
    );
    assert_eq!(
        counts["error"], 1,
        "the composite band has exactly one red file, so the keyboard list must \
         show one badge-error. The old cognitive_health proxy (bounded [60,100]) \
         could NEVER emit badge-error; counts={counts}"
    );
    assert_eq!(
        counts["warning"], 2,
        "the composite band has two yellow files; counts={counts}"
    );
    assert_eq!(
        counts["success"], 2,
        "the composite band has two green files; counts={counts}"
    );
    assert_eq!(
        counts["ghost"], 1,
        "the hotspot with no composite code_health row must badge as 'no data' \
         (badge-ghost); counts={counts}"
    );
    assert_eq!(
        counts["other"], 0,
        "every badge must be one of error/warning/success/ghost; counts={counts}"
    );

    // The red file's badge TEXT must be the composite score (30), not the
    // cognitive_health proxy (62) — proving both colour AND number now come
    // from data.code_health.
    let red_badge_text: String = eval_json(
        &tab,
        "(function () { \
             var items = document.querySelectorAll('[role=\"treeitem\"]'); \
             for (var i = 0; i < items.length; i++) { \
                 var pathEl = items[i].querySelector('.truncate'); \
                 if (pathEl && pathEl.textContent.indexOf('red_file.rs') >= 0) { \
                     var b = items[i].querySelector('span.badge'); \
                     return b ? b.textContent.trim() : ''; \
                 } \
             } \
             return ''; \
         })()",
    );
    assert_eq!(
        red_badge_text, "30",
        "the red file's badge text must be the composite health score (30), not \
         the cognitive_health proxy (62); got {red_badge_text:?}"
    );

    // The legacy hand-rolled `.badge` rule in `template.html` is unlayered
    // while DaisyUI's own badge styles are `@layer`ed — per CSS Cascade 5,
    // ANY property the unlayered rule declares beats DaisyUI's semantic
    // `badge-error` / `badge-warning` / `badge-success` regardless of
    // specificity, so a legacy rule that painted color/background/border
    // would render every badge identically, no matter its modifier class.
    // Prove the fix by comparing the red file's `badge-error` element's
    // computed background against a freshly-injected PLAIN `.badge` (no
    // color modifier, DaisyUI's own default): under the bug this fixed,
    // both were forced to the same hardcoded green and would compare equal.
    let badge_colors_json: String = eval_json(
        &tab,
        "(function () { \
             var errorBadge = null; \
             var items = document.querySelectorAll('[role=\"treeitem\"]'); \
             for (var i = 0; i < items.length; i++) { \
                 var pathEl = items[i].querySelector('.truncate'); \
                 if (pathEl && pathEl.textContent.indexOf('red_file.rs') >= 0) { \
                     errorBadge = items[i].querySelector('span.badge'); \
                     break; \
                 } \
             } \
             if (!errorBadge) return JSON.stringify({ ok: false }); \
             var probe = document.createElement('span'); \
             probe.className = 'badge'; \
             document.body.appendChild(probe); \
             var errorBg = getComputedStyle(errorBadge).backgroundColor; \
             var plainBg = getComputedStyle(probe).backgroundColor; \
             probe.remove(); \
             return JSON.stringify({ ok: true, errorBg: errorBg, plainBg: plainBg }); \
         })()",
    );
    let badge_colors: serde_json::Value =
        serde_json::from_str(&badge_colors_json).expect("badge colors json");
    assert_eq!(
        badge_colors["ok"], true,
        "must locate the red file's badge-error element: {badge_colors}"
    );
    assert_ne!(
        badge_colors["errorBg"], badge_colors["plainBg"],
        "a badge-error badge must NOT render the same computed background as a \
         plain, unmodified .badge — an unlayered legacy rule painting color \
         regardless of the DaisyUI modifier class would make every badge the \
         same colour: {badge_colors}"
    );

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "tree badge test produced {} uncaught browser exception(s):\n  {}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// A missing or truncated `#codelore-data` block must not render as a
/// confident, fully-chromed, entirely EMPTY dashboard — the state that reads
/// as "this repo has no findings" when the truth is "the data never arrived".
/// Both boot guards must replace `<main>` with a `role="alert"` banner that
/// names the condition and the remedy, leaving zero widget bodies behind.
/// Covers BOTH guard paths: unparseable JSON and an absent data block.
#[test]
fn broken_data_block_shows_alert_banner_and_no_widgets() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let good = tmp.path().join("codelore-good.html");
    write_smoke_spa(&good, "CodeLore Broken Data Test");
    let html = std::fs::read_to_string(&good).expect("read good html");

    // -- Case A: truncated payload → JSON.parse throws (parse-failure guard). --
    // Replace the data block's inner JSON with an unterminated fragment, exactly
    // as a half-written CI upload or a truncated download would leave it.
    let open = "id=\"codelore-data\">";
    let start = html.find(open).expect("data block open tag") + open.len();
    let close_rel = html[start..]
        .find("</script>")
        .expect("data block close tag");
    let truncated = format!(
        "{}\n{{\"partial\": true, \"data\":\n  {}",
        &html[..start],
        &html[start + close_rel..],
    );
    let trunc_path = tmp.path().join("codelore-truncated.html");
    std::fs::write(&trunc_path, &truncated).expect("write truncated html");
    assert_banner_and_no_widgets(&trunc_path, "truncated or corrupt");

    // -- Case B: absent data block → getElementById returns null (missing guard). --
    // Rename the id so #codelore-data no longer resolves.
    let missing = html.replace("id=\"codelore-data\"", "id=\"codelore-data-removed\"");
    let missing_path = tmp.path().join("codelore-missing.html");
    std::fs::write(&missing_path, &missing).expect("write missing html");
    assert_banner_and_no_widgets(&missing_path, "missing");
}

/// Boot `html_path` and assert the boot-failure UX: a `role="alert"` banner
/// inside `<main>` whose text contains `condition_phrase` and the regenerate
/// remedy, and ZERO widget-body containers (the banner replaced main's content,
/// so the empty-but-chromed dashboard is gone). Skips cleanly without Chrome.
fn assert_banner_and_no_widgets(html_path: &std::path::Path, condition_phrase: &str) {
    let Some((_browser, tab)) = boot_spa_tab(html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    let banner_present: bool = eval_json(
        &tab,
        "!!document.querySelector('main [role=\"alert\"].codelore-boot-error')",
    );
    assert!(
        banner_present,
        "no role=alert boot-error banner inside <main> for a broken data block — \
         the dashboard rendered empty chrome with no visible failure"
    );

    let banner_text: String = eval_json(
        &tab,
        "(function () { var b = document.querySelector('main [role=\"alert\"]'); \
             return b ? b.textContent : ''; })()",
    );
    assert!(
        banner_text.contains(condition_phrase),
        "boot-error banner did not name the condition ({condition_phrase:?}); \
         text={banner_text:?}"
    );
    assert!(
        banner_text.contains("Regenerate") && banner_text.contains("--format spa"),
        "boot-error banner did not state the regenerate remedy; text={banner_text:?}"
    );

    let widget_bodies: i64 = eval_json(
        &tab,
        "document.querySelectorAll('[id^=\"widget-\"][id$=\"-body\"]').length",
    );
    assert_eq!(
        widget_bodies, 0,
        "broken data block still left {widget_bodies} widget-body container(s) in \
         the DOM — the banner must REPLACE main's content, not sit above a \
         chromed-but-empty dashboard"
    );

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "boot-error path produced {} uncaught browser exception(s):\n  {}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// Clicking a sticky-nav chip scrolls its section into view via
/// `scrollIntoView` — never by mutating `location.hash` (the SPA owns the
/// hash as its state serializer; anchor-style navigation would corrupt
/// it). Also proves the scrollspy highlight moves off the overview chip
/// and onto the clicked one, with zero uncaught console exceptions.
#[test]
fn nav_chip_scrolls_section_into_view_without_hash() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-nav-chip.html");
    write_smoke_spa(&html_path, "CodeLore Nav Chip Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    let hash_before: String = eval_json(&tab, "location.hash");

    tab.evaluate(
        "document.querySelector('.dash-nav-chip[data-target=\"group-architecture\"]').click()",
        false,
    )
    .expect("click architecture chip");

    // Smooth `scrollIntoView` duration scales with distance; poll until
    // the section has actually settled into view instead of a fixed sleep.
    let mut rect_top = f64::MAX;
    let mut viewport_h = 0.0f64;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        rect_top = eval_json(
            &tab,
            "document.getElementById('group-architecture').getBoundingClientRect().top",
        );
        viewport_h = eval_json(&tab, "window.innerHeight");
        if rect_top >= 0.0 && rect_top < viewport_h {
            break;
        }
    }
    assert!(
        rect_top >= 0.0 && rect_top < viewport_h,
        "group-architecture's top ({rect_top}) never settled within the \
         viewport (height {viewport_h})"
    );

    let scroll_y: f64 = eval_json(&tab, "window.scrollY");
    assert!(scroll_y > 0.0, "clicking the chip did not scroll the page");

    let hash_after: String = eval_json(&tab, "location.hash");
    assert_eq!(
        hash_before, hash_after,
        "chip click must never mutate location.hash — the SPA owns it as \
         its state serializer"
    );

    let arch_active: bool = eval_json(
        &tab,
        "document.querySelector('.dash-nav-chip[data-target=\"group-architecture\"]')\
             .classList.contains('dash-active')",
    );
    let overview_active: bool = eval_json(
        &tab,
        "document.querySelector('.dash-nav-chip[data-target=\"group-overview\"]')\
             .classList.contains('dash-active')",
    );
    assert!(arch_active, "clicked chip did not gain the active class");
    assert!(!overview_active, "overview chip is still marked active");

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "nav chip click produced {} browser-console error(s):\n{}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// The four factor tiles double as jump links to their sections, using
/// the same `scrollIntoView` path as the nav chips. This drives the
/// Architecture tile (rendered by `write_smoke_spa`'s real
/// `health_trend_factors` output) via the keyboard — Enter must activate
/// it exactly like a click, scrolling `group-architecture` into view
/// without ever touching `location.hash`.
#[test]
fn factor_tile_is_a_keyboard_activatable_jump_link() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-factor-tile.html");
    write_smoke_spa(&html_path, "CodeLore Factor Tile Jump Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    let tile_selector =
        "document.querySelector('.factor-tile[data-target=\"group-architecture\"]')";

    let tile_role: String = eval_json(
        &tab,
        &format!("({tile_selector}).getAttribute('role') || ''"),
    );
    assert_eq!(tile_role, "link", "factor tile must carry role=\"link\"");

    let cursor: String = eval_json(&tab, &format!("getComputedStyle({tile_selector}).cursor"));
    assert_eq!(cursor, "pointer", "factor tile must show a pointer cursor");

    let hash_before: String = eval_json(&tab, "location.hash");
    tab.evaluate(
        &format!(
            "(() => {{ const t = {tile_selector}; t.focus(); \
                 t.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true }})); \
             }})()"
        ),
        false,
    )
    .expect("dispatch Enter on the factor tile");

    let mut rect_top = f64::MAX;
    let mut viewport_h = 0.0f64;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        rect_top = eval_json(
            &tab,
            "document.getElementById('group-architecture').getBoundingClientRect().top",
        );
        viewport_h = eval_json(&tab, "window.innerHeight");
        if rect_top >= 0.0 && rect_top < viewport_h {
            break;
        }
    }
    assert!(
        rect_top >= 0.0 && rect_top < viewport_h,
        "Enter on the Architecture factor tile never scrolled group-architecture \
         into view (top {rect_top}, viewport {viewport_h})"
    );

    let hash_after: String = eval_json(&tab, "location.hash");
    assert_eq!(
        hash_before, hash_after,
        "factor-tile Enter activation must never mutate location.hash"
    );

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "factor tile keyboard activation produced {} browser-console error(s):\n{}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// Chart containers (`ECharts` / d3 canvases) carry no text alternative
/// on the un-fixed source — `role="img"` count is 0, so a screen reader
/// announces them as empty regions. Each chart renderer must, after its
/// data is computed, stamp `role="img"` + a non-empty `aria-label` on
/// its container. This asserts at least eight chart containers expose
/// that contract after boot.
#[test]
fn chart_containers_expose_text_alternative() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    write_smoke_spa(&html_path, "CodeLore Chart A11y Test");

    // Charts mount across the cooperative boot loop; the helper's 2s settle
    // gives the slowest a window before we count labelled containers.
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // Count chart-body containers that expose BOTH role="img" and a
    // non-empty aria-label. Scoped to `.widget-body` so we don't count
    // incidental img-role nodes elsewhere.
    let labelled_count: i64 = eval_json(
        &tab,
        "Array.from(document.querySelectorAll('.widget-body[role=\"img\"]')) \
             .filter(el => (el.getAttribute('aria-label') || '').trim().length > 0) \
             .length",
    );
    assert!(
        labelled_count >= 8,
        "expected >=8 chart containers with role=img + non-empty aria-label, \
         found {labelled_count}; chart renderers are not stamping text alternatives"
    );
}

/// The hotspot-table filter summary ("N of M rows shown") updates
/// silently on the un-fixed source — a screen reader never hears the
/// row count change. It must be a live region (`aria-live="polite"` +
/// `role="status"`) so assistive tech announces filter results.
#[test]
fn hotspot_table_summary_is_a_live_region() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    write_smoke_spa(&html_path, "CodeLore Live-Region Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    let is_live_region: bool = eval_json(
        &tab,
        "(() => { const el = document.getElementById('hotspot-table-summary'); \
             return !!el && el.getAttribute('aria-live') === 'polite' && \
                    el.getAttribute('role') === 'status'; })()",
    );
    assert!(
        is_live_region,
        "hotspot-table summary is not a polite live region; filter-count \
         updates are silent to screen readers"
    );
}

#[test]
fn detail_drawer_never_renders_empty_for_a_pathless_row() {
    // Defensive guard: even when a row resolves to an empty/missing path
    // (a malformed entry with neither `path` nor `entity`), the drawer must
    // still open with a non-empty title and an explanatory body — never a
    // totally-blank, titleless popup. Without the guard the title is set to
    // the empty string and this assertion fails.
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    write_smoke_spa(&html_path, "CodeLore Empty-Path Drawer Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // Open the drawer with an EMPTY path — models a row whose path/entity
    // field was missing, the scenario that produced a blank popup.
    tab.evaluate("window._codeloreShowDetail('')", false)
        .expect("invoke detail with empty path");
    std::thread::sleep(Duration::from_millis(300));

    let title_nonempty: bool = eval_json(
        &tab,
        "document.getElementById('drawer-title').textContent.trim().length > 0",
    );
    assert!(
        title_nonempty,
        "drawer title is blank for a pathless row — the popup renders empty"
    );

    let body_nonempty: bool = eval_json(
        &tab,
        "document.getElementById('drawer-body').textContent.trim().length > 0",
    );
    assert!(
        body_nonempty,
        "drawer body is blank for a pathless row — the popup renders empty"
    );
}

#[test]
fn detail_drawer_content_is_opaque_when_open() {
    // The drawer content lives in a DaisyUI `.modal-box`, which ships
    // `opacity: 0` and only fades to 1 via a `.modal.modal-open` ancestor
    // this drawer deliberately drops (for positioning). Without an explicit
    // override the content renders fully TRANSPARENT — a blank popup (black
    // in dark mode) even though the DOM is populated. Every other drawer
    // test checks DOM content, not pixels, so this class of bug slipped
    // through; assert the box is actually opaque when the drawer opens.
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    write_smoke_spa(&html_path, "CodeLore Drawer Visibility Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    tab.evaluate("window._codeloreShowDetail('any/path')", false)
        .expect("open the drawer");
    std::thread::sleep(Duration::from_millis(400));

    let box_opaque: bool = eval_json(
        &tab,
        "(() => { const b = document.querySelector('#file-detail-drawer .modal-box'); \
             return !!b && getComputedStyle(b).opacity === '1'; })()",
    );
    assert!(
        box_opaque,
        "drawer .modal-box is not opaque (opacity != 1) — the populated \
         content is invisible, which reads as a blank popup"
    );
}

/// In module-depth mode the coupling sankey names nodes by the first two
/// path segments (e.g. `src/alpha/svc.rs` → `src/alpha`). When the
/// selection store is set to a full file path the subscriber must highlight
/// the node whose name is the file's module prefix — not the raw path.
/// This test uses a fixture that guarantees cross-module co-changes so the
/// depth-2 sankey is always populated; a missing node is a fixture error,
/// not a skip condition.
#[test]
#[allow(clippy::too_many_lines)]
fn sankey_module_depth_highlights_mapped_node() {
    // -- Step 1: build a coupling-rich SPA from the dedicated fixture. --------
    let fixture = coupling_repo::build();
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    let repo = GixRepo::open(fixture.dir.path()).expect("open coupling fixture");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    db.ingest(&repo, &opts).expect("ingest coupling fixture");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");

    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-coupling.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Module-Depth Coupling Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-20 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    // -- Step 2: launch Chrome and let Alpine/widgets boot (skip if absent). --
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // -- Step 3: switch the sankey to module depth 2. -------------------------
    let _: bool = eval_json(
        &tab,
        "(function () { \
             var L = window.Alpine && window.Alpine.store && window.Alpine.store('layout'); \
             if (!L) return false; \
             L.sankeyDepth = 2; \
             return true; \
         })()",
    );

    // -- Step 4: poll until the sankey re-renders with module-prefix nodes. ---
    // We need a node whose name matches the 2-segment module prefix of some
    // hotspot-table row. modulePathSeg(p, 2): first 2 segments when path has
    // >2 segments, else dir-up-to-last-slash (or the full path if no slash).
    let mut prefix_node = String::new();
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        prefix_node = eval_json(
            &tab,
            "(function () { \
                 var el = document.getElementById('widget-coupling-sankey-body'); \
                 if (!el || !window.echarts) return ''; \
                 var chart = window.echarts.getInstanceByDom(el); \
                 if (!chart) return ''; \
                 var opt = chart.getOption(); \
                 var nodes = opt && opt.series && opt.series[0] && opt.series[0].data; \
                 if (!nodes || !nodes.length) return ''; \
                 var tbody = document.getElementById('hotspot-tbody'); \
                 if (!tbody) return ''; \
                 function modPrefix(p) { \
                     var parts = (p || '').split('/'); \
                     if (parts.length <= 2) { \
                         var ls = (p || '').lastIndexOf('/'); \
                         return ls < 0 ? (p || '') : p.slice(0, ls); \
                     } \
                     return parts.slice(0, 2).join('/'); \
                 } \
                 var rows = tbody.querySelectorAll('tr[data-path]'); \
                 for (var r = 0; r < rows.length; r++) { \
                     var p = rows[r].getAttribute('data-path'); \
                     var pref = modPrefix(p); \
                     for (var n = 0; n < nodes.length; n++) { \
                         if (nodes[n].name === pref) { \
                             window.__codeloreModPath2 = p; \
                             window.__codeloreModPrefix2 = pref; \
                             return pref; \
                         } \
                     } \
                 } \
                 return ''; \
             })()",
        );
        if !prefix_node.is_empty() {
            break;
        }
    }

    // A missing prefix node means the fixture is broken — fail, do not skip.
    assert!(
        !prefix_node.is_empty(),
        "depth-2 sankey has no node matching any hotspot-table row's module prefix; \
         the coupling_repo fixture must produce cross-module co-changes at depth 2"
    );

    // -- Step 5: spy dispatchAction, clear selection, publish the full path. --
    let _: bool = eval_json(
        &tab,
        "(function () { \
             var el = document.getElementById('widget-coupling-sankey-body'); \
             var chart = el && window.echarts && window.echarts.getInstanceByDom(el); \
             if (!chart) return false; \
             window.__codeloreModHi2 = null; \
             var orig = chart.dispatchAction.bind(chart); \
             chart.dispatchAction = function (pp) { \
                 if (pp && pp.type === 'highlight') window.__codeloreModHi2 = pp.name || ''; \
                 return orig(pp); \
             }; \
             window.Alpine.store('selection').clear(); \
             return true; \
         })()",
    );
    std::thread::sleep(Duration::from_millis(100));

    // -- Step 6: publish the full file path via the selection store. ----------
    let _: bool = eval_json(
        &tab,
        "(function () { \
             window.Alpine.store('selection').set(window.__codeloreModPath2); \
             return true; \
         })()",
    );
    std::thread::sleep(Duration::from_millis(100));

    // -- Step 7: assert the captured highlight name == the module prefix. -----
    let captured: String = eval_json(
        &tab,
        "(function () { return window.__codeloreModHi2 || ''; })()",
    );
    assert_eq!(
        captured, prefix_node,
        "in module-depth view the coupling subscriber highlighted '{captured}' but \
         expected module prefix '{prefix_node}' — the modulePathSeg mapping is not \
         applied to the incoming selection path",
    );
}

/// The file-detail drawer groups its sections into a 3-tab layout
/// (Overview / Coupling / People) with Overview shown by default, and
/// activating another tab hides Overview and shows that panel.
#[test]
#[allow(clippy::too_many_lines)]
fn detail_drawer_groups_sections_into_tabs() {
    let fixture = coupling_repo::build();
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    let repo = GixRepo::open(fixture.dir.path()).expect("open coupling fixture");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    db.ingest(&repo, &opts).expect("ingest coupling fixture");
    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");
    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-drawer.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Drawer Tabs Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-20 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    /* Open the drawer for the first hotspot-table row via the publish path. */
    let opened: bool = eval_json(
        &tab,
        "(function () { \
             var tbody = document.getElementById('hotspot-tbody'); \
             if (!tbody) return false; \
             var row = tbody.querySelector('tr[data-path]'); \
             if (!row) return false; \
             window._codeloreShowDetail(row.getAttribute('data-path')); \
             return true; \
         })()",
    );
    assert!(opened, "no hotspot-table row to open the drawer from");
    std::thread::sleep(Duration::from_millis(100));

    let tab_count: i64 = eval_json(
        &tab,
        "(function () { \
             var b = document.getElementById('drawer-body'); \
             return b ? b.querySelectorAll('[role=\"tab\"]').length : -1; \
         })()",
    );
    assert_eq!(tab_count, 3, "drawer should expose exactly 3 tabs");

    let overview_default: bool = eval_json(
        &tab,
        "(function () { \
             var ov = document.getElementById('drawer-panel-overview'); \
             var cp = document.getElementById('drawer-panel-coupling'); \
             return !!ov && !!cp && !ov.classList.contains('hidden') \
                 && cp.classList.contains('hidden'); \
         })()",
    );
    assert!(
        overview_default,
        "Overview panel must be visible and Coupling hidden by default"
    );

    let switched: bool = eval_json(
        &tab,
        "(function () { \
             var t = document.getElementById('drawer-tab-coupling'); \
             if (!t) return false; \
             t.click(); \
             var ov = document.getElementById('drawer-panel-overview'); \
             var cp = document.getElementById('drawer-panel-coupling'); \
             return ov.classList.contains('hidden') && !cp.classList.contains('hidden') \
                 && t.getAttribute('aria-selected') === 'true'; \
         })()",
    );
    assert!(
        switched,
        "activating the Coupling tab must show it and hide Overview"
    );
}

/// The coupling chord assigns each module an `ECharts` category so clusters
/// are colour-distinct (top-level module group, or one-per-module on a
/// single-root repo). Rendered from a fixture with real cross-module coupling.
#[test]
#[allow(clippy::too_many_lines)]
fn module_chord_colours_clusters() {
    let fixture = coupling_repo::build();
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    let repo = GixRepo::open(fixture.dir.path()).expect("open coupling fixture");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    db.ingest(&repo, &opts).expect("ingest coupling fixture");
    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");
    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-chord.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Chord Cluster Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-20 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    /* The chord may need its widget scrolled/rendered; poll the ECharts option. */
    let mut cats: i64 = -1;
    let mut first_has_cat = false;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        cats = eval_json(
            &tab,
            "(function () { \
                 var el = document.getElementById('widget-module-chord-body'); \
                 if (!el || !window.echarts) return -1; \
                 var chart = window.echarts.getInstanceByDom(el); \
                 if (!chart) return -1; \
                 var opt = chart.getOption(); \
                 var s = opt && opt.series && opt.series[0]; \
                 if (!s || !s.categories) return -1; \
                 return s.categories.length; \
             })()",
        );
        if cats >= 1 {
            first_has_cat = eval_json(
                &tab,
                "(function () { \
                     var el = document.getElementById('widget-module-chord-body'); \
                     var chart = window.echarts.getInstanceByDom(el); \
                     var d = chart.getOption().series[0].data; \
                     return !!d && d.length > 0 && typeof d[0].category === 'number'; \
                 })()",
            );
            break;
        }
    }
    assert!(
        cats >= 1,
        "module chord should expose at least one ECharts category"
    );
    assert!(
        first_has_cat,
        "each chord node should carry a numeric category index"
    );
}

/// Selecting a file on the hotspot map surfaces WHICH files it is coupled to:
/// each coupling arc now carries its partner path (`_arc.peer`) so the map can
/// outline + name the coupled circles. Uses a fixture with real file-level
/// coupling so at least one selection yields a partner.
#[test]
#[allow(clippy::too_many_lines)]
fn hotspot_map_coupling_arcs_name_their_partner() {
    let fixture = coupling_repo::build();
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    let repo = GixRepo::open(fixture.dir.path()).expect("open coupling fixture");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    db.ingest(&repo, &opts).expect("ingest coupling fixture");
    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let summary = run_summary(&db, &opts).expect("summary");
    let code_health = run_code_health(&db, &opts).expect("code-health");
    let coupling = run_coupling(&db, &opts).expect("coupling");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");
    let dash = SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-coupling-partners.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Coupling Partners Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-20 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    /* Try each hotspot-table path as the selection, one CDP round-trip apart so
    the async selection fan-out flushes between set and read. The first
    coupled file yields >=1 arc whose _arc.peer names the partner. */
    let mut peer = String::new();
    let candidates: i64 = eval_json(
        &tab,
        "(function () { \
             var tbody = document.getElementById('hotspot-tbody'); \
             if (!tbody) return 0; \
             var rows = tbody.querySelectorAll('tr[data-path]'); \
             window.__codeloreCandidatePaths = []; \
             for (var i = 0; i < rows.length; i++) { \
                 window.__codeloreCandidatePaths.push(rows[i].getAttribute('data-path')); \
             } \
             return window.__codeloreCandidatePaths.length; \
         })()",
    );
    assert!(
        candidates > 0,
        "coupling fixture should render hotspot-table rows"
    );

    for idx in 0..candidates {
        let _: bool = eval_json(
            &tab,
            &format!(
                "(function () {{ \
                     var p = window.__codeloreCandidatePaths[{idx}]; \
                     window.Alpine.store('selection').set(p); \
                     return true; \
                 }})()"
            ),
        );
        std::thread::sleep(Duration::from_millis(80));
        peer = eval_json(
            &tab,
            "(function () { \
                 var el = document.getElementById('widget-hotspot-circle-pack-body'); \
                 if (!el || !window.echarts) return ''; \
                 var chart = window.echarts.getInstanceByDom(el); \
                 if (!chart) return ''; \
                 var opt = chart.getOption(); \
                 var arcs = opt && opt.series && opt.series[1] && opt.series[1].data; \
                 if (arcs && arcs.length && arcs[0]._arc && arcs[0]._arc.peer) { \
                     return arcs[0]._arc.peer; \
                 } \
                 return ''; \
             })()",
        );
        if !peer.is_empty() {
            break;
        }
    }

    assert!(
        !peer.is_empty(),
        "selecting a coupled file should draw a coupling arc whose _arc.peer names \
         the partner file — the coupling_repo fixture has file-level coupling"
    );
}

/// The health-trend widget toggle must work without Bug 1 (`DaisyUI` `.toggle`
/// collision) and without Bug 2 (`#ht-charts` zero height). Sequence:
/// 1. Overlay view (default): `#ht-charts` canvas is visible and tall enough.
/// 2. `#ht-toggle` is present, wide enough to click, and says "Split view".
/// 3. After one click: three `.ht-sm` panels each with a rendered canvas.
/// 4. After a second click: back to a single overlay canvas.
#[test]
fn health_trend_toggle_renders_both_views() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-health-trend.html");
    write_smoke_spa(&html_path, "CodeLore Health-Trend Toggle Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // Step 1: overlay canvas exists and has non-zero height (Bug 2 guard).
    let overlay_height: i64 = eval_json(
        &tab,
        "(function () { \
             var host = document.getElementById('ht-charts'); \
             if (!host) return 0; \
             var canvas = host.querySelector('canvas'); \
             if (!canvas) return 0; \
             return canvas.clientHeight; \
         })()",
    );
    assert!(
        overlay_height > 250,
        "overlay #ht-charts canvas clientHeight was {overlay_height}px — \
         expected >250px (CSS sets 320px); the #ht-charts container may have \
         zero or collapsed height (Bug 2)"
    );

    // Step 2: toggle button is visible and carries the expected label (Bug 1 guard).
    // Two separate scalar evals because headless_chrome's RemoteObject.value is
    // only populated for primitive JS types — arrays come back as objectId refs.
    let toggle_width: i64 = eval_json(
        &tab,
        "(function () { \
             var btn = document.getElementById('ht-toggle'); \
             return btn ? btn.offsetWidth : 0; \
         })()",
    );
    assert!(
        toggle_width > 40,
        "ht-toggle offsetWidth was {toggle_width}px — expected >40px; \
         the button may have collapsed to a DaisyUI toggle knob (Bug 1)"
    );
    let toggle_label: String = eval_json(
        &tab,
        "(function () { \
             var btn = document.getElementById('ht-toggle'); \
             return btn ? btn.textContent.trim() : ''; \
         })()",
    );
    assert_eq!(
        toggle_label, "Split view",
        "ht-toggle label was '{toggle_label}'; expected 'Split view' (overlay is the default view)"
    );

    // Step 3: click the toggle → split view with three .ht-sm panels.
    tab.find_element("#ht-toggle")
        .expect("ht-toggle element")
        .click()
        .expect("click ht-toggle");
    std::thread::sleep(Duration::from_millis(600));

    // Return a JSON-encoded string from JS because headless_chrome.RemoteObject.value
    // is None for arrays — only scalar primitive returns go through .value.
    let split_json: String = eval_json(
        &tab,
        "(function () { \
             var panels = document.querySelectorAll('.ht-sm'); \
             var heights = Array.from(panels).map(function (p) { \
                 var c = p.querySelector('canvas'); \
                 return c ? c.clientHeight : 0; \
             }); \
             return JSON.stringify(heights); \
         })()",
    );
    let split_heights: Vec<i64> =
        serde_json::from_str(&split_json).expect("parse split heights JSON");
    assert_eq!(
        split_heights.len(),
        3,
        "expected 3 .ht-sm panels after toggle click, got {}",
        split_heights.len()
    );
    for (i, &h) in split_heights.iter().enumerate() {
        assert!(
            h > 150,
            "split panel {i} canvas clientHeight was {h}px — expected >150px \
             (CSS sets 180px; the pre-fix 130px must fail this); panel height \
             may have regressed to an unreadable size (Bug 3)"
        );
    }

    // Step 4: click again → back to overlay, single canvas in #ht-charts.
    tab.find_element("#ht-toggle")
        .expect("ht-toggle element (second click)")
        .click()
        .expect("click ht-toggle second time");
    std::thread::sleep(Duration::from_millis(600));

    let overlay_back: i64 = eval_json(
        &tab,
        "(function () { \
             var host = document.getElementById('ht-charts'); \
             if (!host) return 0; \
             var canvas = host.querySelector('canvas'); \
             if (!canvas) return 0; \
             return canvas.clientHeight; \
         })()",
    );
    assert!(
        overlay_back > 250,
        "overlay canvas after toggle-back had clientHeight {overlay_back}px — \
         expected >250px; the toggle re-render may not have restored the overlay"
    );
}

/// The DSM Fusion cell-mode toggle (`classifyCells` in
/// `40_architecture.js`) must classify above-diagonal cells by
/// structure×history agreement without throwing. `coupling_repo` (see
/// its doc comment) guarantees a Fisher-significant `src/alpha` ↔
/// `src/beta` co-change; the synthetic import edges below deliberately
/// route around that pair (`alpha→gamma`, `gamma→beta`), so Fusion mode
/// must draw a brand-new `temporal-only` cell for it that structure mode
/// never has — a robust, magnitude-independent proof that reclassification
/// actually ran (the cell COUNT must increase), not just a cosmetic
/// re-paint.
#[test]
fn dsm_fusion_mode_toggle_classifies_cells_without_errors() {
    // -- Step 1: coupling-rich fixture + synthetic cross-module imports
    // that avoid the alpha↔beta pair. ------------------------------------
    let fixture = coupling_repo::build();
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    let repo = GixRepo::open(fixture.dir.path()).expect("open coupling fixture");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    db.ingest(&repo, &opts).expect("ingest coupling fixture");

    let coupling = run_coupling(&db, &opts).expect("coupling");
    assert!(
        !coupling.is_empty(),
        "coupling_repo fixture must produce coupling rows under permissive opts \
         (the Fusion precondition)"
    );
    // Imports deliberately route around the guaranteed alpha↔beta co-change,
    // so Fusion mode must add a temporal-only cell structure mode never draws.
    let dash = SpaDashboard {
        hotspots: run_hotspots(&db, &opts).expect("hotspots"),
        summary: run_summary(&db, &opts).expect("summary"),
        code_health: run_code_health(&db, &opts).expect("code-health"),
        knowledge_islands: run_knowledge_islands(&db, &opts).expect("knowledge-islands"),
        coupling,
        imports: vec![
            ImportEdgeRow {
                src_path: "src/alpha/svc.rs".to_string(),
                target_path: "src/gamma/svc.rs".to_string(),
            },
            ImportEdgeRow {
                src_path: "src/gamma/util.rs".to_string(),
                target_path: "src/beta/util.rs".to_string(),
            },
        ],
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-dsm-fusion.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore DSM Fusion Test",
        &fixture.dir.path().display().to_string(),
        "2026-07-14 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    // -- Step 2: launch Chrome and watch for console errors from here on. -
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    // -- Step 3: default mode is Structure; the wt-btn toggle announces the
    // mode a click switches INTO ('Fusion'), mirroring `#ht-toggle`. -------
    let toggle_label = element_text(&tab, "wam-mode-toggle");
    assert_eq!(
        toggle_label, "Fusion",
        "wam-mode-toggle label was '{toggle_label}'; expected 'Fusion' \
         (structure is the default mode)"
    );
    let structure_cells = echarts_series_len(&tab, "wam-chart-host");
    assert!(
        structure_cells > 0,
        "structure-mode matrix has no rendered cells"
    );

    // -- Step 4: click the toggle → Fusion mode. --------------------------
    tab.find_element("#wam-mode-toggle")
        .expect("wam-mode-toggle element")
        .click()
        .expect("click wam-mode-toggle");
    std::thread::sleep(Duration::from_millis(500));

    let fusion_label = element_text(&tab, "wam-mode-toggle");
    assert_eq!(
        fusion_label, "Structure",
        "toggle label did not flip to 'Structure' after entering Fusion mode"
    );
    let fusion_cells = echarts_series_len(&tab, "wam-chart-host");
    assert!(
        fusion_cells > structure_cells,
        "Fusion-mode cell count ({fusion_cells}) was not greater than structure-mode's \
         ({structure_cells}); the guaranteed src/alpha\u{2194}src/beta coupling-only pair \
         should add a new temporal-only cell that structure mode never draws"
    );

    // -- Step 5: the legend row names all four cell classes as TEXT
    // (never color-only). ------------------------------------------------
    let legend_text = element_text(&tab, "wam-legend");
    for phrase in [
        "agree",
        "structural only",
        "modularity violation",
        "back-edge",
    ] {
        assert!(
            legend_text.contains(phrase),
            "Fusion legend missing '{phrase}'; legend text was: {legend_text}"
        );
    }

    // -- Step 6: the toggle interaction produced zero console errors. -----
    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "DSM Fusion toggle produced {} browser-console error(s):\n{}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// A repo with many modules must not overflow the DSM card. The matrix
/// mounts on a nested `#wam-chart-host`, but the widget BODY
/// (`#widget-arch-matrix-body`, pinned to a 460px fallback in the template)
/// must GROW to contain it — exactly as the widget behaved before the Fusion
/// toolbar/legend restructuring introduced the nested host. A 3-module
/// fixture is too short to trip this (host < 460px), which is why the shipped
/// Fusion test never caught it; the 30-module chain below drives the host past
/// 768px, so a body still pinned at 460 fails the `body >= host` contract.
/// The same test also proves the fullscreen resize sweep now reaches the chart
/// on the nested host (the pre-fix `-body` selector misses it).
#[test]
fn arch_matrix_body_grows_to_contain_tall_matrix() {
    // Coupling-rich base so every widget rendered before the matrix boots
    // cleanly; imports overridden to a 30-module chain that drives the matrix
    // well past the 460px fallback.
    let fixture = coupling_repo::build();
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    let repo = GixRepo::open(fixture.dir.path()).expect("open coupling fixture");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    db.ingest(&repo, &opts).expect("ingest coupling fixture");

    // 30 distinct depth-2 modules (`src/mod00`..`src/mod29`) chained so every
    // edge crosses a module boundary: auto-depth settles at depth 2 with 30
    // nodes, well past the ~24 that overflow the 460px fallback.
    let imports: Vec<ImportEdgeRow> = (0..29)
        .map(|i| ImportEdgeRow {
            src_path: format!("src/mod{i:02}/f.rs"),
            target_path: format!("src/mod{:02}/f.rs", i + 1),
        })
        .collect();
    let dash = SpaDashboard {
        hotspots: run_hotspots(&db, &opts).expect("hotspots"),
        summary: run_summary(&db, &opts).expect("summary"),
        code_health: run_code_health(&db, &opts).expect("code-health"),
        knowledge_islands: run_knowledge_islands(&db, &opts).expect("knowledge-islands"),
        coupling: run_coupling(&db, &opts).expect("coupling"),
        imports,
        ..SpaDashboard::default()
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-dsm-tall.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore DSM Tall Matrix Test",
        &fixture.dir.path().display().to_string(),
        "2026-07-14 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    // #1 — the body grew past the 460px fallback AND contains the chart host.
    let host_h = client_height(&tab, "wam-chart-host");
    let body_h = client_height(&tab, "widget-arch-matrix-body");
    assert!(
        host_h > 460,
        "precondition weak: 30-module host was only {host_h}px; expected >460 \
         so a pinned body would visibly overflow"
    );
    assert!(
        body_h >= host_h && body_h > 460,
        "widget-arch-matrix-body clientHeight was {body_h}px but the chart host \
         was {host_h}px — the body must grow to contain the matrix (>=host, >460) \
         instead of staying pinned at the template's 460px fallback"
    );

    // #2 — the fullscreen resize sweep now reaches the chart on the nested
    // host. The pre-fix selector misses it (the chart is not on a `-body`);
    // the fixed selector descends into `-chart-host`.
    assert!(
        sweep_reaches_chart(
            &tab,
            ".widget-body, [id$=\"-body\"], [id$=\"-chart-host\"]",
            "wam-chart-host",
        ),
        "fixed resize sweep did not reach the DSM chart on #wam-chart-host"
    );
    assert!(
        !sweep_reaches_chart(&tab, ".widget-body, [id$=\"-body\"]", "wam-chart-host"),
        "pre-fix selector unexpectedly matched #wam-chart-host — the regression \
         proof would be vacuous"
    );

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "tall-matrix render produced {} browser-console error(s):\n{}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// Collapsing a `.dash-group` hides its grid entirely (`display: none` via
/// `.dash-collapsed`), so any `ECharts` instance inside resizes to 0 — a
/// canvas that never repaints on its own. Expanding the section must run
/// the resize-on-expand path (`resizeAllEchartsIn`, `00_setup_boot.js`) so
/// the chart recovers. Drives the Architecture section, whose
/// `#wam-chart-host` (arch-matrix) mounts a real chart from the smoke
/// fixture's `imports` data.
#[test]
fn section_collapse_and_expand_keeps_charts_sized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-section-collapse.html");
    write_smoke_spa(&html_path, "CodeLore Section Collapse Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    let console_errors = attach_exception_sink(&tab);

    let hash_before: String = eval_json(&tab, "location.hash");
    let series_before = echarts_series_len(&tab, "wam-chart-host");
    assert!(
        series_before > 0,
        "arch-matrix must have rendered cells before collapsing (got {series_before})"
    );

    let chevron = "document.querySelector('#group-architecture .dash-collapse')";
    tab.evaluate(&format!("{chevron}.click()"), false)
        .expect("click architecture chevron");
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(
        client_height(&tab, "group-architecture-grid"),
        0,
        "group-architecture-grid must be hidden (0 clientHeight) once collapsed"
    );
    let expanded_attr: String =
        eval_json(&tab, &format!("{chevron}.getAttribute('aria-expanded')"));
    assert_eq!(
        expanded_attr, "false",
        "chevron aria-expanded did not flip to false"
    );

    tab.evaluate(&format!("{chevron}.click()"), false)
        .expect("click architecture chevron again");
    std::thread::sleep(Duration::from_millis(200));

    assert!(
        client_height(&tab, "group-architecture-grid") > 0,
        "group-architecture-grid must be visible again after re-expanding"
    );
    let expanded_attr_after: String =
        eval_json(&tab, &format!("{chevron}.getAttribute('aria-expanded')"));
    assert_eq!(
        expanded_attr_after, "true",
        "chevron aria-expanded did not flip back to true"
    );

    // The resize-on-expand path must have kept the chart sized: both the
    // ECharts series data and the canvas's rendered pixel width.
    let series_after = echarts_series_len(&tab, "wam-chart-host");
    assert!(
        series_after > 0,
        "arch-matrix lost its rendered cells after expand (got {series_after})"
    );
    let canvas_width: f64 = eval_json(
        &tab,
        "(function () { \
             var host = document.getElementById('wam-chart-host'); \
             var canvas = host && host.querySelector('canvas'); \
             return canvas ? canvas.width : 0; \
         })()",
    );
    assert!(
        canvas_width > 0.0,
        "arch-matrix canvas has zero rendered width after expand ({canvas_width})"
    );

    let hash_after: String = eval_json(&tab, "location.hash");
    assert_eq!(
        hash_before, hash_after,
        "collapsing/expanding a section must never mutate location.hash"
    );

    let errors = console_errors.lock().expect("console mutex").clone();
    assert!(
        errors.is_empty(),
        "section collapse/expand produced {} browser-console error(s):\n{}",
        errors.len(),
        errors.join("\n  "),
    );
}

/// Below the 1280px breakpoint `.dash-group-grid` collapses to a single
/// column, so every widget — including ones carrying `xl:col-span-2`,
/// which only takes effect at >=1280px — must render at essentially the
/// full content width. Proves the laptop-width single-column contract from
/// measured geometry rather than from asserting a class name is present.
#[test]
fn laptop_width_renders_single_column() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-laptop-width.html");
    write_smoke_spa(&html_path, "CodeLore Laptop Width Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    set_viewport(&tab, 1100, 900);
    std::thread::sleep(Duration::from_millis(300));

    let (_, _, main_w) = bounding_rect(&tab, "main");
    assert!(main_w > 0.0, "main content width was {main_w} at 1100px");

    for id in ["#widget-arch-matrix", "#widget-hotspot-table"] {
        let (_, _, w) = bounding_rect(&tab, id);
        assert!(
            w >= 0.9 * main_w,
            "{id} width ({w}) was not >= 0.9x the main content width \
             ({main_w}) at a 1100px viewport — the single-column laptop \
             layout regressed"
        );
    }
}

/// At >=1280px each section's grid becomes two columns and its designated
/// half-width pair shares a row. Drives the Knowledge section (surfaces +
/// islands) — the same pairing the responsive-rules spec calls out — and
/// proves the desktop layout from measured geometry: equal row tops and
/// each card under 60% of the content width.
#[test]
fn desktop_width_pairs_half_widgets() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-desktop-width.html");
    write_smoke_spa(&html_path, "CodeLore Desktop Width Test");

    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };
    set_viewport(&tab, 1500, 900);
    std::thread::sleep(Duration::from_millis(300));

    let (_, _, main_w) = bounding_rect(&tab, "main");
    let (surfaces_top, _, surfaces_w) = bounding_rect(&tab, "#widget-knowledge-surfaces");
    let (islands_top, _, islands_w) = bounding_rect(&tab, "#widget-knowledge-islands");

    assert!(
        (surfaces_top - islands_top).abs() < 1.0,
        "knowledge-surfaces top ({surfaces_top}) and knowledge-islands top \
         ({islands_top}) are not in the same row at a 1500px viewport"
    );
    for (label, w) in [
        ("knowledge-surfaces", surfaces_w),
        ("knowledge-islands", islands_w),
    ] {
        assert!(
            w < 0.6 * main_w,
            "{label} width ({w}) was not < 0.6x the main content width \
             ({main_w}) at a 1500px viewport — the half-width pairing \
             regressed"
        );
    }
}

/// Every colour lens's field must survive `buildFsHierarchy`'s metrics copy —
/// that literal is the SOLE producer of the circle-pack's `metrics`, and the
/// AI lens rendered the whole map "no data" grey for as long as `ai_pct` was
/// missing from it while the table two panels down showed real percentages.
/// Pinned at the d3 data binding: every leaf's metrics object must carry the
/// lens keys, so a field dropped from the copy fails here by name.
#[test]
fn circle_pack_metrics_carry_every_colour_lens_field() {
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");
    let dash = SpaDashboard {
        hotspots: run_hotspots(&db, &opts).expect("hotspots"),
        summary: run_summary(&db, &opts).expect("summary"),
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore AI Lens Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    // The circle-pack draws to an ECharts CANVAS — there are no DOM leaves
    // to query — so the pin drives the exposed builder hook directly with a
    // synthetic row and asserts every colour-lens field survives the
    // metrics copy (the sole producer the AI lens reads).
    // Primitives only across the evaluate boundary: the probe returns a
    // joined string ('' = every field present).
    let missing: String = eval_json(
        &tab,
        "(() => { \
            const root = window._codeloreBuildFsHierarchy([{ \
              path: 'src/probe.rs', revisions: 3, cognitive: 7.5, \
              cognitive_health: 88.0, hotspot_score: 0.4, \
              ai_pct: 42.0, mi: 71.0, mi_rank: 0.5 }]); \
            let leaf = root; \
            while (leaf && leaf.children && leaf.children.length) { \
              leaf = leaf.children[0]; \
            } \
            if (!leaf || !leaf.metrics) return '<no leaf metrics built>'; \
            return ['revisions','cognitive','cognitive_health','hotspot_score',\
                    'ai_pct','mi','mi_rank'] \
              .filter(k => !(k in leaf.metrics)).join(','); \
         })()",
    );
    assert!(
        missing.is_empty(),
        "buildFsHierarchy dropped colour-lens fields from metrics: {missing}"
    );
}

/// Escape must close the detail drawer: it opens non-modally (`.show()`),
/// so the platform installs no close watcher and only the
/// `@keydown.escape.window` directive provides the universal convention.
#[test]
fn escape_closes_the_detail_drawer() {
    let fixture = differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open fixture repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        age_time_now: Some(time::macros::date!(2099 - 01 - 01)),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest fixture");
    let knowledge_islands = run_knowledge_islands(&db, &opts).expect("knowledge-islands");
    assert!(
        !knowledge_islands.is_empty(),
        "fixture produced no knowledge-island rows; the drawer cannot open"
    );
    let dash = SpaDashboard {
        hotspots: run_hotspots(&db, &opts).expect("hotspots"),
        summary: run_summary(&db, &opts).expect("summary"),
        knowledge_islands,
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(
        &dash,
        "CodeLore Escape Test",
        &fixture.dir.path().display().to_string(),
        "2026-06-16 00:00:00 UTC",
        &mut f,
    )
    .expect("write_spa");
    drop(f);
    let Some((_browser, tab)) = boot_spa_tab(&html_path) else {
        return;
    };

    tab.wait_for_element("tr.ki-row")
        .expect("at least one knowledge-islands row should render");
    tab.evaluate(
        "(() => { const r = document.querySelector('tr.ki-row'); \
         r.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); })()",
        false,
    )
    .expect("open drawer via Enter");
    std::thread::sleep(Duration::from_millis(300));
    let open: bool = eval_json(
        &tab,
        "document.getElementById('file-detail-drawer').open === true",
    );
    assert!(
        open,
        "drawer did not open; Escape assertion would be vacuous"
    );

    tab.evaluate(
        "window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))",
        false,
    )
    .expect("dispatch Escape");
    std::thread::sleep(Duration::from_millis(300));
    let closed: bool = eval_json(
        &tab,
        "document.getElementById('file-detail-drawer').open === false",
    );
    assert!(
        closed,
        "Escape must close the non-modal drawer via the @keydown.escape.window directive"
    );
}
