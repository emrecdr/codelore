# SPA Layout Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Regroup the dashboard's 23 flat widgets into six titled sections with a sticky scrollspy nav, and make every chart full-width below 1280px (single-column laptops).

**Architecture:** Pure SPA-layer change: `template.html` (structure + hand-written CSS), `js/` modules (nav/scrollspy/collapse behaviors + registry order), tests. No Rust analysis changes; `output/spa.rs` untouched except if an integration assertion helper needs it (it should not).

**Tech Stack:** Vendored ECharts SPA — vanilla JS, Alpine store, DaisyUI/Tailwind pre-built CSS bundle (FROZEN — see constraints), headless-Chrome browser tests via the existing CDP harness.

**Spec:** `docs/superpowers/specs/2026-07-14-spa-layout-overhaul-design.md` — binding for all semantics.

## Global Constraints

1. Gates before EVERY commit: `cargo fmt --all --check` + CI-exact `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Browser + SPA integration suites before each commit that touches the SPA: `cargo test -p codelore-lib --features "browser-tests spa test-support" --test spa_browser_test` and `cargo test -p codelore-lib --features "test-support spa" --test spa_integration_test`. Full workspace suite once on the final tree. No `#[allow]` — extract helpers for line-count lints (established precedent in spa_browser_test.rs: `element_text`, `echarts_series_len`, `attach_exception_sink`).
2. **The Tailwind bundle is frozen.** New utility classes will silently not render. Allowed: classes already in the bundle (`grid-cols-1`, `md:grid-cols-2`, `col-span-2`, `md:col-span-2`, `xl:col-span-2`, `.sticky` — but NOT `top-*`/`z-*`/`grid-cols-3+`). All net-new layout/nav/chrome CSS is hand-written in the template's existing inline `<style>` block. Do NOT run or require `just spa-css-rebuild`.
3. **Never touch `location.hash` for navigation** — `readUrlIntoStores`/`writeStoresToUrl` (template.html ~L2486-2538) own it as the state serializer. Scrollspy = IntersectionObserver; chip clicks = `scrollIntoView`.
4. Group containers must NOT carry class `widget` (fullscreen/reset-zoom injection iterates `section.widget`). Every existing widget keeps its exact `id` and inner `-body` id.
5. Heading hierarchy after the change: h1 page → h2 section heading → h3 widget title. Preserve current visual sizing of widget titles via CSS.
6. No ticket IDs/plan-§IDs/version refs in code or docs; CHANGELOG.md `[Unreleased]` gets one entry per user-visible change; comments describe current contracts.
7. Conventional Commits; NEVER `Co-Authored-By`. Respect `prefers-reduced-motion` for smooth scrolling.
8. Worktree-absolute paths ONLY (a separate main-branch checkout exists at /Users/emrec/Projects/playground/codelore/).

## Validated interfaces (source-checked via exploration; re-verify line numbers at HEAD before editing)

- `template.html:1173`: `<main class="mx-auto grid grid-cols-1 md:grid-cols-2 gap-7 p-7 dashboard-main">` — the single page grid all 23 widgets sit in. `.dashboard-main { max-width: 2400px }` hand-written at ~L887. Stale comment at ~L142-144 claims `xl:grid-cols-2` (false — fix in passing).
- Widget inventory + current width classes (DOM order): factor-header(xl:col-span-2, L1178) · kpi-tiles(half, L1191) · knowledge-islands(half, L1210) · knowledge-surfaces(half, L1232) · guided-tour(md:col-span-2, L1252) · hotspot-circle-pack(md:col-span-2, L1267) · trends(md:col-span-2, L1460) · cognitive-boxplot(half, 220px, L1519) · module-chord(half, 320px, L1544) · arch-graph(xl:col-span-2, 380px, L1626) · arch-matrix(xl:col-span-2, L1734) · arch-trend(xl:col-span-2, 340px, L1762) · health-trend(xl:col-span-2, L1782) · improvements-feed(half, L1804) · share-bars(xl:col-span-2, L1817) · hotspot-treemap(xl:col-span-2, 320px, L1841) · parallel-coords(xl:col-span-2, 320px, L1866) · kamei-risk(xl:col-span-2, 280px, L1928) · calendar-heatmap(xl:col-span-2, 260px, L1990) · xray-sunburst(xl:col-span-2, L2007) · hotspot-table(xl:col-span-2, L2026) · coupling-sankey(xl:col-span-2, L2056) · delivery-card(half, L2135). Drawer dialog ~L2160.
- Renderers target `getElementById('widget-X-body')` — DOM reordering is render-safe. Paint order = `WIDGETS` registry (`js/00_setup_boot.js:731-755`, 23 entries; factor-header first, rendered synchronously; loop yields between the rest ~L776-800).
- Fullscreen/reset buttons injected by iterating `document.querySelectorAll('section.widget')` (~L203). Resize sweep `resizeAllEchartsIn` (~L226) queries `.widget-body, [id$="-body"], [id$="-chart-host"]` — reuse it for expand-resize.
- Browser-test conventions (`tests/spa_browser_test.rs`): CDP eval via `eval_json`, helpers `element_text(tab,id)` / `echarts_series_len(tab,host_id)` / `attach_exception_sink(tab)`; 16 existing tests must stay green. Integration test `tests/spa_integration_test.rs` asserts embedded JSON + HTML shape (7 tests).
- Guided tour drives the hero via the Alpine brush store — no scroll/DOM-position coupling; keep tour card adjacent to the hero anyway (UX coherence).
- The theme relies on `Alpine.store('theme')`; layout persistence uses `Alpine.store('layout')` with `$persist` — a new store key needs the same idiom (but per spec, collapse state is NOT persisted).

Plan-time UNKNOWNS each task must verify before editing (cheap greps): whether any inline-CSS selector or test asserts widget `<h2>` (grep `h2` in template.html styles + both test files); the exact set of tests referencing removed classes.

---

### Task 1: Section restructure + responsive grid

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/template.html` (the `<main>` region L1173–~L2158, inline `<style>` block)
- Modify: `crates/codelore-lib/tests/spa_integration_test.rs`
- Test: same

**Interfaces:**
- Consumes: the validated inventory above.
- Produces (later tasks rely on these EXACT ids): six group containers `<section id="group-overview">`, `id="group-hotspots"`, `id="group-code-health"`, `id="group-architecture"`, `id="group-knowledge"`, `id="group-delivery"`, each shaped:

```html
<section id="group-overview" class="dash-group" aria-labelledby="group-overview-h">
  <h2 id="group-overview-h" class="dash-group-title">Overview</h2>
  <div class="dash-group-grid">
    <!-- widget <section class="widget card …"> elements, unchanged ids -->
  </div>
</section>
```

- [ ] **Step 1: Verify unknowns.** From the worktree root: `grep -n "h2" crates/codelore-lib/src/output/spa/template.html | head -40` (find style selectors keyed on widget h2); `grep -n "<h2\|h2\b" crates/codelore-lib/tests/spa_integration_test.rs crates/codelore-lib/tests/spa_browser_test.rs`. Note every hit that will break when widget titles become h3; they are updated in Steps 4–5.
- [ ] **Step 2: Write the failing integration tests** (extend `spa_integration_test.rs`, following its existing HTML-assertion style):

```rust
#[test]
fn dashboard_groups_exist_in_order_with_widgets_assigned() {
    let html = build_dashboard_html(); // reuse the file's existing fixture/build helper
    let order = ["group-overview", "group-hotspots", "group-code-health",
                 "group-architecture", "group-knowledge", "group-delivery"];
    let mut last = 0;
    for id in order {
        let pos = html.find(&format!("id=\"{id}\"")).expect(id);
        assert!(pos > last, "{id} out of order");
        last = pos;
    }
    // spot-check assignments: widget id appears AFTER its group id and BEFORE the next group id
    let idx = |needle: &str| html.find(needle).expect(needle);
    assert!(idx("id=\"widget-hotspot-table\"") > idx("id=\"group-hotspots\"")
         && idx("id=\"widget-hotspot-table\"") < idx("id=\"group-code-health\""));
    assert!(idx("id=\"widget-arch-matrix\"") > idx("id=\"group-architecture\"")
         && idx("id=\"widget-arch-matrix\"") < idx("id=\"group-knowledge\""));
    assert!(idx("id=\"widget-calendar-heatmap\"") > idx("id=\"group-delivery\""));
}

#[test]
fn widget_titles_are_h3_under_h2_groups() {
    let html = build_dashboard_html();
    assert!(html.contains("<h2 id=\"group-overview-h\""));
    // a representative widget title demoted
    assert!(html.contains("<h3") && !html.contains("<h2 class=\"widget-title\"") /* adjust to the actual current title markup found in Step 1 */);
}
```

- [ ] **Step 3: Run them** → FAIL (no groups exist).
- [ ] **Step 4: Restructure `template.html`.** (a) `<main>` loses `grid grid-cols-1 md:grid-cols-2 gap-7` (keep `mx-auto p-7 dashboard-main`); it now contains the six `dash-group` sections per the spec's table, each with the exact shape above; widget `<section class="widget …">` blocks are MOVED (cut/paste, unmodified inside except the width classes below and h2→h3). (b) Width normalization inside each `dash-group-grid`: HALF-WIDTH (no span class) are exactly the spec's pairing cards — `improvements-feed` + `cognitive-boxplot` (Code Health), `knowledge-surfaces` + `knowledge-islands` (Knowledge), `delivery-card` (Delivery, alone in its row). EVERY other widget gets `xl:col-span-2` (replace every `md:col-span-2`; add it to factor-header, kpi-tiles, tour, hero, trends, module-chord, and all currently-wide widgets). (c) Widget `<h2>` → `<h3>` throughout (keep classes); add the hand-written CSS in the inline `<style>` block:

```css
/* Section grouping. Hand-written (the pre-built utility bundle is frozen);
   single column below 1280px, two columns above — wide cards span both. */
.dash-group { margin-bottom: 2.5rem; }
.dash-group-title { font-size: 1.35rem; font-weight: 700; margin: 0 0 1rem 0.25rem; }
.dash-group-grid { display: grid; grid-template-columns: 1fr; gap: 1.75rem; }
@media (min-width: 1280px) {
  .dash-group-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
```

Note `xl:col-span-2` only applies ≥1280px which matches the media query exactly. Fix the stale comment at ~L142-144. Update any h2-keyed selectors found in Step 1 (widget title styling must remain visually identical — copy the h2 rules to the h3 selector).
- [ ] **Step 5: Run the new tests + full integration suite + browser suite** (existing browser tests must stay green — they target ids, but verify none asserts an h2 title; fix any per Step 1 notes). Expected: all pass.
- [ ] **Step 6: Commit** `feat(spa): six titled dashboard sections with single-column laptop layout`.

### Task 2: Sticky scrollspy nav + factor-tile jump links

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/template.html` (nav markup after the header; CSS in inline style)
- Modify: `crates/codelore-lib/src/output/spa/js/00_setup_boot.js` (scrollspy wiring) and `crates/codelore-lib/src/output/spa/js/10_helpers_drawer.js` ONLY if the factor-tile renderer lives there (locate `renderFactorHeader`/factor tile click handling first — grep)
- Test: `crates/codelore-lib/tests/spa_browser_test.rs` + `spa_integration_test.rs`

**Interfaces:**
- Consumes: the six `group-*` ids from Task 1.
- Produces: `<nav id="dash-nav">` with six `<button class="dash-nav-chip" data-target="group-…">` chips; JS `initDashNav()` called from boot; a back-to-top button `#dash-top-btn`.

- [ ] **Step 1: Failing tests.** Integration: html contains `id="dash-nav"` with six `data-target` buttons in section order. Browser (new test, follow the file's conventions incl. the extracted helpers; keep bodies <100 lines by reusing/extending helpers):

```text
nav_chip_scrolls_section_into_view_without_hash: boot the coupling fixture dashboard;
click the chip[data-target="group-architecture"]; wait; assert (a) zero console
exceptions, (b) window.scrollY > 0, (c) the group-architecture bounding rect top is
within the viewport, (d) location.hash is unchanged from before the click, (e) the
clicked chip has the active class and the overview chip does not.
```

- [ ] **Step 2: Run** → FAIL (no nav).
- [ ] **Step 3: Implement.** Markup: sticky nav directly below the existing `<header>` (sibling, NOT inside main). Hand-written CSS:

```css
#dash-nav { position: sticky; top: 0; z-index: 30; display: flex; gap: 0.5rem;
  flex-wrap: wrap; padding: 0.5rem 1.75rem; backdrop-filter: blur(6px);
  background: color-mix(in oklab, var(--color-base-100) 88%, transparent); }
.dash-nav-chip { /* reuse DaisyUI btn btn-xs btn-ghost classes on the buttons
  (present in the bundle — verify with grep on the compiled css; if absent,
  hand-write equivalent minimal styles) */ }
.dash-nav-chip.dash-active { /* accent underline/background via existing tokens */ }
#dash-top-btn { position: fixed; right: 1.25rem; bottom: 1.25rem; z-index: 30;
  display: none; }
```

JS in `00_setup_boot.js` (new function, called once at boot end): chip click → `document.getElementById(target).scrollIntoView({behavior: prefersReducedMotion ? 'auto' : 'smooth', block: 'start'})`; account for the sticky nav height via CSS `scroll-margin-top` on `.dash-group` (hand-written, e.g. `3.25rem`) rather than JS math. Scrollspy: one `IntersectionObserver` over the six group sections (`rootMargin: '-40% 0px -55% 0px'`), toggling `dash-active`. Back-to-top: window scroll listener toggles display at >600px, click scrolls to top. NO `location.hash` reads/writes anywhere in this code. Factor tiles: locate the tile render (grep `factor` in js/) and add `data-target` + the same click path for the four tiles → their sections (Code→group-code-health, Architecture→group-architecture, Knowledge→group-knowledge, Delivery→group-delivery); tiles get `cursor:pointer` + `role="link"` + keyboard Enter activation.
- [ ] **Step 4: Run new + full browser/integration suites** → pass.
- [ ] **Step 5: Commit** `feat(spa): sticky scrollspy section nav with factor-tile jump links`.

### Task 3: Collapsible sections + paint-order alignment

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/template.html` (chevron buttons on group headings)
- Modify: `crates/codelore-lib/src/output/spa/js/00_setup_boot.js` (collapse wiring + `WIDGETS` registry reorder)
- Test: `crates/codelore-lib/tests/spa_browser_test.rs`

- [ ] **Step 1: Failing browser test** `section_collapse_and_expand_keeps_charts_sized`: boot; click the group-architecture collapse chevron → the group's grid is hidden (offsetHeight 0 / hidden attr); click again → visible AND a representative chart in it (`echarts_series_len` on the arch-matrix host, or canvas width > 0) is non-zero-sized (the resize-on-expand path).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** Chevron `<button class="dash-collapse" aria-expanded="true" aria-controls="…grid id">` in each group heading row; click toggles a `dash-collapsed` class on the group (`.dash-collapsed .dash-group-grid { display: none; }` hand-written) and flips `aria-expanded`; on EXPAND call the existing `resizeAllEchartsIn(groupEl)` so charts that resized-while-hidden recover. State is NOT persisted (always expanded at load — charts must never boot hidden). Registry: reorder the 23 `WIDGETS` entries in `00_setup_boot.js` to match the new section order (factor-header stays first/synchronous); comment stays truthful.
- [ ] **Step 4: Run suites** → pass. **Step 5: Commit** `feat(spa): collapsible dashboard sections`.

### Task 4: Responsive geometry proof, height QA, docs

**Files:**
- Modify: `crates/codelore-lib/tests/spa_browser_test.rs` (viewport tests)
- Modify: `crates/codelore-lib/src/output/spa/template.html` (only if height QA demands adjustments)
- Modify: `docs/advanced-usage.md` (SPA section: describe the six sections + nav + responsive behavior — current-contract wording), `README.md` (only if it describes the dashboard layout — grep first), `CHANGELOG.md` `[Unreleased]` (one entry covering the overhaul: sections + nav + single-column laptops)
- Test: browser tests

- [ ] **Step 1: Failing viewport tests.** Use the CDP harness's viewport control (grep how the existing tests size the window — e.g. launch args or Emulation; follow the established idiom; if none exists, add a helper `set_viewport(tab, w, h)` beside the other helpers):

```text
laptop_width_renders_single_column: viewport 1100x900; boot; for widget-arch-matrix
and widget-hotspot-table: boundingRect width >= 0.9 * (main content width). PASS
only when both spans are full-row.

desktop_width_pairs_half_widgets: viewport 1500x900; boot; widget-knowledge-surfaces
and widget-knowledge-islands boundingRect tops are equal (same row) and each width
< 0.6 * content width.
```

- [ ] **Step 2: Run** → laptop test FAILS if Task 1 regressed anything (it should pass already — if it passes immediately, verify it FAILS against a deliberately-broken local revert of the grid CSS, then restore; state the red-check in the report).
- [ ] **Step 3: Height QA.** With the 1100px viewport dashboard from Step 1, list each fixed-height chart (boxplot 220, chord 320, arch-graph 380, arch-trend 340, treemap 320, parallel 320, kamei 280, calendar 260) and eyeball-via-measurement: canvas aspect ratio vs the old half-width rendering. Adjust individual heights ONLY where full-width stretching makes the chart unreadable (e.g. the boxplot at 220px full-width may want ~260px). Keep changes minimal and per-widget; note each adjustment in the report.
- [ ] **Step 4: Docs + CHANGELOG** per the file list above. **Step 5: Full gates** (fmt, CI-exact clippy, full workspace suite incl. spa features + browser). **Step 6: Commit** `feat(spa): responsive geometry tests, height tuning, docs`.

---

## Verification (whole-plan)

- Full gates on the final tree; browser suite green with the new tests (~20 tests).
- Real-CLI: `cargo run -p codelore-cli --features spa -- analyze --analysis dashboard --repo . --format spa --output <scratch>/dash.html`, then a headless measurement pass at 1100px and 1500px confirming the spec's geometry rules on the REAL dashboard (not just fixtures).
- Docs guard: `git grep -nE "F[0-9]{3}|PAR-[0-9]|Task-[0-9]" crates/ docs/advanced-usage.md README.md` → no new hits.

## Out of scope (spec) — do NOT implement

URL-addressable sections; per-section lazy rendering; header/theme-toggle changes; mobile-specific work; new widgets or chart-content changes; persisted collapse state.
