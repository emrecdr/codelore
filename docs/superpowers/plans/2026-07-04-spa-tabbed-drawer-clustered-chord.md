# SPA Tabbed Drawer + Clustered Coupling Chord Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the SPA's long file-detail drawer into three tabs (Overview / Coupling / People) and colour the coupling chord's modules by top-level group, both as pure JS in the vendored single-file SPA.

**Architecture:** Two independent changes in `crates/codelore-lib/src/output/spa/widgets.js`. (A) `showFileDetailDrawer` routes its existing sections into three accumulator strings and wraps them in a DaisyUI tablist with a small keyboard-accessible activator. (B) `renderModuleChord` assigns each node an ECharts `category` (top-level module group, with a per-node fallback for single-root repos) so clusters are colour-distinct. No Rust, store, template, or CSS-rebuild change; no new vendored library.

**Tech Stack:** Vanilla JS, ECharts 6 (`type:'graph'` circular + `categories`), DaisyUI `tabs` classes (in-bundle), Alpine 3 (unchanged). Tested via `spa_browser_test.rs` (headless Chrome — the behaviour gate) reusing the `coupling_repo` fixture.

## Global Constraints

- **Offline single-file SPA sacrosanct** — no new CDN/npm/build step/vendored lib; pure JS in `widgets.js`. No `template.html` change (the drawer body + chord are built by `widgets.js`). No CSS rebuild — use in-bundle DaisyUI utility classes (`tabs`, `tab`, `tab-active`, `hidden`).
- **No Rust change, no store change.** The drawer and chord read the embedded `d`/`rows` data they already receive.
- **Mirror existing conventions.** Tablist a11y mirrors the colour-mode toggle (`role=tablist/tab/tabpanel`, roving `tabindex`, arrow-key nav) already covered by `tablist_arrow_keys_move_focus_and_selection`.
- **British "colour"/"emphasise" in prose comments; American identifiers.** NO task/version/PR/finding-ID markers in `widgets.js` comments (present-state only). The browser test's `Step N:`-style labels are its own convention; inside `\`-continued Rust eval strings use NO `//` JS comments (they collapse the line — use `/* */` or none).
- **Build/test (macOS dev box):** prefix cargo with `MACOSX_DEPLOYMENT_TARGET=15.0`. Browser gate: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test`. Build: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa`. Do NOT run `just ci` (spa link fails locally; GitHub Actions macOS-15 is the gate).
- Conventional Commits; **never** `Co-Authored-By: Claude`.

## File Structure

- **Modify** `crates/codelore-lib/src/output/spa/widgets.js`:
  - `showFileDetailDrawer(path, d)` (`~:1061-1278`): add three top-level helpers just before it (`drawerTabBar()`, `drawerPanel(id, labelledby, inner, emptyLabel)`, `wireDrawerTabs(root)`); change the body-assembly tail so sections accumulate into `overviewHtml`/`couplingHtml`/`peopleHtml` and the body is a tablist + 3 panels.
  - `renderModuleChord(rows)` (`~:3303`): replace the `nodeArr` build with a category-assigning build; add `categories` + tuned `lineStyle` to `setOption`.
- **Modify** `crates/codelore-lib/tests/spa_browser_test.rs`: two new `#[test]` fns reusing `coupling_repo` + `boot_spa_tab`.

## Verified current state (read before editing)

- Drawer body-assembly tail today (`~:1235-1256`):
  ```javascript
      } catch (e) {
        console.error('codelore: drawer section render failed for', path, e);
      }
      if (!html) { html = hasPath ? '<div class="empty">…</div>' : '<div class="empty">…</div>'; }
      html = '<div id="drawer-radar" style="height: 220px; margin-bottom: 14px;"></div>' + html;
      body.innerHTML = html;
      try { renderDrawerRadar(path, d); } catch (e) { … hide #drawer-radar … }
  ```
  Sections built with `html += …` inside one `try`: Hotspot, Knowledge island, Coupling partners, Top contributors, Functions, Clones, Code health. The radar (`renderDrawerRadar`) mounts into `#drawer-radar` AFTER `body.innerHTML` is set and must remain in a VISIBLE container (ECharts needs layout height) — so it belongs in the default-visible Overview panel.
- Chord `nodeArr` today (`~:3402`): `const nodeArr = Object.keys(nodes).sort().map(function (n) { return { name: n }; });` — lexical sort already makes same-prefix modules contiguous; series has no `categories` (all nodes one colour). `setOption` series has `lineStyle: { color: 'source', opacity: 0.55, curveness: 0.3 }` and `emphasis: { focus: 'adjacency', lineStyle: { width: 3 } }`.

---

### Task 1: Tabbed file-detail drawer

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (add 3 helpers before `showFileDetailDrawer`; change its section-routing + body-assembly tail)
- Test: `crates/codelore-lib/tests/spa_browser_test.rs` (new `detail_drawer_groups_sections_into_tabs`)

**Interfaces:**
- Consumes: existing `showFileDetailDrawer(path, d)` entry point, `window._codeloreShowDetail(path)`, the `#drawer-body` container, `renderDrawerRadar(path, d)`, DaisyUI `tabs`/`tab`/`tab-active` + `hidden` classes.
- Produces: a `#drawer-body` layout containing `[role="tablist"]` with exactly 3 `[role="tab"]` (`#drawer-tab-overview|coupling|people`) controlling 3 `[role="tabpanel"]` (`#drawer-panel-overview|coupling|people`); Overview active by default; `wireDrawerTabs(root)` activator.

- [ ] **Step 1: Write the failing browser test**

Add to `crates/codelore-lib/tests/spa_browser_test.rs` (after the existing tests; it uses the shared `boot_spa_tab` + `eval_json` helpers and the `coupling_repo` fixture):

```rust
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
        hotspots, summary, code_health, coupling, knowledge_islands,
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-drawer.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(&dash, "CodeLore Drawer Tabs Test",
        &fixture.dir.path().display().to_string(), "2026-06-20 00:00:00 UTC", &mut f)
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
    assert!(overview_default, "Overview panel must be visible and Coupling hidden by default");

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
    assert!(switched, "activating the Coupling tab must show it and hide Overview");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test detail_drawer_groups_sections_into_tabs`
Expected: FAIL — `tab_count` is `-1`/`0` (no `[role="tab"]` in the drawer yet).

- [ ] **Step 3: Add the three drawer helpers**

Insert immediately BEFORE `function showFileDetailDrawer(path, d) {` (`~:1061`):

```javascript
  // Drawer tab bar: three DaisyUI tabs (in-bundle classes, no CSS rebuild)
  // wired as an ARIA tablist. Overview is the default selection on every
  // open. Panels are toggled by wireDrawerTabs below.
  function drawerTabBar() {
    return '<div class="tabs tabs-bordered" role="tablist" aria-label="File detail sections">' +
      '<button type="button" class="tab tab-active" role="tab" id="drawer-tab-overview" aria-controls="drawer-panel-overview" aria-selected="true" tabindex="0">Overview</button>' +
      '<button type="button" class="tab" role="tab" id="drawer-tab-coupling" aria-controls="drawer-panel-coupling" aria-selected="false" tabindex="-1">Coupling</button>' +
      '<button type="button" class="tab" role="tab" id="drawer-tab-people" aria-controls="drawer-panel-people" aria-selected="false" tabindex="-1">People</button>' +
      '</div>';
  }

  // One tabpanel. Non-overview panels start hidden. An empty section group
  // shows a muted message rather than a blank panel.
  function drawerPanel(id, labelledby, inner, emptyLabel) {
    const content = (inner && inner.length) ? inner : ('<div class="empty">' + emptyLabel + '</div>');
    const hidden = (id === 'drawer-panel-overview') ? '' : ' hidden';
    return '<div id="' + id + '" role="tabpanel" aria-labelledby="' + labelledby + '" class="drawer-tabpanel' + hidden + '">' + content + '</div>';
  }

  // Wire the drawer tablist: click / ArrowLeft-Right / Home-End move the
  // selection; only the active panel is shown. Roving tabindex + aria-selected
  // mirror the colour-mode toggle's accessible-tablist convention.
  function wireDrawerTabs(root) {
    const tabs = Array.prototype.slice.call(root.querySelectorAll('[role="tab"]'));
    const panels = tabs.map(function (t) { return document.getElementById(t.getAttribute('aria-controls')); });
    function select(idx) {
      for (var i = 0; i < tabs.length; i++) {
        const on = i === idx;
        tabs[i].setAttribute('aria-selected', on ? 'true' : 'false');
        tabs[i].setAttribute('tabindex', on ? '0' : '-1');
        tabs[i].classList.toggle('tab-active', on);
        if (panels[i]) panels[i].classList.toggle('hidden', !on);
      }
    }
    tabs.forEach(function (tab, i) {
      tab.addEventListener('click', function () { select(i); });
      tab.addEventListener('keydown', function (e) {
        var next = -1;
        if (e.key === 'ArrowRight') next = (i + 1) % tabs.length;
        else if (e.key === 'ArrowLeft') next = (i - 1 + tabs.length) % tabs.length;
        else if (e.key === 'Home') next = 0;
        else if (e.key === 'End') next = tabs.length - 1;
        if (next >= 0) { e.preventDefault(); tabs[next].focus(); select(next); }
      });
    });
    select(0);
  }
```

- [ ] **Step 4: Route sections into three accumulators**

In `showFileDetailDrawer`, replace the single `var html = '';` (`~:1073`) with three accumulators:

```javascript
    var overviewHtml = '';
    var couplingHtml = '';
    var peopleHtml = '';
```

Then, inside the existing `try { … } catch`, change the target of each section's `+=` (leave the section-building code itself unchanged):
- Hotspot section → `overviewHtml +=`
- Knowledge island section → `peopleHtml +=`
- Coupling partners section → `couplingHtml +=`
- Top contributors section → `peopleHtml +=`
- Functions section → `overviewHtml +=`
- Clones section → `overviewHtml +=`
- Code health section → `overviewHtml +=`

(Every `html +=` inside the try becomes the correct accumulator per the mapping above. Do not otherwise alter the section code.)

- [ ] **Step 5: Rebuild the body-assembly tail**

Replace the tail (`~:1235-1256`, from the `} catch (e) {` closing the section try through `body.innerHTML = html;`) with:

```javascript
    } catch (e) {
      console.error('codelore: drawer section render failed for', path, e);
    }

    // Radar lives at the top of the Overview panel (the default-visible tab —
    // ECharts needs a laid-out container with height). Its mount id is
    // unchanged so renderDrawerRadar still finds it after body.innerHTML.
    const radarDiv = '<div id="drawer-radar" style="height: 220px; margin-bottom: 14px;"></div>';

    body.innerHTML =
      drawerTabBar() +
      drawerPanel('drawer-panel-overview', 'drawer-tab-overview', radarDiv + overviewHtml,
        hasPath ? 'No overview metrics for this file — it may be below the minimum-revision threshold.'
                : 'This row had no resolvable file path, so no metrics could be looked up.') +
      drawerPanel('drawer-panel-coupling', 'drawer-tab-coupling', couplingHtml,
        'No change-coupling partners recorded for this file.') +
      drawerPanel('drawer-panel-people', 'drawer-tab-people', peopleHtml,
        'No ownership or contributor data for this file.');
    wireDrawerTabs(body);
```

(The old `if (!html) { … }` whole-drawer empty fallback is removed — per-panel empty messages replace it. The radar `renderDrawerRadar` call + its try/catch and the dialog-show block below remain unchanged.)

- [ ] **Step 6: Build + run the test to verify it passes**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa` (Expected: Finished), then
`MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test`
Expected: ALL pass including `detail_drawer_groups_sections_into_tabs` (and the existing drawer tests `detail_drawer_content_is_opaque_when_open` / `detail_drawer_has_accessible_name_and_manages_focus` / `detail_drawer_never_renders_empty_for_a_pathless_row` still pass — they assert the drawer opens populated + manages focus, which the tabbed body preserves).

- [ ] **Step 7: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js crates/codelore-lib/tests/spa_browser_test.rs
git commit -m "feat(spa): group the file-detail drawer into Overview/Coupling/People tabs"
```

---

### Task 2: Clustered coupling chord

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (`renderModuleChord` node-build + `setOption`)
- Test: `crates/codelore-lib/tests/spa_browser_test.rs` (new `module_chord_colours_clusters`)

**Interfaces:**
- Consumes: the existing `renderModuleChord` locals `nodes` (map of module-name → true), `linkRows`, and the ECharts circular-graph `setOption`.
- Produces: the module-chord series carrying `categories` (≥1) with every `data` node holding a numeric `category` index.

- [ ] **Step 1: Write the failing browser test**

Add to `crates/codelore-lib/tests/spa_browser_test.rs`:

```rust
/// The coupling chord assigns each module an ECharts category so clusters
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
        hotspots, summary, code_health, coupling, knowledge_islands,
        ..SpaDashboard::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let html_path = tmp.path().join("codelore-chord.html");
    let mut f = std::fs::File::create(&html_path).expect("create html");
    write_spa(&dash, "CodeLore Chord Cluster Test",
        &fixture.dir.path().display().to_string(), "2026-06-20 00:00:00 UTC", &mut f)
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
    assert!(cats >= 1, "module chord should expose at least one ECharts category");
    assert!(first_has_cat, "each chord node should carry a numeric category index");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test module_chord_colours_clusters`
Expected: FAIL — `cats` stays `-1` (series has no `categories`).

- [ ] **Step 3: Replace the `nodeArr` build with a category-assigning build**

In `renderModuleChord`, replace the single line (`~:3402`):
```javascript
    const nodeArr = Object.keys(nodes).sort().map(function (n) { return { name: n }; });
```
with:
```javascript
    // Colour each module by its top-level (first path segment) group so
    // clusters are visually distinct; the lexical sort already places
    // same-group modules on a contiguous arc. When every module shares one
    // top-level root (single-root repo → one group), fall back to one
    // category per module so modules stay individually distinguishable
    // rather than collapsing to a single colour.
    function chordTopGroup(name) {
      const slash = name.indexOf('/');
      return slash < 0 ? name : name.slice(0, slash);
    }
    const sortedNames = Object.keys(nodes).sort();
    const groupNames = [];
    const groupIndex = {};
    for (var gi = 0; gi < sortedNames.length; gi++) {
      const g = chordTopGroup(sortedNames[gi]);
      if (!(g in groupIndex)) { groupIndex[g] = groupNames.length; groupNames.push(g); }
    }
    const chordPerNode = groupNames.length < 2;
    const categories = chordPerNode
      ? sortedNames.map(function (n) { return { name: n }; })
      : groupNames.map(function (g) { return { name: g }; });
    const nodeArr = sortedNames.map(function (n, idx) {
      return { name: n, category: chordPerNode ? idx : groupIndex[chordTopGroup(n)] };
    });
```

- [ ] **Step 4: Add `categories` + tune the line style in `setOption`**

In the same `setOption` series object, add `categories: categories,` (e.g. immediately after `data: nodeArr,`) and update the `lineStyle` so intra-cluster edges hug the perimeter and non-focused edges fade:
```javascript
        lineStyle: {
          color: 'source',
          opacity: 0.45,
          curveness: 0.45,
        },
```
(Leave `emphasis: { focus: 'adjacency', lineStyle: { width: 3 } }` unchanged — it already fades non-adjacent edges on hover.)

- [ ] **Step 5: Build + run the test to verify it passes**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa` (Expected: Finished), then
`MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test`
Expected: ALL pass including `module_chord_colours_clusters`.

- [ ] **Step 6: Integration test (no regression)**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`
Expected: 4/4 pass (HTML-string assertions unaffected).

- [ ] **Step 7: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js crates/codelore-lib/tests/spa_browser_test.rs
git commit -m "feat(spa): colour coupling-chord modules by top-level cluster"
```

---

### Task 3: CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add entries under `[Unreleased] > ### Added`**

```markdown
- **Tabbed file-detail drawer.** Clicking a file now opens a drawer split into Overview / Coupling / People tabs instead of one long scroll — the risk summary (hotspot, health, clones, functions, radar) is on the first tab, with change-coupling partners and ownership/contributors one click away. Keyboard-navigable (arrow keys) and screen-reader-labelled.
- **Clustered coupling chord.** The module change-coupling chord now colours each module by its top-level group (falling back to a distinct colour per module in single-root repos), so related modules read as a cluster instead of a uniform ring.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(spa): changelog for tabbed drawer + clustered coupling chord"
```

---

## Self-Review

**Spec coverage** (design doc §Component A/B/Testing):
- Tabbed drawer, 3 tabs Overview/Coupling/People, Overview default per open → Task 1 (Steps 3-5). ✓
- DaisyUI in-bundle tabs, no CSS rebuild → Task 1 uses `tabs`/`tab`/`tab-active`/`hidden`. ✓
- A11y tablist (roles, roving tabindex, arrow keys) → `wireDrawerTabs` + `drawerTabBar` (Task 1 Step 3). ✓
- Per-section failure isolation preserved → the single section `try/catch` is kept; only `+=` targets change (Task 1 Step 4). ✓
- Empty-tab muted message → `drawerPanel(…, emptyLabel)` (Task 1 Step 3/5). ✓
- Radar stays in the visible Overview panel → Task 1 Step 5 (radar div prepended to overview). ✓
- Chord categories by top-level group + single-root per-node fallback + ordering → Task 2 Step 3. ✓
- Chord tuned curveness/opacity, only node-build + setOption touched → Task 2 Steps 3-4. ✓
- Browser tests reusing `coupling_repo` for both; integration stays green → Task 1 Step 1, Task 2 Step 1/6. ✓
- CHANGELOG → Task 3. ✓
- Non-goals (true HEB / d3-swap, 3e, new libs) → none introduced. ✓

**Placeholder scan:** every code step shows real code; no TBD/"similar to"/"handle edge cases". The Task 1 Step 4 section-routing is a precise per-section mapping (not "route appropriately") — the section names match the verified current-state list. ✓

**Type/name consistency:** `drawerTabBar()`/`drawerPanel(id,labelledby,inner,emptyLabel)`/`wireDrawerTabs(root)` used consistently; panel ids `drawer-panel-overview|coupling|people` and tab ids `drawer-tab-overview|coupling|people` match between the helpers, the body assembly, and the browser test's `getElementById`/`querySelector` calls; `overviewHtml`/`couplingHtml`/`peopleHtml` declared (Step 4) before the `try` and consumed (Step 5). Chord: `chordTopGroup`/`sortedNames`/`groupNames`/`groupIndex`/`chordPerNode`/`categories`/`nodeArr` all defined and used within Task 2 Step 3; `categories` consumed in Step 4. ✓

**Open risk for the executor:** the drawer's existing tests assert focus management + opaque content on open — the tabbed body must keep the drawer opening populated with Overview visible (Task 1 Step 6 re-runs them). If the chord widget isn't rendered until scrolled into view, the Task 2 test polls up to 3s for the ECharts instance (mirrors the module-depth test's poll). Single-root fixture (`coupling_repo`, all under `src/`) exercises the per-node fallback path — a multi-root repo would exercise the group path; both are covered by construction.
