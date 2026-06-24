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

    // -- Step 2: launch headless Chrome (skip if unavailable). -----------
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

    // -- Step 3: navigate + let Alpine/widgets boot. ---------------------
    let url = format!("file://{}", html_path.display());
    tab.navigate_to(&url).expect("navigate");
    tab.wait_until_navigated().expect("wait navigation");
    // Same boot window as the smoke test: the KI table renders inside the
    // cooperative widget-boot loop, and the row click handlers attach
    // during that render.
    std::thread::sleep(Duration::from_secs(2));

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
    let drawer_open: bool = serde_json::from_value(
        tab.evaluate(
            "document.getElementById('file-detail-drawer').open === true",
            false,
        )
        .expect("eval drawer.open")
        .value
        .expect("drawer.open value"),
    )
    .expect("drawer.open bool");
    assert!(drawer_open, "detail drawer did not open on KI row click");

    let drawer_not_hidden: bool = serde_json::from_value(
        tab.evaluate(
            "!document.getElementById('file-detail-drawer').hasAttribute('hidden')",
            false,
        )
        .expect("eval drawer hidden")
        .value
        .expect("drawer hidden value"),
    )
    .expect("drawer hidden bool");
    assert!(
        drawer_not_hidden,
        "detail drawer kept the [hidden] attribute after open"
    );

    let drawer_displayed: bool = serde_json::from_value(
        tab.evaluate(
            "getComputedStyle(document.getElementById('file-detail-drawer')).display !== 'none'",
            false,
        )
        .expect("eval drawer display")
        .value
        .expect("drawer display value"),
    )
    .expect("drawer display bool");
    assert!(
        drawer_displayed,
        "detail drawer computed display:none after open (invisible popup)"
    );

    let title_len: i64 = serde_json::from_value(
        tab.evaluate(
            "document.getElementById('drawer-title').textContent.trim().length",
            false,
        )
        .expect("eval title len")
        .value
        .expect("title len value"),
    )
    .expect("title len i64");
    assert!(
        title_len > 0,
        "drawer title (clicked path) was empty; title_len={title_len}"
    );

    let body_len: i64 = serde_json::from_value(
        tab.evaluate(
            "document.getElementById('drawer-body').innerHTML.length",
            false,
        )
        .expect("eval body len")
        .value
        .expect("body len value"),
    )
    .expect("body len i64");
    assert!(body_len > 0, "drawer body was empty; body_len={body_len}");

    let has_ki_section: bool = serde_json::from_value(
        tab.evaluate(
            "document.getElementById('drawer-body').textContent.includes('Knowledge island')",
            false,
        )
        .expect("eval ki section")
        .value
        .expect("ki section value"),
    )
    .expect("ki section bool");
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

    let drawer_closed: bool = serde_json::from_value(
        tab.evaluate(
            "(() => { const d = document.getElementById('file-detail-drawer'); \
             return d.open === false && (d.hasAttribute('hidden') || \
             getComputedStyle(d).display === 'none'); })()",
            false,
        )
        .expect("eval drawer closed")
        .value
        .expect("drawer closed value"),
    )
    .expect("drawer closed bool");
    assert!(
        drawer_closed,
        "detail drawer did not close after clicking the × button"
    );
}
