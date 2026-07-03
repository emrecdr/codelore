# SPA Bivariate Health×Activity Map Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a **bivariate health×activity** color mode to the SPA circle-pack — encoding code-health *band* and development *activity* in one glyph so a viewer sees the danger quadrant (sick + churning) without swapping lenses — and make it the default map view.

**Architecture:** Evolve the existing `renderHotspotCirclePack(rows, colorMode)` in `output/spa/widgets.js` — its `colorMode` switch already routes cognitive/health/friction/author/ai/knowledge-loss/clones, and the composite `code_health` rows (with `band`/`structural_risk`) are already embedded in the SPA payload by `build_spa_dashboard`. Add: (1) a `bandByPath` lookup joining `d.code_health` by path, (2) a `bivariate` branch computing a 3×3 CVD-safe color from `band` × activity-bucket(`hotspot_score`), (3) a `data-mode="bivariate"` tab + a 3×3 legend, and make it the default. No new libraries; no Rust data-contract change.

**Tech Stack:** Vanilla JS inside the vendored single-file SPA (`widgets.js`), Alpine.js 3, ECharts 6 custom series (existing circle-pack), DaisyUI/Tailwind tokens. Gated behind the `spa` Cargo feature. Tested via `spa_integration_test.rs` (HTML-string assertions, no browser) + `spa_browser_test.rs` (headless Chrome, optional heavier gate).

## Global Constraints

- **Offline single-file SPA is sacrosanct.** No new runtime CDN, no new npm/build step, no new vendored lib in THIS plan. Everything is vanilla JS added to `widgets.js` + markup in `template.html`.
- **No Rust data-contract change.** `build_spa_dashboard` (`crates/codelore-cli/src/main.rs:3016`) already runs `run_code_health` and embeds `code_health: Vec<CodeHealthRow>` (with `structural_risk`, `percentile`, `band`) into the SPA JSON. Consume it in JS; do not touch `SpaDashboard` or the builder.
- **Use existing conventions (HARD RULE).** Mirror the existing per-file-lookup pattern (`cloneCountByPath`, `primaryAuthorByPath`), the existing `colorMode` switch shape, the existing `data-mode` tab + `initHotspotColorToggles` handler, the existing tooltip-host markup. No parallel patterns.
- **Determinism / theme.** Colors come from a fixed palette constant (not random); the mode must survive the theme-rerender path (`registerThemeRerender`) like the other modes. Class names used dynamically must appear as complete literals (Tailwind v4 `@source` scanner requirement — see the `initHotspotColorToggles` comment).
- **Accessibility.** The 3×3 palette must be CVD-safe (colorblind-distinguishable); keep the existing `role="tab"` / `aria-selected` pattern; the danger quadrant must be distinguishable by more than hue.
- **Build + test commands (macOS dev box):** prefix cargo with `MACOSX_DEPLOYMENT_TARGET=15.0`. Build the feature: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa`. Integration test: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`. **Do NOT** run the full `just ci` spa BUILD locally — it fails at the LINK stage on this macOS-26 box (pre-existing deployment-target/linker issue); `cargo build --features spa` (compile/check) exits 0 and GitHub Actions (macOS-15) is unaffected. Gate on `cargo build --features spa` + the integration test.
- Conventional Commits; **never** add `Co-Authored-By: Claude`. No task/version markers in code comments (present-state only).

## Scope notes (this plan is ONE slice of the §5 overhaul)

The design spec §5 (`docs/superpowers/specs/2026-07-02-code-health-and-dashboard-redesign-design.md`) is decomposed into sequential plans, each shipping a working dashboard:

- **THIS plan (3a):** bivariate health×activity color mode + 3×3 legend + default. Delivers the "kill the lens swap" win alone.
- **Deferred to later plans:** bivariate legend as a *click-to-filter* control (needs the Alpine focus-entity store) → **3b/linked brushing**; tabbed drawer + on-demand edge-bundled coupling → **3d**; Observable Plot vendoring + KPI→hero→drawer IA restructure → **3e**. This plan does NOT restructure the widget wall or add libraries — it upgrades one existing widget in place.

## File Structure

- **Modify** `crates/codelore-lib/src/output/spa/widgets.js`:
  - Add a `bandByPath` lookup + `activityBucket()` + `BIVARIATE_PALETTE` const + `bivariateColor()` helper near the other per-file lookups / color helpers.
  - Add the `else if (colorMode === 'bivariate')` branch in the `renderHotspotCirclePack` color switch (~line 1808, before the final `else`).
  - Add a `renderBivariateLegend()` helper + call it from the circle-pack render.
- **Modify** `crates/codelore-lib/src/output/spa/template.html`:
  - Add a `data-mode="bivariate"` tab (first, made default) to `#hotspot-color-toggles` (~line 1055); flip `cognitive` off default.
  - Add a `#bivariate-legend` mount point near the circle-pack.
- **Modify** `crates/codelore-lib/tests/spa_integration_test.rs`: assert the emitted HTML carries the bivariate tab + legend + the palette, and that composite `code_health` band data reaches the payload.

## Reference: verified current code (consume, don't re-derive)

- `renderHotspotCirclePack(rows, colorMode)` — `widgets.js:1494`; `colorMode = colorMode || 'cognitive'` at `:1502`; color switch `:1734–1810` (branches end with `else { leafColor = heatmapColor(ratio); }`).
- Per-file lookups already built in that scope: `primaryAuthorByPath`, `cloneCountByPath` (pattern to mirror). `m = n.data.metrics` is the per-file hotspots row (`m.hotspot_score`, `m.code_health` = hotspots inline value, `m.cognitive`). `n.data.fullPath` is the file path key.
- Composite health: `d.code_health` is `Vec<CodeHealthRow>` = `{ path, cognitive, score, structural_risk, percentile, band }`; the drawer already joins it: `const ch = (d.code_health || []).find(r => r.path === path)` (`:1150`, `:1210`). `band ∈ {"red","yellow","green"}`.
- Color-mode tabs: `template.html:1055` `<div role="tablist" ... id="hotspot-color-toggles">`; each `<button role="tab" data-mode="..." class="tab ... tooltip-host">`. Default tab has `tab-active` + `aria-selected="true"`.
- Toggle handler: `initHotspotColorToggles()` `widgets.js:3963` — reads `data-mode`, swaps `tab-active`/`active`/`aria-selected`, re-renders via `startViewTransition`, sets `currentHotspotColorMode`.
- Integration test shape: `spa_integration_test.rs` (`#![cfg(feature = "spa")]`) builds a `SpaDashboard`, calls `write_spa(&dash, &mut buf, ...)`, and asserts on the resulting HTML `String`.

---

### Task 1: Bivariate color helpers (palette + band/activity join)

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (add helpers near the other color/lookup helpers, e.g. just after `codeHealthColor` at `:857`)
- Test: `crates/codelore-lib/tests/spa_integration_test.rs`

**Interfaces:**
- Produces (JS, in `widgets.js` scope): `BIVARIATE_PALETTE` (9-entry array, CVD-safe), `activityBucket(hotspotScore) → 0|1|2`, `healthBucket(band) → 0|1|2`, `bivariateColor(band, hotspotScore) → cssColor`, and a `bandByPath` map built from `d.code_health`.
- Consumes: `d.code_health` rows (`{path, band}`), per-file `m.hotspot_score`.

- [ ] **Step 1: Write the failing test (palette present in emitted HTML)**

The JS helpers aren't unit-testable in Rust, so we assert the palette constant ships in the emitted SPA (proves the helper code is inlined). In `spa_integration_test.rs`, add to an existing `write_spa`-and-assert test (or a new `#[test]`):

```rust
#[test]
fn spa_embeds_bivariate_palette() {
    let dash = sample_dashboard(); // existing helper that builds a SpaDashboard with code_health rows
    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::spa::write_spa(&dash, &mut buf, "t", std::path::Path::new("."), "now")
        .expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8");
    assert!(html.contains("BIVARIATE_PALETTE"), "bivariate palette constant must ship in widgets.js");
    assert!(html.contains("function bivariateColor"), "bivariateColor helper must ship");
}
```

(If `sample_dashboard()` / the exact `write_spa` signature differ, mirror the existing test in the file — read its top `#[test]` first and copy its setup verbatim.)

- [ ] **Step 2: Run test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test spa_embeds_bivariate_palette`
Expected: FAIL — the HTML does not yet contain `BIVARIATE_PALETTE`.

- [ ] **Step 3: Add the helpers to widgets.js**

Insert after the `codeHealthColor` function (`widgets.js:~857`):

```javascript
  // Bivariate health × activity encoding. A 3×3 matrix: rows = health
  // (green/yellow/red), cols = activity (low/med/high). CVD-safe: the
  // green→red health axis is paired with increasing saturation+darkness on
  // the activity axis, so the danger cell (red × high) is the darkest/most
  // saturated regardless of hue perception. Indexed [health*3 + activity].
  const BIVARIATE_PALETTE = [
    '#c3e8bd', '#8fd18a', '#4fae53', // green  × low/med/high
    '#f6e5a3', '#e8c85a', '#c99a1f', // yellow × low/med/high
    '#f0b0a0', '#dc7050', '#b52d16'  // red    × low/med/high (darkest = danger)
  ];

  // Health band → row index. Unknown/missing band → neutral (handled by caller).
  function healthBucket(band) {
    if (band === 'green') return 0;
    if (band === 'yellow') return 1;
    if (band === 'red') return 2;
    return -1; // unknown
  }

  // Activity (hotspot_score ∈ [0,10]) → column index. Thresholds split the
  // [0,10] range into low (<2), med (<5), high (>=5) — coarse on purpose so
  // the encoding stays legible at 3 levels.
  function activityBucket(hotspotScore) {
    const s = (typeof hotspotScore === 'number') ? hotspotScore : 0;
    if (s < 2) return 0;
    if (s < 5) return 1;
    return 2;
  }

  // Combined bivariate color. Missing band → neutral grey (same convention as
  // the other modes' "no data" grey).
  function bivariateColor(band, hotspotScore) {
    const h = healthBucket(band);
    if (h < 0) return 'rgba(140, 140, 140, 0.55)';
    return BIVARIATE_PALETTE[h * 3 + activityBucket(hotspotScore)];
  }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test spa_embeds_bivariate_palette`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js crates/codelore-lib/tests/spa_integration_test.rs
git commit -m "feat(spa): add bivariate health×activity color helpers"
```

---

### Task 2: Wire the bivariate branch into the circle-pack color switch

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (the `bandByPath` build in `renderHotspotCirclePack` + the color switch)

**Interfaces:**
- Consumes: `BIVARIATE_PALETTE`, `bivariateColor()` (Task 1); `d.code_health`, `m.hotspot_score`, `n.data.fullPath`.
- Produces: `colorMode === 'bivariate'` renders each leaf via `bivariateColor(band, hotspot_score)`.

- [ ] **Step 1: Build the `bandByPath` lookup**

In `renderHotspotCirclePack` (`widgets.js:1494`), near where `cloneCountByPath` / `primaryAuthorByPath` are built (before the `.map` at `:1728`), add:

```javascript
    // Join composite code-health band onto files by path (the circle-pack's
    // `m.code_health` is the hotspots inline cognitive-only value, NOT the
    // composite band — those come from d.code_health). Mirrors cloneCountByPath.
    const bandByPath = {};
    (data.code_health || []).forEach(function (r) { bandByPath[r.path] = r.band; });
```

(Confirm the in-scope name for the embedded data object — it is `data` in this function per the existing `data.clones` usage the clones-mode comment references. If the local is named `d`, use that.)

- [ ] **Step 2: Add the bivariate branch to the color switch**

In the color switch, add a branch immediately before the final `} else {` (`widgets.js:~1808`):

```javascript
            } else if (colorMode === 'bivariate') {
              // Health × activity in one glyph: band (green/yellow/red) ×
              // hotspot activity (low/med/high). The danger quadrant
              // (red × high) is the darkest/most saturated cell — visible
              // without swapping lenses. Missing band → neutral grey.
              leafColor = bivariateColor(
                bandByPath[n.data.fullPath],
                m ? m.hotspot_score : null
              );
```

- [ ] **Step 3: Run the existing SPA integration tests (no regression)**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`
Expected: all existing tests + `spa_embeds_bivariate_palette` PASS. (No new assertion here — this task wires internal render logic the browser test exercises; correctness of the branch is covered by Task 4's browser-level check + the build.)

- [ ] **Step 4: Build the feature to confirm the JS blob still parses/compiles into the crate**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa`
Expected: `Finished` (the `include_str!` of widgets.js always compiles; this confirms no accidental Rust-side breakage).

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/output/spa/widgets.js
git commit -m "feat(spa): render bivariate color mode in the hotspot circle-pack"
```

---

### Task 3: Bivariate tab + 3×3 legend + make it the default map view

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/template.html` (tab + legend mount)
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (`renderBivariateLegend` + call it; flip default mode)
- Test: `crates/codelore-lib/tests/spa_integration_test.rs`

**Interfaces:**
- Consumes: `BIVARIATE_PALETTE` (Task 1); the `#hotspot-color-toggles` tablist + `#bivariate-legend` mount.
- Produces: a default-selected `data-mode="bivariate"` tab; a rendered 3×3 legend; `renderHotspotCirclePack` default arg becomes `'bivariate'`.

- [ ] **Step 1: Write the failing test (tab + legend markup ship)**

Add to `spa_integration_test.rs`:

```rust
#[test]
fn spa_bivariate_is_default_map_mode() {
    let dash = sample_dashboard();
    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::spa::write_spa(&dash, &mut buf, "t", std::path::Path::new("."), "now")
        .expect("write_spa");
    let html = String::from_utf8(buf).expect("utf8");
    // The bivariate tab exists and is the active default.
    assert!(html.contains("data-mode=\"bivariate\""), "bivariate tab must exist");
    // The legend mount ships.
    assert!(html.contains("id=\"bivariate-legend\""), "bivariate legend mount must exist");
    // The bivariate tab carries the active/selected default (cognitive no longer default).
    let biv = html.find("data-mode=\"bivariate\"").unwrap();
    let tab_open = html[..biv].rfind("<button").unwrap();
    let tab_html = &html[tab_open..biv + 40];
    assert!(tab_html.contains("tab-active"), "bivariate tab must be the default (tab-active)");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test spa_bivariate_is_default_map_mode`
Expected: FAIL — no `data-mode="bivariate"` markup yet.

- [ ] **Step 3: Add the bivariate tab as default in template.html**

In `#hotspot-color-toggles` (`template.html:~1055`), add as the FIRST button and remove `tab-active`/`aria-selected="true"` from the current `cognitive` default (set it to `aria-selected="false"`, drop `tab-active`):

```html
        <button type="button" role="tab" data-mode="bivariate"      class="tab tab-active tooltip-host" aria-selected="true">
          <span aria-hidden="true">▦</span> Health×Activity
          <span class="tooltip-popup" role="tooltip">Bivariate map: code-health band (green→red) × development activity (low→high) in one glyph. The darkest cells are unhealthy AND churning — refactor there first. No lens swap needed.</span>
        </button>
```

Change the `cognitive` button's class from `tab tab-active tooltip-host` to `tab tooltip-host` and `aria-selected="true"` to `aria-selected="false"`.

- [ ] **Step 4: Add the legend mount to template.html**

Immediately after the `#hotspot-color-toggles` tablist closing `</div>` (or near the circle-pack container), add:

```html
      <div id="bivariate-legend" class="mt-2" aria-label="Bivariate health × activity legend"></div>
```

- [ ] **Step 5: Add `renderBivariateLegend()` and flip the default in widgets.js**

Change the default color mode at `widgets.js:1502` from `colorMode = colorMode || 'cognitive';` to `colorMode = colorMode || 'bivariate';`. Also set the initial `currentHotspotColorMode` (find its initialization near `initHotspotColorToggles`) to `'bivariate'`.

Add the legend renderer (near the circle-pack helpers) and call it once when the circle-pack renders:

```javascript
  // 3×3 bivariate legend: a small grid keyed to BIVARIATE_PALETTE, axes
  // labelled health (green→red, top→bottom) × activity (low→high, left→right).
  // Only shown when the map is in bivariate mode; a no-op if the mount is absent.
  function renderBivariateLegend() {
    const mount = document.getElementById('bivariate-legend');
    if (!mount) return;
    const cells = BIVARIATE_PALETTE.map(function (c, i) {
      return '<div style="width:14px;height:14px;background:' + c + '" '
        + 'title="health ' + (['healthy','warning','unhealthy'][Math.floor(i / 3)])
        + ' × activity ' + (['low','med','high'][i % 3]) + '"></div>';
    }).join('');
    mount.innerHTML =
      '<div class="text-xs opacity-70 mb-1">Health × Activity</div>' +
      '<div style="display:grid;grid-template-columns:repeat(3,14px);gap:2px">' + cells + '</div>' +
      '<div class="text-xs opacity-50 mt-1">↓ less healthy&nbsp;&nbsp;→ more active</div>';
  }
```

Call `renderBivariateLegend();` inside `renderHotspotCirclePack` after the chart renders (near where the widget finishes its setChart/setOption), so the legend appears with the default view.

- [ ] **Step 6: Run the test to verify it passes**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`
Expected: `spa_bivariate_is_default_map_mode` + `spa_embeds_bivariate_palette` + all existing tests PASS.

- [ ] **Step 7: Build the feature**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa`
Expected: `Finished`.

- [ ] **Step 8: Commit**

```bash
git add crates/codelore-lib/src/output/spa/template.html crates/codelore-lib/src/output/spa/widgets.js crates/codelore-lib/tests/spa_integration_test.rs
git commit -m "feat(spa): bivariate map default + 3×3 legend + color-mode tab"
```

---

### Task 4: Colorblind verification, tooltip fidelity, browser smoke, CHANGELOG

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` (drawer/tooltip: surface the band + activity bucket for the focused file, if not already)
- Modify: `CHANGELOG.md`
- (Optional gate) `crates/codelore-lib/tests/spa_browser_test.rs`

**Interfaces:** none new — closes out the slice.

- [ ] **Step 1: Confirm the file-detail drawer names the band**

Read `showFileDetailDrawer` (`widgets.js:984`) and its code_health join (`:1150`). If the drawer does not already display the composite `band`, add one line to its metrics list (mirror the existing `<dt>Code health</dt><dd>...` row at `:1008`):

```javascript
        + '<dt>Health band</dt><dd>' + (ch ? ch.band : '—') + '</dd>'
```

(Only add if absent — do not duplicate an existing band row.)

- [ ] **Step 2: Colorblind check (manual, documented)**

Verify the `BIVARIATE_PALETTE` danger cell (`#b52d16`, red×high) is the darkest and most saturated of the 9 — so it reads as "worst" under deuteranopia/protanopia where the green↔red hue distinction collapses (the lightness+saturation gradient carries the signal). Document in the palette comment that CVD-safety rests on the lightness axis, not hue alone. No code change if the palette already satisfies this (it does by construction: green row lightest, red×high darkest).

- [ ] **Step 3: (If the browser-tests feature is available) extend the headless smoke test**

Read `spa_browser_test.rs` (`#![cfg(all(feature = "browser-tests", feature = "spa", feature = "test-support"))]`). If it already boots the SPA and asserts the circle-pack renders, add an assertion that the `#bivariate-legend` mount is non-empty and the default active tab is `data-mode="bivariate"`. If the `browser-tests` feature can't run locally, note it and rely on CI (it runs there); do NOT block the slice on a local headless-Chrome run.

Run (only if feasible locally): `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,browser-tests,test-support" --test spa_browser_test`
Expected: PASS, or documented-skipped with CI as the gate.

- [ ] **Step 4: CHANGELOG**

Add under `[Unreleased] > ### Added`:

```markdown
- **SPA bivariate health×activity map.** The dashboard hotspot circle-pack now defaults to a bivariate color mode: each file's glyph encodes its code-health band (green→red) *and* its development activity (low→high) at once, so the danger quadrant (unhealthy **and** churning) is visible without swapping color lenses. A 3×3 legend keys the encoding; the previous single-signal modes (Cognitive, Code Health, Friction, Author, AI, Knowledge-loss, Clones) remain available as tabs. The palette is colorblind-safe (health read via lightness, not hue alone).
```

- [ ] **Step 5: Full local gate + commit**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-lib --features spa` and `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features "spa,test-support" --test spa_integration_test`
Expected: build `Finished`; all integration tests PASS.

```bash
git add crates/codelore-lib/src/output/spa/widgets.js CHANGELOG.md
git commit -m "feat(spa): surface health band in drawer; document bivariate map"
git add crates/codelore-lib/tests/spa_browser_test.rs 2>/dev/null || true
git commit -m "test(spa): assert bivariate default in browser smoke" 2>/dev/null || true
```

---

## Self-Review

**Spec coverage** (design spec §5, the bivariate bullet):
- "Circle-pack stays … lens-swap killed via a bivariate health×activity glyph (3×3 blend)" → Tasks 1–3 (helpers + branch + default). ✓
- "CVD-safe palette + colorblind toggle" → CVD-safe palette (Task 1 by construction, Task 4 verification). The design's optional "colorblind toggle" is satisfied structurally (lightness carries the signal, so no separate toggle is required); a dedicated toggle is deferred as unneeded — noted, not silently dropped. ✓
- "cap 3 encodings/glyph; coverage ring reserved for Phase 2" → this plan uses exactly 2 (hue+activity); no ring. ✓
- "bivariate legend is the primary filter (click a cell to brush)" → **explicitly deferred to 3b/linked brushing** (needs the Alpine focus-entity store, not built here). The legend renders here; the click-to-filter behavior is the next plan. Noted in Scope. ✓
- "deterministic node positions" → the circle-pack's positions are already a function of the (deterministic) hotspots ordering; this plan doesn't change layout, so positions are as deterministic as today. A dedicated position-key is deferred to 3e (cartography on-ramp). ✓ (not regressed)
- "linked brushing / tabbed drawer / Observable Plot / IA restructure" → later plans (Scope). ✓

**Placeholder scan:** no TBD/"handle edge cases"/"similar to". Every code step shows real JS/Rust/HTML; the palette, buckets, and thresholds are explicit. The two "confirm the local variable name is `data` vs `d`" and "mirror the existing test setup" notes are verification instructions with the exact fallback, not placeholders — the executor reads the one referenced line to confirm. ✓

**Type/name consistency:** `BIVARIATE_PALETTE`, `bivariateColor`, `healthBucket`, `activityBucket`, `bandByPath`, `renderBivariateLegend`, `#bivariate-legend`, `data-mode="bivariate"` are used identically across Tasks 1–4. The default-mode flip (`'cognitive'` → `'bivariate'`) is done once (Task 3) at both the render default and `currentHotspotColorMode` init. ✓

**Open risk for the executor:** the SPA's JS is validated primarily by HTML-string assertions + build; the *visual* correctness of the bivariate branch (right cell for right file) is best confirmed by the headless browser test (Task 4) or a manual `--format spa` open. If the browser-tests feature can't run locally, the integration test + build are the gate and CI runs the browser test — call that out in the task report rather than claiming visual verification that wasn't done.
