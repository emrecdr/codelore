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
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::analyses::knowledge_islands::run_knowledge_islands;
use codelore_lib::analyses::summary::run_summary;
use codelore_lib::facts::FactsDb;
use codelore_lib::output::spa::{SpaDashboard, write_spa};
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::differential_repo;
use headless_chrome::Browser;
use headless_chrome::protocol::cdp::types::Event;

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
        let captured: String =
            eval_json(&tab, "(function(){return window.__codeloreSankeyHi || '';})()");
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
                  tr[data-path=\"{}\"]'); return !!r && r.classList.contains('!bg-base-300'); }})()",
                bridged_path
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

    // -- Step 13: coupling subscriber highlights the mapped node in module depth. -
    // Switch the sankey to module depth 2; nodes are then modulePathSeg(path,2)
    // prefixes. Set the selection to a full file path and assert the subscriber
    // highlights its 2-segment module prefix, not the raw path — guarding the
    // module-name-space mapping. Poll for the re-render (cooperatively
    // scheduled). This is a correct-by-construction guard that only FIRES on a
    // repo with cross-module change-coupling at depth 2; the differential
    // fixture has near-zero co-changes, so the sankey is empty at depth 2 and
    // this step skips (no qualifying node). A coupling-rich fixture would make
    // it live — see the deep-analysis report follow-up.
    let module_target: String = eval_json(
        &tab,
        "(function () { \
             var L = window.Alpine && window.Alpine.store && window.Alpine.store('layout'); \
             if (!L) return ''; \
             L.sankeyDepth = 2; \
             return 'set'; \
         })()",
    );
    if module_target == "set" {
        // Poll up to ~3s for the sankey to re-render with prefix node names.
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
                                 window.__codeloreModPath = p; \
                                 window.__codeloreModPrefix = pref; \
                                 return pref; \
                             } \
                         } \
                     } \
                     return ''; \
                 })()",
            );
            if !prefix_node.is_empty() { break; }
        }
        if !prefix_node.is_empty() {
            // Spy dispatchAction, clear then publish the full path, read the
            // captured highlight name — must be the module PREFIX, not the path.
            let _: bool = eval_json(
                &tab,
                "(function () { \
                     var el = document.getElementById('widget-coupling-sankey-body'); \
                     var chart = el && window.echarts && window.echarts.getInstanceByDom(el); \
                     if (!chart) return false; \
                     window.__codeloreModHi = null; \
                     var orig = chart.dispatchAction.bind(chart); \
                     chart.dispatchAction = function (pp) { \
                         if (pp && pp.type === 'highlight') window.__codeloreModHi = pp.name || ''; \
                         return orig(pp); \
                     }; \
                     window.Alpine.store('selection').clear(); \
                     return true; \
                 })()",
            );
            std::thread::sleep(Duration::from_millis(100));
            let _: bool = eval_json(
                &tab,
                "(function () { \
                     window.Alpine.store('selection').set(window.__codeloreModPath); return true; \
                 })()",
            );
            std::thread::sleep(Duration::from_millis(100));
            let captured: String =
                eval_json(&tab, "(function(){return window.__codeloreModHi || '';})()");
            assert_eq!(
                captured, prefix_node,
                "in module-depth view the coupling subscriber did not highlight the \
                 selected file's module prefix — the modulePathSeg mapping is broken"
            );
        } else {
            println!(
                "spa_browser_test: module-depth sankey step skipped — no fixture file whose \
                 2-segment module prefix is a visible sankey node"
            );
        }
    }
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
fn write_smoke_spa(html_path: &std::path::Path, title: &str) {
    use codelore_lib::analyses::dashboard::{
        DailyCommit, ImportEdgeRow, KameiRiskRow, TrendPoint, XRayEntry,
    };

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
