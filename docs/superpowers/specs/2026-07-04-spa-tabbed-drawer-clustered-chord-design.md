# SPA Plan 3d Design — Tabbed Drawer + Clustered Coupling Chord

**Status:** Approved design (2026-07-04). Next: implementation plan via writing-plans.

**Scope:** Two independent SPA refinements under the "3d" label of the linked-brushing overhaul: (A) restructure the long file-detail drawer into tabs; (B) make top-level module clusters visible in the existing coupling chord. Both are pure JS inside the vendored single-file SPA — no Rust change, no new vendored library, no CSS rebuild.

**Deferred / out of scope (documented, not dropped):**
- **True hierarchical edge bundling** (a d3-hierarchy radial cluster + hand-rolled bundled B-splines) — rejected: it would re-implement one widget in d3 while every other chart is ECharts, a rendering-tech split not justified by the clutter win over clustered coloring.
- **3e** (Observable Plot migration + IA restructure) — a separate future idea. Observable Plot would duplicate ECharts (already vendored) and grow the bundle/dependency surface; revisit only if charting needs ever outgrow ECharts.

## Context (verified current state)

- The SPA is a single self-contained `codelore.html`: `crates/codelore-lib/src/output/spa.rs` `include_str!`s `template.html` + `widgets.js` and substitutes `{{ECHARTS_JS}}`/`{{D3_HIERARCHY_JS}}`/`{{ALPINE_JS}}`/`{{ALPINE_PERSIST_JS}}` (build-time SHA-pinned fetch in `build.rs`). Adding a lib is possible but is a deliberate surface-area decision; this design adds none.
- **Drawer:** `showFileDetailDrawer(path, d)` (`widgets.js:~1061-1283`, ~222 lines) builds ONE scrolling panel via imperative `innerHTML +=`, with sections: Hotspot, Knowledge island (ownership %), Coupling partners, Top contributors, Functions, Clones, Code health, plus a radar (`renderDrawerRadar`, `:~1284`). It is NOT Alpine-templated — it is imperative DOM building. The drawer `<dialog>` is non-modal (`.show()`).
- **Chord:** `renderModuleChord` (`widgets.js:~3303`) is NOT a true chord — it is an ECharts `type:'graph'`, `layout:'circular'`, `curveness:0.3`, `lineStyle.color:'source'`, `emphasis.focus:'adjacency'`. Nodes are module paths (`modulePath(p, depth)`, first-N-segments) ordered by `Object.keys(nodes).sort()` (lexical — already groups same-prefix siblings adjacently). It has **no ECharts `categories`**, so all nodes are one colour and top-level clusters are not visually distinguished. Infrastructure files (lock/env/docs/manifests) are filtered; depth is `Alpine.store('layout').chordDepth` (`'auto'` adaptive or fixed 2-6), re-rendered by the layout Alpine effect.
- **A11y precedent to reuse:** the SPA already has an accessible tablist pattern (the colour-mode toggle / sankey-depth toggle) with a passing `tablist_arrow_keys_move_focus_and_selection` browser test, and a `wireRowKbActivation(el)` helper (role=button + tabindex + Enter/Space→click). DaisyUI `tabs`/`tab`/`tab-active` classes are in the pre-built bundle.

## Component A — Tabbed file-detail drawer

**Problem:** one ~222-line scroll; reaching "who owns this" means scrolling past coupling + clones. Sections have no top-level grouping.

**Design:** group the existing sections into **3 tabs**, built inside `showFileDetailDrawer`'s produced markup:
- **Overview** (default on every open): Hotspot + Code health (band) + Clones + Functions + the radar. The "how risky is this file" summary.
- **Coupling:** the coupling-partners list + strengths.
- **People:** Knowledge island / ownership % + Top contributors.

Behaviour + implementation:
- **Tab bar** uses DaisyUI `tabs`/`tab`/`tab-active` (in-bundle → no CSS rebuild). Three `tabpanel` containers; a small plain-JS click/keydown handler toggles `tab-active` on the tab and a `hidden` class on the panels — matching the drawer's existing imperative-innerHTML style (the drawer is not Alpine-reactive, so no `x-show`).
- **Default tab resets to Overview on every file open** (you always want the summary first; no cross-file tab persistence — simplest, and avoids a stale tab pointing at an empty section for a different file).
- **A11y:** `role="tablist"` on the bar, `role="tab"` + `aria-selected` + `aria-controls` per tab, `role="tabpanel"` + `aria-labelledby` per panel, roving `tabindex`, Left/Right arrow-key navigation — mirror the existing tablist pattern and reuse `wireRowKbActivation` where it fits. Keep the existing drawer title + non-modal close wiring unchanged.
- **Empty sections:** a tab whose sections are all empty for this file shows a muted "No <group> data for this file." line instead of a blank panel (the drawer already wraps each section lookup in try/catch and has an empty-body fallback; extend that per-tab).
- **Failure isolation preserved:** keep the per-section try/catch so one malformed row can't blank the whole drawer.

**Interfaces / boundaries:** `showFileDetailDrawer(path, d)` keeps its signature and remains the single entry point. Internally it now emits `buildTabBar()` + three `buildOverviewPanel/​buildCouplingPanel/​buildPeoplePanel(path, d)` string builders + one `wireDrawerTabs(root)` activator. Each panel builder owns exactly its sections; the activator owns only tab switching. This keeps each unit small and independently testable (a panel builder is a pure `(path,d)->html`; the activator is pure DOM wiring).

## Component B — Clustered coupling chord

**Problem:** the circular graph colours edges by source but has no node grouping, so top-level module clusters are invisible and unrelated edges cross the circle.

**Design (stay in ECharts, no tech swap, no new lib):**
- Compute each node's **top-level group** = its first path segment (e.g. `src/alpha` → `src`; a single-segment module is its own group). Build an ECharts `categories: [{name: group}, …]` list and set each node's `category` index. ECharts then colours nodes by cluster automatically.
- **Order nodes by (group, path)** so each cluster occupies a contiguous arc of the circle (makes `.sort()`'s grouping explicit and correct for 3+ segment depths).
- Keep `layout:'circular'`, `emphasis.focus:'adjacency'`. Tune `curveness` (slightly higher so intra-cluster edges bow inward) and edge `opacity` (fade non-emphasised edges) so clusters read at a glance. Edge colour stays `'source'` (now meaningfully cluster-tinted since sources are category-coloured), or switch to category colour if it reads better at implementation time.
- Preserve the existing infrastructure-file filter, depth adaptation, empty-state messages, and the layout Alpine-effect re-render contract unchanged.

**Boundary:** all changes are inside `renderModuleChord`; the node/edge aggregation (`aggregateAt`, `modulePath`, `isInfrastructureFile`) is untouched — only the node-array build (add category + ordering) and the `setOption` (add `categories`, tune line style) change.

## Testing

**Browser test (`spa_browser_test.rs`) — the real behaviour gate:**
- **Drawer:** open the detail drawer for a file (drive `_codeloreShowDetail(path)` or a row click), assert a `role="tablist"` with 3 `role="tab"` tabs exists, the Overview panel is visible by default, and activating the Coupling/People tab hides Overview and shows that panel (assert `hidden`/visibility + `aria-selected`). Reuse the `coupling_repo` fixture (has coupling partners so the Coupling tab is populated).
- **Chord:** render from `coupling_repo` (real cross-module clusters), assert the module-chord ECharts option has `categories.length >= 1` and nodes carry a `category`.
- Follow the existing `boot_spa_tab` + split-eval + poll idioms. No `//` comments inside `\`-continued Rust eval strings (they collapse the line). No finding-IDs in comments (describe the invariant). `Step N:`-style labels are the test's own convention.

**Integration test (`spa_integration_test.rs`):** stays green — HTML-string assertions unaffected; add a light assertion only if a new stable string anchor is worth pinning (optional).

## Constraints (carried)

- Offline single-file SPA sacrosanct — no new CDN/npm/build step/vendored lib; pure JS in `widgets.js` (+ minimal `template.html` if a tab-panel container or a hand-rolled style is needed, mirroring the `.hotspot-row-brushed`/`.sr-only` precedent — but prefer in-bundle DaisyUI classes).
- No Rust change, no store change (the drawer/chord read the embedded `data` they already receive).
- British "colour" in prose comments; American identifiers. No task/version/PR/finding-ID markers in `widgets.js`/`template.html` comments (present-state only).
- Build/test on the macOS dev box: prefix cargo with `MACOSX_DEPLOYMENT_TARGET=15.0`; browser gate `spa_browser_test`; do NOT run `just ci` (spa link fails locally; GitHub Actions macOS-15 is the gate).
- Conventional Commits; never `Co-Authored-By: Claude`.

## Self-review checklist (fill at execution)

- Drawer: exactly 3 tabs; Overview default resets per open; every section lands in exactly one tab; empty-tab muted message; a11y tablist mirrors the existing pattern; per-section try/catch retained; no store/Rust change.
- Chord: categories = distinct first-segment groups; nodes ordered (group, path) → contiguous arcs; infra filter + depth adaptation + empty states unchanged; only node-build + setOption touched.
- Tests: drawer tab-switch + chord categories asserted on `coupling_repo`; no `//` in continued eval strings; no finding-IDs in comments; integration green.
