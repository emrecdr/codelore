# SPA Linked-Brushing Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete spec §5's "select a file once, highlight it everywhere" by registering selection-highlight listeners on the widgets that don't yet subscribe — the hotspot table, the coupling sankey, the architecture DSM, and the hotspot circle-pack map itself — so a click in any view lights up the same file across all of them.

**Architecture:** The SPA already has a working single-focus linked-brushing bus: an `Alpine.store('selection')` with a `.path`, a reactive `Alpine.effect` that fans `selection.path` out to every function in `window._codeloreSelectionListeners`, a publish side (widgets call `selection.set(path)`), and a `window._codeloreRegisterSelectionListener(source, fn)` registry. Today only `trends` and `parallel-coords` subscribe. This plan adds four more subscribers, each mirroring the proven `trends` listener (ECharts `dispatchAction` highlight/downplay) or, for the table, a DOM class toggle; the map reuses its existing `selectedCouplingFile` + `updateCouplingArcs()` overlay. Pure JS in `widgets.js`; no store change, no new library, no Rust change.

**Tech Stack:** Vanilla JS inside the vendored single-file SPA (`widgets.js`), Alpine.js 3 store + effect, ECharts 6 (`dispatchAction`). Gated behind the `spa` Cargo feature. Tested via `spa_integration_test.rs` (HTML-string assertions) + `spa_browser_test.rs` (headless Chrome — the real gate for runtime highlight behavior).

## Global Constraints

- **Offline single-file SPA is sacrosanct.** No new runtime CDN, npm, build step, or vendored lib. Pure JS added to `widgets.js`.
- **No store change, no Rust change.** The `selection` store (`template.html`) and the fan-out effect already exist and work — do NOT modify them. Consume the existing `window._codeloreRegisterSelectionListener(source, fn)` registry. No `SpaDashboard` change.
- **Use existing conventions (HARD RULE).** Every new subscriber mirrors the canonical `trends` subscriber (`widgets.js:2603-2614`): `window._codeloreRegisterSelectionListener('<name>', function (selectedPath) { if (!selectedPath) { <downplay/clear>; return; } <highlight the path>; });`. Registration goes at the END of that widget's render function (after its `chart` is built), exactly like trends/parallel-coords. Highlight (don't hide) non-matches — a null path clears/downplays.
- **Idempotent registration.** The registry dedupes by `source` string (`widgets.js:120-126`), so each widget must use a UNIQUE, STABLE source name (`'hotspot-table'`, `'coupling'`, `'dsm'`, `'hotspot-map'`). A re-render re-registers under the same name → replaces, never duplicates.
- **British "colour" in prose comments** matches the codebase convention; identifiers stay American (color). No task/version/PR markers in comments (present-state only) — HARD RULE.
- **Build/test (macOS dev box):** prefix cargo with `MACOSX_DEPLOYMENT_TARGET=15.0`. Integration: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`. Browser (the real behavior gate): `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test`. Build: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa`. Do NOT run `just ci` (spa link fails locally on this box).
- Conventional Commits; **never** add `Co-Authored-By: Claude`.

## Scope notes (this plan is ONE slice of the §5 overhaul)

- **THIS plan (3b):** broaden single-`.path` selection SUBSCRIBER coverage to table + coupling + DSM + map. Completes "highlight everywhere" using the existing store.
- **Deferred (stated, not dropped):** the **bivariate legend click-to-filter** (clicking a 3×3 cell brushes ALL files in that health×activity quadrant) is a SET brush — a different interaction from the single-`.path` store — and gets its own slice (**3c**) with its own design decision (how to model a multi-file brush without conflating it with single-focus selection). Tabbed drawer + edge-bundled coupling → **3d**; Observable Plot + IA restructure → **3e**.

## File Structure

- **Modify** `crates/codelore-lib/src/output/spa/widgets.js` only:
  - Hotspot table render fn (`renderHotspotTable`, ~`:1979+`): add a `'hotspot-table'` subscriber that toggles a highlight class on the row whose path matches.
  - Coupling sankey render fn (`renderCouplingSankey`): add a `'coupling'` subscriber that `dispatchAction` highlights the file's sankey node.
  - DSM render fn (`renderArchMatrix` / the heatmap): add a `'dsm'` subscriber that highlights the selected row/col.
  - Circle-pack render fn (`renderHotspotCirclePack`, ~`:1494+`): add a `'hotspot-map'` subscriber that reuses `selectedCouplingFile = selectedPath; updateCouplingArcs();` so cross-widget selection lights the file's coupling arcs on the map.
- **Modify** `crates/codelore-lib/tests/spa_browser_test.rs`: assert cross-widget highlight fires (select a path → a subscriber widget reflects it).

## Reference: verified current code (mirror this exactly)

- Canonical subscriber — `widgets.js:2603-2614` (trends):
  ```javascript
  window._codeloreRegisterSelectionListener('trends', function (selectedPath) {
    if (!selectedPath) { chart.dispatchAction({ type: 'downplay' }); return; }
    const idx = paths.indexOf(selectedPath);
    if (idx >= 0) { chart.dispatchAction({ type: 'highlight', seriesIndex: idx }); }
    else { chart.dispatchAction({ type: 'downplay' }); }
  });
  ```
- Registry (do not modify) — `widgets.js:111` `window._codeloreSelectionListeners = []`; `:120` `window._codeloreRegisterSelectionListener = function (source, fn) {…dedupe by source…}`.
- Fan-out effect (do not modify) — `template.html`: an `Alpine.effect` reads `Alpine.store('selection').path` and calls every listener with it.
- Publish side (already works) — circle-pack leaf click → `Alpine.store('selection').set(path)` (`widgets.js:660`); drawer-close clears.
- Map overlay machinery to reuse — `selectedCouplingFile` (module-scope), `updateCouplingArcs()`, `buildCouplingArcs(selectedCouplingFile, lastHotspotNodePositions, data.coupling)`, the second custom series at `widgets.js:1903-1949`. Leaf click already sets `selectedCouplingFile` + calls `updateCouplingArcs()` (`:1961-1962`).

---

### Task 1: Hotspot-table selection subscriber (DOM row highlight)

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (end of `renderHotspotTable`)

**Interfaces:**
- Consumes: `window._codeloreRegisterSelectionListener` (existing); the table's rendered `<tr>` rows carrying a per-row path.
- Produces: a `'hotspot-table'` subscriber that adds/removes a `is-selected` highlight class on the matching row.

- [ ] **Step 1: Read the table render to find the row → path binding**

Read `renderHotspotTable` (`widgets.js:~1979+`). Confirm how each `<tr>` is associated with its path (a `data-path` attribute, a closure var, or the row's cell text). The rows are re-created on filter/sort/paginate, so the subscriber must query the CURRENT rows each time it fires (query the DOM inside the listener, do not cache row nodes).

- [ ] **Step 2: Add the subscriber at the end of `renderHotspotTable`**

After the table is built and its click handlers wired, add:

```javascript
    // Cross-widget selection: highlight the row for the selected path (if
    // it's on the current page); a null selection clears all row highlights.
    // Rows are rebuilt on sort/filter/paginate, so query the live DOM each
    // time rather than caching nodes.
    window._codeloreRegisterSelectionListener('hotspot-table', function (selectedPath) {
      const tbody = document.getElementById('hotspot-table-body');
      if (!tbody) return;
      const rows = tbody.querySelectorAll('tr');
      for (var i = 0; i < rows.length; i++) {
        const rowPath = rows[i].getAttribute('data-path');
        rows[i].classList.toggle('is-selected', !!selectedPath && rowPath === selectedPath);
      }
    });
```

(Confirm the tbody id + that rows carry `data-path`; if rows don't yet carry the path as an attribute, add `data-path` when the row is created — one attribute on the existing `<tr>` build. If the tbody has a different id, use the real one.)

- [ ] **Step 3: Ensure the `is-selected` style exists**

The `is-selected` class must be a visible highlight. If a highlight style isn't already defined for table rows, add a minimal rule to the SPA CSS source is out of scope (CSS is precompiled); instead style inline-safe via a DaisyUI utility class already in the bundle — use `'!bg-base-300'` (a complete literal so Tailwind's scanner keeps it) instead of a custom class:

```javascript
        rows[i].classList.toggle('!bg-base-300', !!selectedPath && rowPath === selectedPath);
```

Use this DaisyUI-utility form (not a custom `is-selected` class) so no CSS rebuild is needed. Update Step 2's toggle accordingly.

- [ ] **Step 4: Build + integration test (no regression)**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa` (Expected: Finished) and `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test` (Expected: all pass — this task adds no new integration assertion; the highlight behavior is browser-tested in Task 5).

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js
git commit -m "feat(spa): highlight the selected file's row in the hotspot table"
```

---

### Task 2: Coupling-sankey selection subscriber

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (end of `renderCouplingSankey`)

**Interfaces:**
- Consumes: `window._codeloreRegisterSelectionListener`; the sankey `chart` handle and its node names (file paths).
- Produces: a `'coupling'` subscriber that highlights the selected file's sankey node.

- [ ] **Step 1: Read the sankey render to confirm the chart var + node naming**

Read `renderCouplingSankey`. Confirm the ECharts `chart` variable name and that sankey nodes are named by file path (ECharts sankey highlight targets a node by `name`). Note whether nodes use full paths or basenames — the selection path is a full repo-relative path, so the highlight `name` must match the node name space.

- [ ] **Step 2: Add the subscriber at the end of `renderCouplingSankey`**

```javascript
    // Cross-widget selection: emphasise the selected file's node (and its
    // links) in the coupling sankey; a null selection downplays everything.
    window._codeloreRegisterSelectionListener('coupling', function (selectedPath) {
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      chart.dispatchAction({ type: 'highlight', seriesIndex: 0, name: selectedPath });
    });
```

(If sankey nodes are named by basename rather than full path, map `selectedPath` to the node-name space before dispatching — read the node build to confirm. If the file isn't a sankey node, the `highlight` is a harmless no-op after the `downplay`.)

- [ ] **Step 3: Build + integration test**

Run the build + `spa_integration_test` as in Task 1 Step 4. Expected: Finished + all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js
git commit -m "feat(spa): highlight the selected file's node in the coupling sankey"
```

---

### Task 3: Architecture-DSM selection subscriber

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (end of the DSM/`renderArchMatrix` render fn)

**Interfaces:**
- Consumes: `window._codeloreRegisterSelectionListener`; the DSM heatmap `chart` handle + its axis label → path mapping.
- Produces: a `'dsm'` subscriber that emphasises the selected file's row/column in the dependency-structure matrix.

- [ ] **Step 1: Read the DSM render to find the chart var + axis path mapping**

Read the DSM render fn (the ECharts heatmap; grep `renderArchMatrix` or the heatmap series). Confirm the `chart` var and how the matrix axes are labelled (by path or by a shortened module name) and how a cell maps to (rowPath, colPath). Heatmap emphasis is per-dataItem; to emphasise a whole row/col you dispatch highlight for the matching data indices OR mark the axis label.

- [ ] **Step 2: Add the subscriber at the end of the DSM render fn**

Mirror the trends structure; the exact highlight call depends on the DSM's data model (read in Step 1). A robust form that works for an axis-indexed heatmap:

```javascript
    // Cross-widget selection: emphasise the selected file's row + column in
    // the DSM. Null selection downplays. Path→axis index via the axis label
    // list captured when the matrix was built (`dsmPaths` below).
    window._codeloreRegisterSelectionListener('dsm', function (selectedPath) {
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      const idx = dsmPaths.indexOf(selectedPath);
      if (idx < 0) return;
      // Highlight every cell in row `idx` and column `idx`.
      const n = dsmPaths.length;
      const indices = [];
      for (var k = 0; k < n; k++) { indices.push(idx * n + k); indices.push(k * n + idx); }
      chart.dispatchAction({ type: 'highlight', seriesIndex: 0, dataIndex: indices });
    });
```

(`dsmPaths` is the ordered axis-label array the matrix already builds — use its real name from Step 1. If the heatmap `dataIndex` layout differs from `row*n+col`, adjust the index math to match how the matrix pushes its data. If the DSM widget is absent for small repos, the render fn returns early and never registers — that's fine.)

- [ ] **Step 3: Build + integration test**

Build + `spa_integration_test`. Expected: Finished + all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js
git commit -m "feat(spa): emphasise the selected file's row and column in the DSM"
```

---

### Task 4: Circle-pack map selection subscriber (reuse coupling-arc overlay)

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (end of `renderHotspotCirclePack`, near the existing `renderBivariateLegend()` call at `:1977`)

**Interfaces:**
- Consumes: `window._codeloreRegisterSelectionListener`; the existing module-scope `selectedCouplingFile` + `updateCouplingArcs()`.
- Produces: a `'hotspot-map'` subscriber so a selection originating in ANOTHER widget lights up the file's coupling arcs on the map (the same overlay a direct leaf-click already shows).

- [ ] **Step 1: Confirm the overlay machinery is in scope**

Read `renderHotspotCirclePack`. Confirm `selectedCouplingFile` and `updateCouplingArcs()` are reachable at the point after the chart is built (they are used by the leaf-click handler at `widgets.js:1961-1962` and the zr background-click at `:1970-1975`). The subscriber sets the same variable + calls the same updater, so cross-widget selection reuses the proven overlay path.

- [ ] **Step 2: Add the subscriber next to the existing `renderBivariateLegend()` call**

Just before `renderBivariateLegend();` at the end of `renderHotspotCirclePack`, add:

```javascript
    // Cross-widget selection: when a file is selected in ANY widget, light up
    // its coupling arcs on the map — the same overlay a direct leaf-click
    // shows. Reuses the existing selectedCouplingFile + updateCouplingArcs
    // machinery, so the map participates in the shared focus without a
    // second highlight mechanism. A null selection clears the arcs.
    window._codeloreRegisterSelectionListener('hotspot-map', function (selectedPath) {
      selectedCouplingFile = selectedPath || null;
      updateCouplingArcs();
    });
```

- [ ] **Step 3: Guard against a publish→subscribe echo loop**

The map is BOTH a publisher (leaf click → `selection.set(path)`) and now a subscriber. Confirm this can't loop: the subscriber sets `selectedCouplingFile` + redraws arcs but does NOT call `selection.set(...)`, so it does not re-publish. The publish side (`onLeafClick`) sets `selectedCouplingFile` directly too, then publishes — the subscriber firing back with the same path is idempotent (same file → same arcs). Document this in the comment (done in Step 2) and verify no `selection.set` appears inside the new listener.

- [ ] **Step 4: Build + integration test**

Build + `spa_integration_test`. Expected: Finished + all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js
git commit -m "feat(spa): light the selected file's coupling arcs on the map from any widget"
```

---

### Task 5: Browser-test the cross-widget highlight + CHANGELOG

**Files:**
- Modify: `crates/codelore-lib/tests/spa_browser_test.rs`
- Modify: `CHANGELOG.md`

**Interfaces:** none new — verifies the slice end-to-end and documents it.

- [ ] **Step 1: Read the browser test harness**

Read `spa_browser_test.rs` (`#![cfg(all(feature = "browser-tests", feature = "spa", feature = "test-support"))]`). Find how it boots the SPA in headless Chrome, evaluates JS (`eval_json` / `find_element`), and where the existing assertions live (Plan 3a added `#bivariate-legend` + default-tab assertions). Reuse that harness.

- [ ] **Step 2: Add a cross-widget highlight assertion**

After boot, drive a selection from JS and assert a subscriber widget reflects it. A robust, widget-agnostic check uses the store + a subscriber's observable effect. Add a step that sets the selection and asserts the table row highlight (the most directly-observable subscriber):

```rust
    // -- Cross-widget linked brushing: selecting a path highlights its row. --
    // Pick the first hotspot path, publish it via the selection store, and
    // assert the matching table row gains the highlight class.
    let selected_ok: bool = eval_json(
        &tab,
        r#"(function () {
            var body = document.getElementById('hotspot-table-body');
            if (!body) return false;
            var first = body.querySelector('tr[data-path]');
            if (!first) return false;
            var p = first.getAttribute('data-path');
            window.Alpine.store('selection').set(p);
            // Alpine effect is microtask-async; flush synchronously for the assert.
            return document.querySelector('tr[data-path="' + p + '"]').classList.contains('!bg-base-300');
        })()"#,
    );
    assert!(selected_ok, "selecting a path should highlight its hotspot-table row");
```

(If the Alpine effect that fans selection out is async and the assertion races, wrap the read in a short poll — mirror any existing `wait_for`/poll helper in the harness. If `eval_json`'s signature differs, match the real one from Step 1. If `data-path` was not added in Task 1, this assertion drove that requirement — reconcile with Task 1.)

- [ ] **Step 3: Run the browser test (the real behavior gate)**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test`
Expected: PASS including the new assertion. If headless Chrome can't run locally, note it — CI runs it; do NOT block the slice on a local browser run, but DO make the assertion correct by construction.

- [ ] **Step 4: CHANGELOG**

Add under `[Unreleased] > ### Added`:

```markdown
- **SPA linked brushing across all widgets.** Selecting a file in any dashboard view now highlights the same file everywhere at once — the hotspot table row, the coupling sankey node, the architecture DSM row/column, the trends and parallel-coordinates series, and the file's coupling arcs on the hotspot map. One shared focus, highlight (not hide) — clearing the selection downplays everything back to neutral.
```

- [ ] **Step 5: Full local gate + commit**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa` and `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`.
Expected: build Finished; integration tests pass.

```bash
git add crates/codelore-lib/tests/spa_browser_test.rs CHANGELOG.md
git commit -m "test(spa): assert cross-widget linked-brushing highlight; document it"
```

---

## Self-Review

**Spec coverage** (design spec §5, linked-brushing bullet):
- "selecting a file/dir highlights it across map/DSM/coupling/ownership/trend simultaneously" → trends + parallel-coords already subscribe; this plan adds table (Task 1), coupling (Task 2), DSM (Task 3), map (Task 4). ⚠️ **ownership**: the design names "ownership" as a highlight target; there is no standalone ownership widget in the current SPA (ownership is folded into the circle-pack's author color mode + the drawer). Covered indirectly via the map + drawer; a dedicated ownership-widget subscriber is N/A. Noted, not silently dropped. ✓
- "One 'focus entity' model; highlight (don't hide) non-matches" → all subscribers highlight/emphasise and downplay on null; none hide. ✓
- "bivariate legend is the primary filter (click a cell to brush)" → **explicitly deferred to Plan 3c** (set-brush, different interaction). Noted in Scope. ✓
- "deterministic node positions / cartography / tabbed drawer / Observable Plot / IA restructure" → later plans (Scope). ✓

**Placeholder scan:** no TBD/"handle edge cases"/"similar to Task N". Each subscriber shows the real listener code mirroring the verified trends pattern. The per-widget "confirm the chart var / node-name space / axis array name" notes are verification instructions (with the exact fallback), not placeholders — the pattern and the highlight call are concrete; the implementer binds the one real variable name from the widget it's editing. ✓

**Type/name consistency:** the four source names (`'hotspot-table'`, `'coupling'`, `'dsm'`, `'hotspot-map'`) are unique and stable; `window._codeloreRegisterSelectionListener` is used identically across tasks; `selectedCouplingFile`/`updateCouplingArcs` (Task 4) are the existing module-scope names; the `!bg-base-300` highlight literal is consistent between Task 1 and the Task-5 browser assertion; `data-path` on table rows is introduced in Task 1 and consumed by the Task-5 assertion (Task 1 Step 2 adds it if absent). ✓

**Open risk for the executor:** the ECharts emphasis calls (sankey `name`, DSM `dataIndex`) depend on each widget's exact series data shape — the plan gives the correct API form and directs the implementer to confirm the node/axis naming from the widget it edits; the browser test (Task 5) is the real gate that the highlight visibly fires. The map subscriber must NOT call `selection.set` (echo-loop guard, Task 4 Step 3).
