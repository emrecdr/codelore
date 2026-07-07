// CodeLore SPA dashboard — widget render logic.
// Reads the embedded JSON data block and renders one widget per
// section. Uses d3-hierarchy.pack() for the circle-pack layout
// (CodeScene-equivalent hotspot map) and ECharts for everything
// else.
//
// All globals (`echarts`, `d3`) come from the SHA-pinned vendored
// libraries embedded above this script in the template.

(function () {
  'use strict';

  // ═════════════════════════════════════════════════════════════════
  //  TABLE OF CONTENTS
  // ═════════════════════════════════════════════════════════════════
  //   §1  Data load & IIFE setup
  //   §2  Per-metric provenance definitions (METRIC_DEFS)
  //   §3  Boot — dispatch render() per widget + register re-renderers
  //
  //   §4  Helpers
  //         mountEcharts · bindChartResize ·
  //         buildTooltipHtml · getCssVar · fmtInt · fmtNumberFlex ·
  //         escapeHtml
  //         · token (cached) · invalidateTokenCache ·
  //           registerThemeRerender · resolveCssColor ·
  //           codeHealthColor · heatRamp
  //   §5  Detail drawer
  //         initDetailDrawer · showFileDetailDrawer
  //   §6  Widget: KPI tiles                        — renderKpiTiles
  //   §7  Widget: knowledge islands                — renderKnowledgeIslands
  //   §8  Widget: hotspot circle-pack              — renderHotspotCirclePack
  //   §9  Widget: hotspot table                    — renderHotspotTable
  //   §10 Widget: change-coupling sankey           — renderCouplingSankey
  //   §11 Widget: trends multi-line                — renderTrends
  //   §11b Widget: Kamei delivery-risk sparkline    — renderKameiRiskSparkline
  //   §12 Widget: calendar heatmap                 — renderCalendarHeatmap
  //   §13 Widget: X-Ray sunburst                   — renderXRaySunburst
  //   §14 Controls: hotspot color-mode toggles     — initHotspotColorToggles
  //   §15 Utility helpers
  //         buildFsHierarchy · heatmapColor ·
  //         computePrimaryAuthorByPath · makeAuthorPalette
  //
  //  All function declarations in §4-§15 are hoisted to script scope,
  //  so the boot section at §3 can call them despite being source-
  //  earlier. Only function declarations move freely; let / const /
  //  expression statements must stay in source order.
  // ═════════════════════════════════════════════════════════════════


  // ═════════════════════════════════════════════════════════════════
  //  §1  Data load & IIFE setup
  // ═════════════════════════════════════════════════════════════════

  const dataBlock = document.getElementById('codelore-data');
  if (!dataBlock) {
    console.error('CodeLore: data block not found');
    return;
  }
  let data;
  try {
    data = JSON.parse(dataBlock.textContent);
  } catch (e) {
    console.error('CodeLore: failed to parse data block:', e);
    return;
  }

  // Cross-widget state — declared early so handlers attached inside
  // function declarations can read/write via closure. None of these
  // are read at script-execution time; they're consulted only inside
  // click / Alpine.effect callbacks that fire after this point.
  //
  //   selectedCouplingFile:
  //     The leaf the user last clicked to surface its top-N
  //     Fisher-significant coupling partners as arcs on the
  //     circle-pack. `null` = no overlay.
  //
  //   lastHotspotChart / lastHotspotNodePositions:
  //     The most-recent circle-pack chart instance and its laid-out
  //     {path → (x,y,r)} map. Cached so `updateCouplingArcs()` can
  //     do a partial `setOption` (touching only the arc series) on
  //     click instead of re-running d3.pack().
  let selectedCouplingFile = null;
  let lastHotspotChart = null;
  let lastHotspotNodePositions = null;
  // Module-scoped ref to the circle-pack render payload + the active
  // bivariate quadrant brush set, so updateHotspotBrush() can re-tint leaf
  // opacities on a brush change without re-running buildFsHierarchy/d3.pack.
  let lastCirclePackData = null;
  let brushedPaths = null; // Set<fullPath> for the active quadrant, or null
  // Shared arc-overlay data array. Owned at module scope so both the
  // arc-series renderItem inside renderHotspotCirclePack AND the
  // partial-update call from updateCouplingArcs (which runs on every
  // leaf click without re-running d3.pack) mutate the SAME reference.
  // Critical: arcData.length = 0 + .push(...) — NOT reassignment — is
  // how updateCouplingArcs has to refresh it so the closure inside
  // renderItem keeps seeing live data.
  let arcData = [];

  // Detail drawer state — set up once, reused by every widget that
  // wants to surface per-file details.
  initDetailDrawer();
  // Registry of re-render callbacks. Each ECharts widget pushes its
  // re-render fn so the theme toggle can repaint all of them when
  // CSS variables change. (Theme uses CSS variables for axis / grid
  // colors; ECharts caches the *resolved* values at setOption time
  // so a CSS variable update alone doesn't refresh the chart.)
  window._codeloreRerenderers = [];
  // Cross-widget selection listeners. Each path-aware widget pushes a
  // `function (selectedPath | null) { ... }` callback that updates its
  // emphasis (typically via `chart.dispatchAction({ type: 'highlight'
  // | 'downplay', ... })`). Fired from an `Alpine.effect` in
  // template.html whenever `$store.selection.path` changes — i.e.
  // when the user opens the detail drawer, the file's profile lights
  // up across the trends, parallel-coords, and any other widget that
  // registered a listener.
  // Factory for a source-tagged listener bus. Both the single-file
  // `selection` bus and the SET `brush` bus are identical: an array on
  // `window[arrayName]` plus a register fn that drops any prior entry from
  // the same `source` before pushing, so re-rendering widgets don't leak
  // closures over disposed charts. The firing loops live in template.html
  // and read `window[arrayName]` directly — the factory MUST reassign that
  // global property (not a captured local) so those effects see the latest
  // array.
  function makeListenerBus(arrayName) {
    window[arrayName] = [];
    return function (source, fn) {
      window[arrayName] = window[arrayName].filter(function (l) {
        return l.__source !== source;
      });
      fn.__source = source;
      window[arrayName].push(fn);
    };
  }

  // Register a selection listener keyed by its source widget. Widgets
  // that re-render on theme / Top-N changes (trends, parallel-coords)
  // call this from inside their render fn; tagging by `source` and
  // dropping any prior listener from the same widget before pushing
  // keeps the array bounded (one entry per widget) instead of leaking a
  // fresh closure — over a now-disposed chart — on every re-render. The
  // `__source` tag is inert to the firing loop, which just calls each fn.
  window._codeloreRegisterSelectionListener = makeListenerBus('_codeloreSelectionListeners');

  // Cross-widget quadrant BRUSH listeners — a SET emphasis, distinct from
  // the single-file `selection` bus above. Fired from an Alpine.effect in
  // template.html whenever `$store.brush.cell` changes. Same source-tagged
  // de-dup so re-rendering widgets don't leak closures over disposed charts.
  window._codeloreRegisterBrushListener = makeListenerBus('_codeloreBrushListeners');

  // Screen-reader announcement of the shared selection. A dedicated polite
  // live region (created once, kept visually hidden via .sr-only) speaks the
  // selected file — the visual highlight + aria-current alone are silent to
  // assistive tech. Registered once at boot, not per widget.
  window._codeloreRegisterSelectionListener('a11y-announce', function (selectedPath) {
    let live = document.getElementById('codelore-selection-live');
    if (!live) {
      live = document.createElement('div');
      live.id = 'codelore-selection-live';
      live.className = 'sr-only';
      live.setAttribute('role', 'status');
      live.setAttribute('aria-live', 'polite');
      document.body.appendChild(live);
    }
    live.textContent = selectedPath ? ('Selected ' + selectedPath) : 'Selection cleared';
  });

  // ─── §3a  Fullscreen toggle per widget ──────────────────────────
  // Injects a button into every `<section class="widget">` at boot
  // and wires it to the native HTML5 Fullscreen API. Listens for
  // `fullscreenchange` globally to resize every ECharts instance
  // inside the target panel so charts re-layout to the new viewport
  // size (and back to the panel size when exiting). No Alpine
  // binding — the DOM mutation is one-shot per widget.
  // Reset-zoom handler registry. Each zoom-capable widget registers
  // its panel-id → reset function pair here; the corresponding
  // button (installed by `installWidgetResetZoomButtons` below)
  // looks it up by walking from the click target up to the nearest
  // `section.widget`.
  window._codeloreResetZoomHandlers = window._codeloreResetZoomHandlers || {};
  function installWidgetResetZoomButtons() {
    const RESET_ICON = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 3-6.7"/><polyline points="3 4 3 9 8 9"/></svg>';
    const ids = Object.keys(window._codeloreResetZoomHandlers);
    for (let i = 0; i < ids.length; i++) {
      const panel = document.getElementById(ids[i]);
      if (!panel || panel.querySelector('.widget-reset-zoom-btn')) continue;
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'widget-reset-zoom-btn';
      btn.setAttribute('aria-label', 'Reset zoom');
      btn.title = 'Reset zoom';
      btn.innerHTML = RESET_ICON;
      btn.addEventListener('click', function (e) {
        e.stopPropagation();
        const fn = window._codeloreResetZoomHandlers[ids[i]];
        if (typeof fn === 'function') fn();
      });
      panel.appendChild(btn);
    }
  }

  function installWidgetFullscreenButtons() {
    const FS_ICON = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/></svg>';
    const sections = document.querySelectorAll('section.widget');
    for (let i = 0; i < sections.length; i++) {
      const panel = sections[i];
      if (panel.querySelector('.widget-fullscreen-btn')) continue;
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'widget-fullscreen-btn';
      btn.setAttribute('aria-label', 'Toggle fullscreen');
      btn.title = 'Toggle fullscreen';
      btn.innerHTML = FS_ICON;
      btn.addEventListener('click', function (e) {
        e.stopPropagation();
        if (document.fullscreenElement === panel) {
          document.exitFullscreen && document.exitFullscreen();
        } else if (panel.requestFullscreen) {
          panel.requestFullscreen();
        }
      });
      panel.appendChild(btn);
    }
  }
  function resizeAllEchartsIn(root) {
    if (!root || !window.echarts) return;
    const bodies = root.querySelectorAll('.widget-body, [id$="-body"]');
    for (let i = 0; i < bodies.length; i++) {
      const inst = window.echarts.getInstanceByDom(bodies[i]);
      if (inst) inst.resize();
    }
  }
  // Defer install + bind global fullscreenchange so charts re-layout.
  // The reset-zoom installer fires AFTER the widget renderers have
  // populated `_codeloreResetZoomHandlers` (boot block calls them
  // synchronously above), so a microtask delay is enough.
  function installPanelControls() {
    installWidgetFullscreenButtons();
    installWidgetResetZoomButtons();
  }

  // Promote a `<tr>` (or any container that already has a click handler
  // wired to drill into the detail drawer) into a keyboard-activable
  // control. WCAG 2.1.1 — every operation reachable by mouse must also
  // be reachable by keyboard. Sets `tabindex="0"` to enter the tab
  // order, `role="button"` so screen readers announce it as a control
  // (otherwise it announces as "row" — correct for table semantics but
  // gives no hint that it's interactive), and forwards Enter / Space
  // to the existing click listener so the caller doesn't have to
  // duplicate handler logic. Space is `preventDefault()`-ed so the
  // page doesn't scroll when the row is focused.
  function wireRowKbActivation(rowEl) {
    rowEl.setAttribute('tabindex', '0');
    rowEl.setAttribute('role', 'button');
    rowEl.addEventListener('keydown', function (evt) {
      if (evt.key === 'Enter' || evt.key === ' ') {
        evt.preventDefault();
        evt.currentTarget.click();
      }
    });
  }

  // Wire a `role="tablist"` for the WAI-ARIA Tabs keyboard pattern.
  // The tabs already carry `role="tab"` + `aria-selected` (set either
  // imperatively by initHotspotColorToggles or reactively by Alpine
  // bindings), but without this they have no arrow-key navigation and
  // every tab sits in the tab order. We add:
  //   - a roving tabindex (the selected tab is `tabindex="0"`, the rest
  //     `-1`) so Tab lands on one tab and arrows move within the group;
  //   - Left/Right (and Home/End) handlers that move focus AND activate
  //     (click) the target tab. Activation reuses each tab's existing
  //     click handler so selection state stays owned by whoever owns
  //     it — no duplicated selection logic here.
  // Mirrors `wireRowKbActivation`: focus management + forward to click.
  function wireTablistArrows(tablistEl) {
    const tabs = Array.prototype.slice.call(
      tablistEl.querySelectorAll('[role="tab"]')
    );
    if (!tabs.length) return;
    // Roving tabindex: the currently-selected tab (or the first) is the
    // single tab stop; all others are removed from the sequential order.
    function syncRovingTabindex() {
      var selectedIdx = tabs.findIndex(function (t) {
        return t.getAttribute('aria-selected') === 'true';
      });
      if (selectedIdx < 0) selectedIdx = 0;
      for (var i = 0; i < tabs.length; i++) {
        tabs[i].setAttribute('tabindex', i === selectedIdx ? '0' : '-1');
      }
    }
    syncRovingTabindex();
    function focusAndActivate(idx) {
      const target = tabs[idx];
      if (!target) return;
      // Activate first (updates aria-selected via the tab's own click
      // handler / Alpine binding), then move focus + roving tabindex so
      // the freshly-selected tab is the one carrying tabindex="0".
      target.click();
      target.focus();
      // The selection may settle on a later microtask (Alpine effects),
      // so set the roving tabindex against the just-activated tab
      // directly rather than re-reading aria-selected synchronously.
      for (var i = 0; i < tabs.length; i++) {
        tabs[i].setAttribute('tabindex', i === idx ? '0' : '-1');
      }
    }
    tablistEl.addEventListener('keydown', function (evt) {
      const current = tabs.indexOf(document.activeElement);
      if (current < 0) return;
      var next = null;
      if (evt.key === 'ArrowRight' || evt.key === 'ArrowDown') {
        next = (current + 1) % tabs.length;
      } else if (evt.key === 'ArrowLeft' || evt.key === 'ArrowUp') {
        next = (current - 1 + tabs.length) % tabs.length;
      } else if (evt.key === 'Home') {
        next = 0;
      } else if (evt.key === 'End') {
        next = tabs.length - 1;
      }
      if (next === null) return;
      evt.preventDefault();
      focusAndActivate(next);
    });
  }
  function wireAllTablists() {
    const tablists = document.querySelectorAll('[role="tablist"]');
    for (var i = 0; i < tablists.length; i++) {
      wireTablistArrows(tablists[i]);
    }
  }

  // Expose a chart container to assistive tech as a single labelled
  // image. Canvas/ECharts/d3 charts paint to a bitmap that screen
  // readers can't interpret, so without this they announce as an empty
  // region. `role="img"` collapses the subtree to one node; the
  // `aria-label` is the chart's text alternative — a concise one-line
  // summary derived from the real data each renderer holds. Idempotent:
  // renderers that re-run on theme toggle just overwrite the label.
  function setChartAriaLabel(containerEl, label) {
    if (!containerEl) return;
    containerEl.setAttribute('role', 'img');
    containerEl.setAttribute('aria-label', label);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', installPanelControls);
  } else {
    installPanelControls();
  }
  document.addEventListener('fullscreenchange', function () {
    const target = document.fullscreenElement;
    // 60 ms gives the browser a frame to apply the :fullscreen
    // pseudo-class + height: calc(100vh - 220px) before we ask
    // ECharts to measure.
    setTimeout(function () {
      if (target) {
        resizeAllEchartsIn(target);
      } else {
        resizeAllEchartsIn(document);
      }
    }, 60);
  });

  // ─── §3b  Canvas pan/zoom helper ────────────────────────────────
  // Attach wheel-zoom + drag-pan to a chart container's canvas
  // child via CSS transform. Used by the hotspot circle-pack (a
  // `type: 'custom'` ECharts series that doesn't support native
  // `roam`). The zoom is purely visual (transform-based) so it
  // doesn't fight with ECharts' click handling — clicks still
  // reach the underlying canvas. Double-click resets.
  function attachCanvasZoom(containerEl) {
    if (!containerEl || containerEl._codeloreZoomAttached) return;
    containerEl._codeloreZoomAttached = true;
    var scale = 1, panX = 0, panY = 0;
    var isDragging = false, lastX = 0, lastY = 0, downX = 0, downY = 0;
    function apply() {
      const canvas = containerEl.querySelector('canvas');
      if (!canvas) return;
      canvas.style.transformOrigin = '0 0';
      canvas.style.transform = 'translate(' + panX + 'px, ' + panY + 'px) scale(' + scale + ')';
    }
    function reset() { scale = 1; panX = 0; panY = 0; apply(); }
    containerEl.addEventListener('wheel', function (e) {
      e.preventDefault();
      const rect = containerEl.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      const delta = e.deltaY > 0 ? 1 / 1.12 : 1.12;
      const newScale = Math.max(0.4, Math.min(8, scale * delta));
      // Zoom around the cursor: keep the point under the cursor
      // stationary by adjusting pan to compensate for the scale change.
      panX = cx - (cx - panX) * (newScale / scale);
      panY = cy - (cy - panY) * (newScale / scale);
      scale = newScale;
      apply();
    }, { passive: false });
    containerEl.addEventListener('mousedown', function (e) {
      isDragging = true;
      lastX = e.clientX;
      lastY = e.clientY;
      downX = e.clientX;
      downY = e.clientY;
      containerEl.style.cursor = 'grabbing';
    });
    document.addEventListener('mousemove', function (e) {
      if (!isDragging) return;
      panX += e.clientX - lastX;
      panY += e.clientY - lastY;
      lastX = e.clientX;
      lastY = e.clientY;
      apply();
    });
    document.addEventListener('mouseup', function (e) {
      if (!isDragging) return;
      isDragging = false;
      containerEl.style.cursor = '';
      // If the user barely moved (<4 px), treat as click — don't
      // block the underlying canvas's click handler. (Browsers fire
      // click after mouseup natively; we just need to not eat the
      // event with drag state.)
      const moved = Math.hypot(e.clientX - downX, e.clientY - downY);
      if (moved < 4) { /* allow click to pass through */ }
    });
    containerEl.addEventListener('dblclick', function (e) {
      e.preventDefault();
      reset();
    });
    // Expose for the fullscreenchange handler to reset on
    // enter/exit (otherwise the panned position carries across
    // size changes and the chart drifts off-screen).
    containerEl._codeloreZoomReset = reset;
  }
  // Theme toggle is now an Alpine store registered in template.html
  // (`$store.theme.isDark`). The store's `Alpine.effect` reactively
  // sets `<html data-theme>` AND fires registered re-renderers, so
  // this script doesn't manage the toggle directly anymore.
  // Color-mode toggles for the hotspot circle-pack (cognitive / author / ai).
  initHotspotColorToggles();
  // Arrow-key navigation + roving tabindex for every `role="tablist"`
  // (hotspot color modes, trends, chord/arch depth, kamei, sankey).
  wireAllTablists();


  // ═════════════════════════════════════════════════════════════════
  //  §2  Per-metric provenance definitions
  // ═════════════════════════════════════════════════════════════════
  //
  // Hoisted ABOVE the renderXxx(data) calls in §3: renderKpiTiles and
  // the hotspot-table header both reach METRIC_DEFS via
  // buildTooltipHtml. A const at the bottom of the IIFE hits TDZ
  // when those callers fire — 'Cannot access METRIC_DEFS before
  // initialization' surfaces on every browser load otherwise.
  //
  // Per-metric provenance: formula in plain English + a link to the
  // research-foundations.md section that grounds the metric. Surfaced
  // as `?` tooltips on KPI tiles and table column headers. Static
  // data — no per-repo variation — so it lives in this JS const map
  // rather than the SpaDashboard JSON payload.

  const RESEARCH_FOUNDATIONS_URL =
    'https://github.com/emrecdr/codelore/blob/main/docs/research-foundations.md';
  const METRIC_DEFS = {
    files_analyzed: {
      formula: 'Count of files surviving the live-at-HEAD filter (not deleted in the most recent change touching the path).',
      citation: { label: 'Live-at-HEAD selection', anchor: '#hotspots-' },
    },
    commits: {
      formula: 'Count of commits in the analysed history, after --after / --before / --include-merges filters.',
      citation: { label: 'Behavioural code analysis foundations', anchor: '#authors-' },
    },
    authors: {
      formula: 'Distinct canonical author identities after mailmap consolidation and bot filtering.',
      citation: { label: 'Bird et al. 2011 — Don\'t Touch My Code', anchor: '#authors-' },
    },
    median_code_health: {
      formula: 'code_health = 100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv), where structural_risk is a weighted sum of biomarker intensities. Median is the per-file midpoint across the analysed set.',
      citation: { label: 'code-health composite', anchor: '#code-health-' },
    },
    cognitive_p95: {
      formula: '95th percentile of per-file cognitive complexity (SonarSource formalisation, max across entities in each file).',
      citation: { label: 'Campbell 2018 — Cognitive Complexity', anchor: '#hotspots-' },
    },
    knowledge_islands: {
      formula: 'Files where departed primary author + no substantial other owner intersect with hotspot risk. CodeLore-only signal.',
      citation: { label: 'Knowledge-island detector', anchor: '#knowledge-islands-' },
    },
    coupling_pairs: {
      formula: 'Pairs (a, b) where the two files change in the same commit, gated by min_shared_revs ≥ ${min_shared_revs} and Fisher exact p < ${fisher_significance}.',
      citation: { label: 'Gall et al. 1998 + Tornhill 2015', anchor: '#coupling-' },
    },
    coupling_density: {
      formula: 'edges / (V·(V−1)/2) where V is the candidate node set (files with revs ≥ ${min_revs}) and edges are Fisher-significant coupling pairs (p < ${fisher_significance}).',
      citation: { label: 'Newman 2010 §6.10 — graph density', anchor: '#hotspots-' },
    },
    mi_band: {
      formula: 'Repo-relative percentile band of file-level Maintainability Index (SEI variant). Low = bottom 25% / Moderate = middle 50% / High = top 25%.',
      citation: { label: 'Coleman 1994 + SEI 1997 — why repo-relative', anchor: '#hotspots-' },
    },
    hotspot_score: {
      formula: 'percentile_rank(revisions) × percentile_rank(cognitive) × (100 − code_health) / 4. Range [0, 10] (CodeScene convention).',
      citation: { label: 'Tornhill 2018 — Software Design X-Rays', anchor: '#hotspots-' },
    },
    revisions: {
      formula: 'Count of distinct commits touching the file in the analysed history, after time-bucket and lineage rewrites.',
      citation: { label: 'Revisions analysis', anchor: '#revisions-' },
    },
    code_health: {
      formula: 'Hotspot-table Code Health column: the hotspots analysis\'s own inline signal, 100 × (1 − 0.40 × normalize(cognitive)), empirical range [60, 100]; lower = more cognitively complex. (Distinct from the code-health composite score.)',
      citation: { label: 'hotspots inline code-health', anchor: '#hotspots-' },
    },
    cognitive: {
      formula: 'Max cognitive complexity across entities within the file (SonarSource formalisation, Campbell 2018).',
      citation: { label: 'Campbell 2018 — Cognitive Complexity', anchor: '#hotspots-' },
    },
    mi: {
      formula: '171 − 5.2·log₂(V) − 0.23·CC − 16.2·log₂(SLOC) + 50·sin(√(2.4·comments%)). Values surfaced are the rust-code-analysis `kind=\'unit\'` (file-level) entry.',
      citation: { label: 'Coleman 1994 + SEI 1997', anchor: '#hotspots-' },
    },
    ai_pct: {
      formula: 'COUNT(CASE WHEN ai_attribution IN (\'ai-assisted\', \'ai-authored\') THEN 1 END) × 100 / COUNT(commits touching this file).',
      citation: { label: 'AI authorship classifier (identity::bots)', anchor: '#authors-' },
    },
  };


  // ═════════════════════════════════════════════════════════════════

  // ─── Theme-token helpers — hoisted before the boot section ───
  // The cached `token(name)` helper is read INSIDE per-node color
  // callbacks invoked by `renderXxx(data)` in §3 below. Declaring
  // `const _tokenCache = {}` further down used to TDZ-fault every
  // boot — same shape as the METRIC_DEFS regression earlier. Hoisted
  // here so every render path sees an initialised cache.
  // Theme-token helpers for the per-mode color readers.
  //
  // Two read paths exist by design:
  //
  //   getCssVar(name)  ← UNCACHED. For non-hot-path reads that happen
  //                       at most once per chart setOption (axis colors,
  //                       grid colors, ring fills). Existing widgets.
  //
  //   token(name)      ← CACHED. For hot-path reads inside per-node
  //                       color callbacks (called once per leaf circle).
  //                       Cache invalidated on theme toggle via
  //                       registerThemeRerender so DaisyUI's
  //                       semantic tokens stay theme-accurate.
  //
  // Distinct surfaces because mixing them would either over-cache
  // (sunburst rings going stale on toggle) or under-cache (per-circle
  // re-read of getComputedStyle on 5000-file repos).
  const _tokenCache = {};
  function token(name) {
    if (!(name in _tokenCache)) {
      _tokenCache[name] = getComputedStyle(document.documentElement)
        .getPropertyValue(name).trim();
    }
    return _tokenCache[name];
  }
  function invalidateTokenCache() {
    for (const k in _tokenCache) delete _tokenCache[k];
  }

  // Lazy-init cache for the hidden DOM element `resolveCssColor`
  // uses to round-trip `color-mix()` / `oklch()` expressions through
  // the browser's CSS parser into concrete `rgb(...)` strings ECharts
  // can paint on canvas. Declared above §3 Boot because the boot
  // block synchronously calls widgets (Kamei sparkline, friction
  // mode, etc.) that reach `resolveCssColor()` — `let` bindings are
  // NOT hoisted, so any reference from a function called during boot
  // before this line lands in the Temporal Dead Zone.
  let _colorResolver;

  //  §3  Boot
  // ═════════════════════════════════════════════════════════════════
  //
  // Each widget render runs once at script execution and is registered
  // for the theme-toggle re-render path (`window._codeloreRerenderers`)
  // when its visuals depend on resolved CSS variables.

  // `currentHotspotColorMode` lives at IIFE scope because the user-
  // controlled color-toggle handler (§14) mutates it and the
  // hotspot-circle-pack render closure in WIDGETS below reads the
  // latest value via closure capture. Declared before the registry
  // so the closure has a binding to capture.
  let currentHotspotColorMode = 'bivariate';

  // ─── Widget registry ────────────────────────────────────────────
  // Single source of truth for the boot sequence. Each entry is a
  // `{ name, render, rerender }` triple:
  //
  //   - `name`     — human-readable id for logging/observability
  //   - `render`   — `() => {}` thunk closing over `data` (parsed at
  //                  the top of the IIFE) and any mutable state
  //                  (e.g. `currentHotspotColorMode`). Called once
  //                  at boot AND on every theme-toggle re-render
  //                  pass, unless `rerender` opts out.
  //   - `rerender` — `'theme'` registers via `registerThemeRerender`
  //                  (which invalidates the token cache before
  //                  calling the render — see §8's friction heat
  //                  ramp / health bands). `false` opts out of any
  //                  theme rerender (pure-DOM widgets that don't
  //                  read CSS variables — KPI tiles, KI table,
  //                  hotspot table). Omitted/undefined defaults to
  //                  the regular `_codeloreRerenderers.push` path.
  //
  // Adding a widget = appending one entry to the array. Pre-V4 the
  // boot section had the render call AND the `_codeloreRerenderers
  // .push(() => ...)` line duplicated per widget, which invited
  // theme-rerender drift every time a new widget landed.
  const WIDGETS = [
    { name: 'kpi-tiles',          rerender: false, render: () => renderKpiTiles(data) },
    { name: 'knowledge-islands',  rerender: false, render: () => renderKnowledgeIslands(data.knowledge_islands || []) },
    { name: 'hotspot-circle-pack', rerender: 'theme', render: () => renderHotspotCirclePack(data.hotspots || [], currentHotspotColorMode) },
    { name: 'hotspot-table',      rerender: false, render: () => renderHotspotTable(data.hotspots || []) },
    { name: 'coupling-sankey',    rerender: 'theme', render: () => renderCouplingSankey(data.coupling || []) },
    { name: 'trends',             render: () => renderTrends(data.trends || []) },
    { name: 'kamei-risk-sparkline', render: () => renderKameiRiskSparkline(data.kamei_risk || []) },
    { name: 'hotspot-treemap',    rerender: 'theme', render: () => renderHotspotTreemap(data.hotspots || []) },
    { name: 'parallel-coords',    render: () => renderParallelCoords(data.hotspots || []) },
    { name: 'cognitive-boxplot',  render: () => renderCognitiveBoxplot(data.hotspots || []) },
    { name: 'module-chord',       render: () => renderModuleChord(data.coupling || []) },
    { name: 'arch-graph',         render: () => renderArchGraph(data.imports || [], data.modularity_violations || [], data.unstable_interface || [], data.architecture_roles || []) },
    { name: 'arch-matrix',        render: () => renderArchMatrix(data.imports || [], data.architecture_roles || []) },
    { name: 'arch-trend',         render: () => renderArchTrend(data.architecture_trend || []) },
    { name: 'health-trend',       render: () => renderHealthTrend(data.health_trend || []) },
    { name: 'calendar-heatmap',   rerender: 'theme', render: () => renderCalendarHeatmap(data.daily_commits || []) },
    { name: 'xray-sunburst',      render: () => renderXRaySunburst(data.xray || []) },
  ];

  // F97: boot widgets cooperatively. The synchronous `forEach` blocked
  // first paint until all 14 widgets had run their initial render
  // (ECharts mount + d3.pack layout + initial DOM injection each
  // costs tens of ms on large repos). Now: render the first widget
  // synchronously so the user sees SOMETHING immediately, then yield
  // between each subsequent widget so the browser can paint progress.
  // The theme/regular rerender registration is unchanged (those
  // rerenderers still fire as a single batch on theme toggle — F135
  // already yields between them via _codeloreYieldToMain).
  //
  // `yieldToMain` prefers `scheduler.yield()` on Chrome 129+ and falls
  // back to MessageChannel-postMessage (sub-millisecond, no 4 ms
  // clamp like setTimeout(0)). On browsers without either the
  // `Promise.resolve()` fallback degrades to "run on the next
  // microtask" — still better than fully synchronous.
  //
  // The boot is fire-and-forget: any synchronous follow-up below
  // (window._codeloreShowDetail registration, Alpine store wiring)
  // does NOT depend on widget rendering being complete.
  (async function bootWidgets() {
    for (var i = 0; i < WIDGETS.length; i++) {
      var w = WIDGETS[i];
      w.render();
      if (w.rerender === 'theme') {
        registerThemeRerender(w.render);
      } else if (w.rerender !== false) {
        window._codeloreRerenderers.push(w.render);
      }
      // Yield between widgets, NOT after the last one (a trailing
      // yield is a wasted task). The first widget (kpi-tiles) is
      // cheap structural HTML, so by the time we yield after it the
      // browser has already painted the page chrome + KPI cards.
      if (i < WIDGETS.length - 1) {
        // eslint-disable-next-line no-await-in-loop -- sequential yield is the point
        await yieldToMain();
      }
    }
    // Widget render fns register their reset-zoom handlers lazily as they
    // run in this async loop, so the DOMContentLoaded-time installer ran
    // before those handlers existed (the arch-graph is late enough to miss
    // it reliably). Re-run it now that every widget has rendered; it is
    // idempotent — it skips panels already carrying a reset-zoom button.
    installWidgetResetZoomButtons();
  })();

  // Expose the drawer-show callback so the hotspot-table row-click
  // handler can fire it. Must execute after `data` is loaded (above);
  // order vs the renderXxx() calls is immaterial because this is
  // invoked at user-click time.
  window._codeloreShowDetail = function (path) {
    // Open + populate the drawer FIRST, isolated, so nothing below can
    // leave a blank popup. A failure rendering one row's details is logged
    // to the console (for diagnosis) but the drawer still shows its title
    // and a fallback body.
    try {
      showFileDetailDrawer(path, data);
    } catch (e) {
      console.error('codelore: detail drawer render failed for', path, e);
    }
    // Then publish selection so registered listeners (trends, parallel-
    // coords, etc.) light up the same file across every widget. Best-effort
    // and isolated: a selection-store hiccup must not block the drawer.
    // Defensive ordering — the drawer is opened and populated above, BEFORE
    // this selection-publish, so a throw from the selection store can't
    // pre-empt the drawer from showing. (The blank-popup symptom itself is
    // fixed by the `.detail-drawer .modal-box { opacity: 1 }` CSS override,
    // not by this ordering.) Drawer-close clears the selection via the dialog
    // `close` listener in template.html.
    try {
      if (window.Alpine && window.Alpine.store) {
        const sel = window.Alpine.store('selection');
        if (sel) sel.set(path);
      }
    } catch (e) {
      console.error('codelore: selection publish failed for', path, e);
    }
  };

  // Populate the offboarding picker's author list from the
  // current dataset's entity_ownership. Alpine has auto-initialized
  // by the time this script runs (template.html script order:
  // ALPINE_JS loads → fires alpine:init synchronously → our store
  // listener runs → store is registered), so the store assignment is
  // reactive and the dropdown's x-for template renders against fresh
  // data. Guarded for the no-Alpine fallback path (drawer-only).
  if (window.Alpine && window.Alpine.store) {
    const scenarioStore = window.Alpine.store('scenario');
    if (scenarioStore) {
      scenarioStore.available = computeUniqueAuthors(data.entity_ownership || []);
    }
    // Populate the parallel DOM tree's data. Top-50
    // by hotspot_score keeps the menu navigable for screen readers
    // while still surfacing every high-priority file. Includes only
    // the fields the menu binds against — keeps the reactive proxy
    // light and avoids leaking metric internals into Alpine's
    // reactivity graph.
    const dashboardStore = window.Alpine.store('dashboard');
    if (dashboardStore) {
      const HOTSPOT_TREE_LIMIT = 50;
      // `primary_author` per path is what the off-boarding scenario
      // toggle reads — including it on each list entry lets the
      // template flag affected files reactively as the user picks
      // departures, mirroring the canvas circle-pack's
      // knowledge-loss tint.
      const listPrimaryAuthorByPath =
        computePrimaryAuthorByPath(data.entity_ownership || []);
      // Expose globally so the hotspot-table renderer (different
      // call site, no shared closure) can stamp `data-primary-author`
      // on each row for the off-boarding reactive class toggle.
      window._codelorePrimaryAuthorByPath = listPrimaryAuthorByPath;
      const sorted = (data.hotspots || [])
        .slice()
        .sort(function (a, b) {
          const sa = (typeof a.hotspot_score === 'number') ? a.hotspot_score : -Infinity;
          const sb = (typeof b.hotspot_score === 'number') ? b.hotspot_score : -Infinity;
          return sb - sa;
        })
        .slice(0, HOTSPOT_TREE_LIMIT)
        .map(function (r) {
          return {
            path: r.path,
            code_health: r.code_health,
            hotspot_score: r.hotspot_score,
            primary_author: listPrimaryAuthorByPath[r.path] || null,
          };
        });
      dashboardStore.hotspots = sorted;
    }
  }


  // ═════════════════════════════════════════════════════════════════
  //  §4-§15 — FUNCTION DECLARATIONS (hoisted to IIFE scope)
  // ═════════════════════════════════════════════════════════════════


  // ─── §4  Helpers ─────────────────────────────────────────────────

  // Bind a ResizeObserver to keep `chart` sized to `container`. Stores
  // the observer on `container._codeloreResizeObserver` and disconnects
  // any prior observer first so re-renders (color-mode toggles, theme
  // switches) don't accumulate listeners across the SPA's lifetime.
  // ResizeObserver also fires when the container's own dimensions
  // change (e.g. sidebar collapses) — strictly better than
  // `window.addEventListener('resize')`, which only fires on viewport
  // changes and leaks one closure per re-render.
  function bindChartResize(chart, container) {
    if (container._codeloreResizeObserver) {
      container._codeloreResizeObserver.disconnect();
    }
    const ro = new ResizeObserver(function () { chart.resize(); });
    ro.observe(container);
    container._codeloreResizeObserver = ro;
  }

  // Single-call helper for the ECharts widget lifecycle:
  //   1. Dispose any prior instance bound to the same DOM node — without
  //      this, re-render leaks the old instance (caches stale state +
  //      keeps its event listeners alive).
  //   2. Create a fresh instance with the canvas renderer.
  //   3. Wire the resize observer so the chart tracks container size.
  // Five widgets used to repeat this triplet verbatim; the next bug fix
  // for any one of them would have had to touch every site (the dispose-
  // drift class of regression that earlier work explicitly fixed).
  //
  // Callers still set the option via `chart.setOption(...)` themselves —
  // each widget's option shape is bespoke, so wrapping that step in here
  // would just hide the widget's most-edited surface behind an argument.
  function mountEcharts(container) {
    const prior = echarts.getInstanceByDom(container);
    if (prior) prior.dispose();
    const chart = echarts.init(container, null, { renderer: 'canvas' });
    bindChartResize(chart, container);
    return chart;
  }

  // Build the HTML for a `?` tooltip. Returns the trigger button plus
  // an absolutely-positioned popup with the formula + citation link.
  // Caller is responsible for putting `.tooltip-host` on the wrapping
  // element so the popup positions correctly.
  // V5: substitute `${key}` placeholders in METRIC_DEFS formula
  // strings with the run's effective threshold values (data.options).
  // Without this the tooltip read "Fisher exact p < fisher_significance"
  // (the parameter NAME) instead of "Fisher exact p < 0.05" (the value
  // actually in force on this run). Unknown keys are left as-is so a
  // stale METRIC_DEFS placeholder shows as the literal `${key}` and
  // the gap is visible during review rather than silently filled with
  // `undefined`. Pure formatting — runs OUTSIDE escapeHtml so the `<`,
  // `>`, etc. inside formulas are escaped together with the
  // substituted values.
  function interpolate(formula, opts) {
    return formula.replace(/\$\{([a-z_][a-z0-9_]*)\}/g, function (match, key) {
      return Object.prototype.hasOwnProperty.call(opts, key) ? String(opts[key]) : match;
    });
  }

  function buildTooltipHtml(defKey) {
    const def = METRIC_DEFS[defKey];
    if (!def) return '';
    const citationHref = RESEARCH_FOUNDATIONS_URL + (def.citation.anchor || '');
    const formulaResolved = interpolate(def.formula, data.options || {});
    return '<span class="tooltip-host">' +
      '<button type="button" class="tooltip-trigger" aria-label="What does this metric mean?" tabindex="0">?</button>' +
      '<span class="tooltip-popup" role="tooltip">' +
        '<strong>Formula</strong>' +
        '<div class="tooltip-formula">' + escapeHtml(formulaResolved) + '</div>' +
        '<div class="tooltip-citation">📖 <a href="' + escapeHtml(citationHref) + '" target="_blank" rel="noopener">' +
          escapeHtml(def.citation.label) + ' ↗</a></div>' +
      '</span>' +
    '</span>';
  }

  // Read a CSS variable from the current theme; used by ECharts widgets
  // that pull axis / grid colors from the same palette as the CSS shell.
  function getCssVar(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  function fmtInt(v) {
    if (typeof v !== 'number' || !isFinite(v)) return '';
    return Math.round(v).toLocaleString('en-US');
  }

  function fmtNumberFlex(v, decimals) {
    if (typeof v !== 'number' || !isFinite(v)) return '';
    return v.toFixed(decimals);
  }

  function escapeHtml(s) {
    return String(s || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }


  // Register a theme-aware rerenderer that uses cached token() reads.
  // Wraps fn so the token cache is flushed before fn() runs, so the
  // first read after a theme toggle sees the new --color-* values.
  // Widgets that read theme colors only via getCssVar() should keep
  // calling window._codeloreRerenderers.push(fn) directly — they don't
  // need the cache flush.
  function registerThemeRerender(fn) {
    window._codeloreRerenderers.push(function () {
      invalidateTokenCache();
      fn();
    });
  }

  function resolveCssColor(cssExpr) {
    if (!_colorResolver) {
      _colorResolver = document.createElement('div');
      _colorResolver.style.cssText =
        'position:absolute;visibility:hidden;pointer-events:none;';
      document.body.appendChild(_colorResolver);
    }
    _colorResolver.style.color = cssExpr;
    return getComputedStyle(_colorResolver).color;
  }

  // CodeScene-equivalent three-band code-health color. CodeLore's
  // scale is 0-100 (SonarSource formalisation) where CodeScene's is
  // 1-10; the cutoffs scale accordingly. Returns DaisyUI semantic
  // tokens so the bands theme-adapt (green/yellow/red in both light
  // and dark modes). Falls back to --color-base-content (dim
  // foreground) when score is null (unsupported language).
  function codeHealthColor(score) {
    if (score == null) return token('--color-base-content');
    if (score <= 40)   return token('--color-error');
    if (score <= 70)   return token('--color-warning');
    return token('--color-success');
  }

  // Bivariate health × activity encoding. A 3×3 matrix: rows = health
  // (green/yellow/red), cols = activity (low/med/high). CVD-safety rests on
  // LIGHTNESS, not hue: the health axis descends monotonically in lightness
  // (green row lightest → red row darkest, with a clear gap between rows at
  // every activity column), and the activity axis darkens left→right within a
  // row. So the danger cell (red × high) is the darkest of all nine and the
  // health bands stay separable for deuteranopia/protanopia (amber-leaning
  // yellows keep the middle row well below the green row in lightness).
  // Indexed [health*3 + activity].
  const BIVARIATE_PALETTE = [
    '#d4efd0', '#a3d99b', '#6bbf6b', // green  × low/med/high (lightest row)
    '#d9b74a', '#c19a2e', '#a37d18', // yellow × low/med/high (amber, mid lightness)
    '#c65c46', '#a83c28', '#7d2414'  // red    × low/med/high (darkest row; danger = last)
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

  // Continuous heat ramp from --color-warning to --color-error in OKLCH
  // space. Perceptually uniform — midpoint stays in the orange family,
  // unlike sRGB / HSL interpolation which mudbrowns through grey at
  // 50%. Browser-native via CSS color-mix(), no JS color math.
  // ratio ∈ [0, 1]; returns a concrete rgb() string ready for canvas.
  function heatRamp(ratio) {
    const pct = Math.max(0, Math.min(1, ratio)) * 100;
    const expr = 'color-mix(in oklch, ' + token('--color-warning') +
                 ', ' + token('--color-error') + ' ' + pct + '%)';
    return resolveCssColor(expr);
  }

  // View Transitions API wrapper. Runs updateFn inside the browser's
  // snapshot-then-animate boundary so colour-mode swaps crossfade
  // smoothly. Graceful no-op fallback on browsers without the API
  // (Safari < 18, Firefox < 124 — both currently shipping).
  // The transition itself is purely visual; correctness is in
  // updateFn, which runs synchronously either way.
  function startViewTransition(updateFn, scope) {
    if (typeof document.startViewTransition !== 'function') {
      updateFn();
      return;
    }
    // Honor `prefers-reduced-motion: reduce` — users with vestibular
    // disorders / motion sensitivity opt out of cross-page transitions
    // at the OS level. `startViewTransition` does not gate on the
    // media query itself, so without this check the crossfade still
    // runs. Apply the update synchronously instead so the swap is
    // instant.
    const prefersReducedMotion =
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (prefersReducedMotion) {
      updateFn();
      return;
    }
    // Element-scoped View Transitions (Chrome 147+) animate only the
    // subtree rooted at `scope` and leave the rest of the dashboard
    // interactive during the transition. The document-scoped form
    // blocks every other widget until the crossfade settles —
    // theme-toggle and `show all` were the worst offenders. When
    // `scope` is missing or the browser doesn't support per-element
    // transitions, fall back to the document-scoped path.
    if (scope && typeof scope.startViewTransition === 'function') {
      scope.startViewTransition(updateFn);
      return;
    }
    document.startViewTransition(updateFn);
  }

  // ─── yieldToMain ────────────────────────────────────────────────
  // Cooperative yield primitive: surrender the main thread so the
  // browser can paint queued work and process pending input before
  // the caller resumes. Prefers `scheduler.yield()` (Chrome 129+,
  // continuation-prioritised) and falls back to a `MessageChannel`
  // postMessage trick on browsers without scheduler — that pattern
  // beats `setTimeout(0)` because postMessage isn't clamped to 4ms
  // and runs at the same priority as input. Used to break up the
  // hotspot-table 'Show all' rebuild and the theme-toggle re-layout
  // so user input stays responsive during heavy renders.
  //
  // References:
  // - https://developer.chrome.com/blog/use-scheduler-yield
  //
  // The MessageChannel fallback is lazy-initialized inside the function
  // so browsers with `scheduler.yield()` (Chrome 129+, the common case
  // for this dashboard's audience) never allocate one at module load.
  let _yieldFallbackChannel = null;
  let _yieldFallbackInitialized = false;
  function yieldToMain() {
    if (typeof scheduler === 'object' && scheduler && typeof scheduler.yield === 'function') {
      return scheduler.yield();
    }
    if (!_yieldFallbackInitialized) {
      _yieldFallbackInitialized = true;
      if (typeof MessageChannel === 'function') {
        _yieldFallbackChannel = new MessageChannel();
      }
    }
    if (_yieldFallbackChannel) {
      return new Promise(function (resolve) {
        _yieldFallbackChannel.port1.onmessage = function () { resolve(); };
        _yieldFallbackChannel.port2.postMessage(0);
      });
    }
    return Promise.resolve();
  }
  // Expose on `window` so the template's Alpine.effect (which lives
  // in a different lexical scope) can yield between rerenderers
  // without duplicating the feature-detection logic. F135 fix.
  window._codeloreYieldToMain = yieldToMain;


  // ─── §5  Detail drawer (cross-widget click target) ────────────────

  function initDetailDrawer() {
    // No-op when Alpine is present: the drawer's `@click` on the
    // close button and `@keydown.escape.window` on the aside drive
    // open/close via `$store.detail` — no imperative listeners
    // needed here. Kept as a stub so the boot call at §1 keeps a
    // single, uniform shape across the Alpine / no-Alpine branches.
    //
    // Fallback path (Alpine missing for any reason): re-attach the
    // legacy imperative listeners so the drawer still works.
    if (window.Alpine) return;
    const closeBtn = document.getElementById('drawer-close');
    if (closeBtn) {
      closeBtn.addEventListener('click', function () {
        const drawer = document.getElementById('file-detail-drawer');
        if (drawer) drawer.hidden = true;
      });
    }
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') {
        const drawer = document.getElementById('file-detail-drawer');
        if (drawer) drawer.hidden = true;
      }
    });
  }

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

  function showFileDetailDrawer(path, d) {
    const drawer = document.getElementById('file-detail-drawer');
    const title = document.getElementById('drawer-title');
    const body = document.getElementById('drawer-body');
    if (!drawer || !title || !body) return;
    // Defensive: a malformed row (no resolvable path/entity → empty/non-
    // string `path`) must never produce a blank, titleless drawer. Fall
    // back to a stable label; `hasPath` also drives a clearer empty-body
    // message below so the drawer is always self-explanatory.
    const hasPath = typeof path === 'string' && path.length > 0;
    title.textContent = hasPath ? path : 'File details';

    var overviewHtml = '';
    var couplingHtml = '';
    var peopleHtml = '';

    // All section lookups are wrapped so one row's malformed data can't
    // blank the drawer: partial html built before any throw is still shown,
    // and the underlying error is surfaced to the console for diagnosis.
    try {
    // Section: hotspot row
    const hot = (d.hotspots || []).find(function (r) { return r.path === path; });
    if (hot) {
      overviewHtml += '<h4>Hotspot</h4><dl>' +
        '<dt>Revisions</dt><dd>' + fmtInt(hot.revisions) + '</dd>' +
        '<dt>Cognitive</dt><dd>' + fmtNumberFlex(hot.cognitive, 0) + '</dd>' +
        '<dt>Code health</dt><dd>' + fmtNumberFlex(hot.code_health, 1) + '</dd>' +
        '<dt>Hotspot score</dt><dd>' + fmtNumberFlex(hot.hotspot_score, 2) + '</dd>' +
        '</dl>';
    }

    // Section: knowledge island. Payload uses `entity` here (not
    // `path` like the other tables) — check both so the lookup
    // succeeds regardless of which field carries the path.
    const ki = (d.knowledge_islands || []).find(function (r) {
      return (r.path || r.entity) === path;
    });
    if (ki) {
      peopleHtml += '<h4>Knowledge island</h4><dl>' +
        '<dt>Primary author</dt><dd>' + escapeHtml(ki.main_author || '') + '</dd>' +
        '<dt>Ownership</dt><dd>' + fmtNumberFlex(ki.ownership_pct, 1) + ' %</dd>' +
        '<dt>Days since active</dt><dd>' + fmtInt(ki.days_since_main_active) + '</dd>' +
        '<dt>Total LoC</dt><dd>' + fmtInt(ki.total_loc) + '</dd>' +
        '</dl>';
    }

    // Section: coupling partners. Each partner is annotated with
    // its primary author and (when scenario.departed contains that
    // author) a knowledge-loss badge — same reactive signal the
    // hotspot table + KI list use. Authors are looked up from the
    // window-global map populated at boot.
    const primaryAuthorByPath = window._codelorePrimaryAuthorByPath || {};
    const departedSet = (window.Alpine && window.Alpine.store && window.Alpine.store('scenario'))
      ? new Set(window.Alpine.store('scenario').departed)
      : new Set();
    const partners = (d.coupling || []).filter(function (r) {
      return r.entity_a === path || r.entity_b === path;
    });
    if (partners.length) {
      couplingHtml += '<h4>Coupling partners</h4><ul class="drawer-partners">';
      for (var i = 0; i < Math.min(partners.length, 20); i++) {
        const p = partners[i];
        const other = (p.entity_a === path) ? p.entity_b : p.entity_a;
        const partnerAuthor = primaryAuthorByPath[other] || '';
        const isDeparted = partnerAuthor && departedSet.has(partnerAuthor);
        couplingHtml += '<li' + (isDeparted ? ' class="drawer-partner-departed"' : '') + '>' +
          '<code>' + escapeHtml(other) + '</code>' +
          ' — ' + fmtInt(p.shared) + ' shared revs' +
          (p.degree != null ? (' (' + fmtNumberFlex(p.degree, 1) + '% coupling)') : '') +
          (partnerAuthor ? ' <span class="drawer-author">' + escapeHtml(partnerAuthor) + '</span>' : '') +
          (isDeparted ? ' <span class="ki-knowledge-loss-badge">knowledge-loss</span>' : '') +
          '</li>';
      }
      if (partners.length > 20) {
        couplingHtml += '<li>… ' + (partners.length - 20) + ' more</li>';
      }
      couplingHtml += '</ul>';
    }

    // Section: top contributors. Aggregates entity_ownership rows
    // for the file by author and ranks by total LoC contribution.
    // Useful for "who else has touched this besides the primary
    // author?" — answers the bus-factor recovery question without
    // leaving the drawer.
    const contribRows = (d.entity_ownership || []).filter(function (r) {
      return r.entity === path;
    });
    if (contribRows.length) {
      const byAuthor = {};
      for (var ci = 0; ci < contribRows.length; ci++) {
        const r = contribRows[ci];
        if (!byAuthor[r.author]) byAuthor[r.author] = { added: 0, deleted: 0 };
        byAuthor[r.author].added += (r.added || 0);
        byAuthor[r.author].deleted += (r.deleted || 0);
      }
      // Drop zero-contribution entries. Entity_ownership keeps a row
      // for any commit that touched the path; renames / reverts can
      // net to 0 added + 0 deleted and produce misleading "0%"
      // contributors (especially when flagged as knowledge-loss —
      // if they didn't contribute lines, their departure doesn't
      // actually lose knowledge of this file). Only show authors
      // with substantive contribution.
      const contribList = Object.keys(byAuthor)
        .filter(function (a) { return (byAuthor[a].added + byAuthor[a].deleted) > 0; })
        .map(function (a) {
          return { author: a, added: byAuthor[a].added, deleted: byAuthor[a].deleted };
        })
        .sort(function (a, b) {
          return (b.added + b.deleted) - (a.added + a.deleted);
        });
      if (contribList.length) {
        const total = contribList.reduce(function (acc, r) { return acc + r.added + r.deleted; }, 0) || 1;
        peopleHtml += '<h4>Top contributors</h4><ul class="drawer-partners">';
        for (var pi = 0; pi < Math.min(contribList.length, 5); pi++) {
          const c = contribList[pi];
          const pct = Math.round(((c.added + c.deleted) / total) * 100);
          const cDeparted = departedSet.has(c.author);
          peopleHtml += '<li' + (cDeparted ? ' class="drawer-partner-departed"' : '') + '>' +
            escapeHtml(c.author) +
            ' — ' + pct + '% (<span class="drawer-author">+' + fmtInt(c.added) + ' / -' + fmtInt(c.deleted) + '</span>)' +
            (cDeparted ? ' <span class="ki-knowledge-loss-badge">knowledge-loss</span>' : '') +
            '</li>';
        }
        if (contribList.length > 5) {
          peopleHtml += '<li>… ' + (contribList.length - 5) + ' more contributors</li>';
        }
        peopleHtml += '</ul>';
      }
    }

    // Section: functions (from X-ray complexity scan). Lists the
    // file's top-complexity functions inline so the user doesn't
    // have to drill into the sunburst widget separately.
    const fileFunctions = (d.xray || [])
      .filter(function (r) { return r.path === path; })
      .sort(function (a, b) {
        const ca = (typeof a.cognitive === 'number') ? a.cognitive : 0;
        const cb = (typeof b.cognitive === 'number') ? b.cognitive : 0;
        return cb - ca;
      });
    if (fileFunctions.length) {
      overviewHtml += '<h4>Functions</h4><ul class="drawer-partners">';
      for (var fi = 0; fi < Math.min(fileFunctions.length, 8); fi++) {
        const f = fileFunctions[fi];
        overviewHtml += '<li><code>' + escapeHtml(f.function || '(anonymous)') + '</code>' +
          ' — cognitive <b>' + fmtNumberFlex(f.cognitive, 0) + '</b>' +
          (typeof f.start_line === 'number' ? ' <span class="drawer-author">L' + f.start_line + '</span>' : '') +
          '</li>';
      }
      if (fileFunctions.length > 8) {
        overviewHtml += '<li>… ' + (fileFunctions.length - 8) + ' more functions</li>';
      }
      overviewHtml += '</ul>';
    }

    // Section: clone groups. If the file appears in any clone
    // family, surface the count + group IDs so the user can
    // cross-reference with the Clones color mode in the hotspot
    // circle-pack.
    const cloneRow = (d.clones || []).find(function (r) { return r.path === path; });
    if (cloneRow && (cloneRow.groups || cloneRow.group_count || cloneRow.clone_groups)) {
      const groupCount = cloneRow.groups || cloneRow.group_count || cloneRow.clone_groups;
      overviewHtml += '<h4>Clones</h4><dl>' +
        '<dt>Clone groups</dt><dd>' + fmtInt(groupCount) + '</dd>' +
        '</dl>';
    }

    // Section: code health
    const ch = (d.code_health || []).find(function (r) { return r.path === path; });
    if (ch) {
      overviewHtml += '<h4>Code health</h4><dl>' +
        '<dt>Score</dt><dd>' + fmtNumberFlex(ch.score, 1) + '</dd>' +
        '<dt>Cognitive</dt><dd>' + fmtNumberFlex(ch.cognitive, 0) + '</dd>' +
        '<dt>Health band</dt><dd>' + (ch.band || '—') + '</dd>' +
        '</dl>';
    }

    } catch (e) {
      console.error('codelore: drawer section render failed for', path, e);
    }

    // Radar lives at the top of the Overview panel (the default-visible tab —
    // ECharts needs a laid-out container with height). Its mount id is
    // unchanged so renderDrawerRadar still finds it after body.innerHTML.
    const radarDiv = '<div id="drawer-radar" style="height: 220px; margin-bottom: 14px;"></div>';

    // The radar is always present, so Overview's emptiness is judged on its
    // OTHER sections: when a metric-sparse file has none, the radar self-hides
    // and we show a muted message instead of a visually blank default tab.
    const overviewInner = radarDiv + (overviewHtml.length
      ? overviewHtml
      : ('<div class="empty">' + (hasPath
          ? 'No overview metrics for this file — it may be below the minimum-revision threshold.'
          : 'This row had no resolvable file path, so no metrics could be looked up.') + '</div>'));

    body.innerHTML =
      drawerTabBar() +
      drawerPanel('drawer-panel-overview', 'drawer-tab-overview', overviewInner, '') +
      drawerPanel('drawer-panel-coupling', 'drawer-tab-coupling', couplingHtml,
        'No change-coupling partners recorded for this file.') +
      drawerPanel('drawer-panel-people', 'drawer-tab-people', peopleHtml,
        'No ownership or contributor data for this file.');
    wireDrawerTabs(body);

    // Render the radar after body.innerHTML so the container exists.
    // Isolated: an ECharts failure on sparse/edge-case data must not wipe
    // the drawer body that's already populated above — just hide the radar.
    try {
      renderDrawerRadar(path, d);
    } catch (e) {
      console.error('codelore: drawer radar render failed for', path, e);
      const radarEl = document.getElementById('drawer-radar');
      if (radarEl) radarEl.style.display = 'none';
    }

    // Native <dialog>. Alpine.store('detail') routes show()/hide()
    // through showModal()/close(). Fallback path for environments
    // without Alpine: call showModal() directly. Both paths
    // converge on the same native modal flow.
    if (window.Alpine) {
      window.Alpine.store('detail').show();
    } else if (typeof drawer.showModal === 'function' && !drawer.open) {
      drawer.showModal();
    }
  }

  // Drawer radar — six-axis behavioural profile for the file shown
  // in the drawer. Sources every axis from the live dataset and
  // normalises against the run's max for each metric so the shape
  // reads "how this file compares to the rest of THIS analysis."
  function renderDrawerRadar(path, d) {
    const container = document.getElementById('drawer-radar');
    if (!container) return;
    const hotspots = d.hotspots || [];
    const ch = (d.code_health || []).find(function (r) { return r.path === path; });
    const hot = hotspots.find(function (r) { return r.path === path; });
    if (!hot) {
      // Files with no hotspot row are typically empty / near-empty
      // (`__init__.py`, generated stubs) that have no functions or
      // classes for the complexity scan to measure. There's nothing
      // useful to plot on the radar — but the Overview, Coupling and
      // People tabs still surface what we DO know about this file
      // (ownership, author, total LoC, change-coupling partners). Hide
      // the radar container so the Overview tab shows its muted message
      // instead of wasting 220px on an empty chart.
      container.style.display = 'none';
      return;
    }
    // Make sure a previous hidden render doesn't keep the container
    // collapsed when we switch to a file that DOES have metrics.
    container.style.display = '';
    // Per-axis run-relative anchors.
    let maxCog = 0, maxRev = 0, maxAI = 0, maxCoup = 0;
    for (var i = 0; i < hotspots.length; i++) {
      const r = hotspots[i];
      if (r.cognitive > maxCog) maxCog = r.cognitive;
      if (r.revisions > maxRev) maxRev = r.revisions;
      if (typeof r.ai_pct === 'number' && r.ai_pct > maxAI) maxAI = r.ai_pct;
    }
    const coupling = d.coupling || [];
    const couplingCounts = {};
    for (var ci = 0; ci < coupling.length; ci++) {
      const c = coupling[ci];
      couplingCounts[c.entity_a] = (couplingCounts[c.entity_a] || 0) + 1;
      couplingCounts[c.entity_b] = (couplingCounts[c.entity_b] || 0) + 1;
    }
    for (var ck in couplingCounts) {
      if (couplingCounts[ck] > maxCoup) maxCoup = couplingCounts[ck];
    }
    const fileCouplingCount = couplingCounts[path] || 0;

    const axes = [
      { name: 'Cognitive', max: 1.0, value: maxCog ? hot.cognitive / maxCog : 0 },
      { name: 'Churn',     max: 1.0, value: maxRev ? hot.revisions / maxRev : 0 },
      { name: 'Coupling',  max: 1.0, value: maxCoup ? fileCouplingCount / maxCoup : 0 },
      { name: 'MI',        max: 1.0, value: typeof hot.mi_rank === 'number' ? Math.max(0, Math.min(1, hot.mi_rank)) : 0 },
      { name: 'AI%',       max: 1.0, value: (typeof hot.ai_pct === 'number' && maxAI) ? hot.ai_pct / maxAI : 0 },
      { name: 'Health',    max: 1.0, value: ch && typeof ch.score === 'number' ? Math.max(0, 1 - ch.score / 100) : 0 },
    ];

    const chart = mountEcharts(container);
    chart.setOption({
      radar: {
        indicator: axes.map(function (a) { return { name: a.name, max: a.max }; }),
        radius: '65%',
        axisName: { color: getCssVar('--fg-dim'), fontSize: 10 },
        splitLine: { lineStyle: { color: getCssVar('--bg-elev-2') } },
      },
      series: [{
        type: 'radar',
        data: [{
          value: axes.map(function (a) { return a.value; }),
          name: 'profile',
          areaStyle: { color: 'rgba(245, 158, 11, 0.25)' },
          lineStyle: { color: token('--color-warning'), width: 2 },
          itemStyle: { color: token('--color-warning') },
        }],
      }],
    });
  }


  // ─── §6  Widget: KPI tiles ────────────────────────────────────────

  function renderKpiTiles(d) {
    const container = document.getElementById('widget-kpi-tiles-body');
    if (!container) return;

    const tiles = [];
    const hotspots = d.hotspots || [];
    const codeHealth = d.code_health || [];
    const coupling = d.coupling || [];
    const knowledgeIslands = d.knowledge_islands || [];

    // Pull summary metrics (commits, entities, etc.) by name from the
    // summary[] array since it's a (metric, value) shape.
    const summaryByName = {};
    for (var i = 0; i < (d.summary || []).length; i++) {
      const s = d.summary[i];
      summaryByName[s.metric] = s.value;
    }

    // Tile 1: tracked files
    const fileCount = hotspots.length || codeHealth.length || 0;
    tiles.push({
      label: 'Files analyzed',
      defKey: 'files_analyzed',
      value: fmtInt(fileCount),
      sub: fileCount === 1 ? 'one file' : 'live at HEAD',
    });

    // Tile 2: commits
    const commits = summaryByName.commits || summaryByName['number-of-commits'] || 0;
    tiles.push({
      label: 'Commits',
      defKey: 'commits',
      value: fmtInt(commits),
      sub: 'in the analysed history',
    });

    // Tile 3: authors
    const authors = summaryByName['authors'] || summaryByName['number-of-authors'] || 0;
    tiles.push({
      label: 'Distinct authors',
      defKey: 'authors',
      value: fmtInt(authors),
      sub: 'after mailmap resolution',
    });

    // Tile 4: median code health (proxy for codebase health)
    if (codeHealth.length) {
      const healths = codeHealth.map(function (r) { return r.score; })
        .filter(function (v) { return typeof v === 'number'; })
        .sort(function (a, b) { return a - b; });
      if (healths.length) {
        const mid = Math.floor(healths.length / 2);
        const median = (healths.length % 2) ? healths[mid] : (healths[mid - 1] + healths[mid]) / 2;
        const healthBand =
          median >= 90 ? 'healthy' :
          median >= 80 ? 'fair' :
          median >= 70 ? 'concern' : 'critical';
        tiles.push({
          label: 'Median code health',
          defKey: 'median_code_health',
          value: median.toFixed(1),
          sub: 'band: ' + healthBand,
        });
      }
    }

    // Tile 5: p95 cognitive (proxy for complexity tail)
    if (hotspots.length) {
      const cogs = hotspots.map(function (r) { return r.cognitive; })
        .filter(function (v) { return typeof v === 'number' && v > 0; })
        .sort(function (a, b) { return a - b; });
      if (cogs.length) {
        const idx = Math.min(cogs.length - 1, Math.floor(cogs.length * 0.95));
        tiles.push({
          label: 'Cognitive p95',
          defKey: 'cognitive_p95',
          value: fmtInt(cogs[idx]),
          sub: 'top-5% file complexity',
        });
      }
    }

    // Tile 6: knowledge-island count (only when >0 — CodeLore-only
    // signal, surface prominently when the repo has any)
    if (knowledgeIslands.length) {
      tiles.push({
        label: 'Knowledge islands',
        defKey: 'knowledge_islands',
        value: fmtInt(knowledgeIslands.length),
        sub: 'departed-author files',
      });
    }

    // Tile 7: coupling pair count + density
    if (coupling.length) {
      tiles.push({
        label: 'Coupled file pairs',
        defKey: 'coupling_pairs',
        value: fmtInt(coupling.length),
        sub: 'Fisher-significant',
      });
    }
    if (typeof d.coupling_density === 'number' && isFinite(d.coupling_density)) {
      tiles.push({
        label: 'Coupling density',
        defKey: 'coupling_density',
        value: d.coupling_density.toFixed(4),
        sub: 'graph |E|/(|V|·(|V|−1)/2)',
      });
    }

    // Tile 8: MI band breakdown
    if (d.mi_rollup && typeof d.mi_rollup === 'object') {
      const r = d.mi_rollup;
      const known = (r.low || 0) + (r.moderate || 0) + (r.high || 0);
      if (known > 0) {
        tiles.push({
          label: 'MI band breakdown',
          defKey: 'mi_band',
          value: '🟢 ' + fmtInt(r.high || 0) + ' / 🟡 ' + fmtInt(r.moderate || 0) + ' / 🔴 ' + fmtInt(r.low || 0),
          sub: 'top / mid / bottom quartile',
        });
      }
    }

    var html = '';
    for (var j = 0; j < tiles.length; j++) {
      const t = tiles[j];
      const tip = t.defKey ? buildTooltipHtml(t.defKey) : '';
      // DaisyUI `stat` classes layered onto the `.kpi-*` semantic
      // classes. The kpi-grid wrapper still drives layout; DaisyUI's
      // stat-title/value/desc give typography + spacing tokens
      // consistent with the rest of the dashboard chrome.
      html += '<div class="kpi-tile stat">' +
        '<div class="kpi-label stat-title">' + escapeHtml(t.label) + tip + '</div>' +
        '<div class="kpi-value stat-value">' + escapeHtml(t.value) + '</div>' +
        '<div class="kpi-sub stat-desc">' + escapeHtml(t.sub) + '</div>' +
        '</div>';
    }
    container.innerHTML = html;
  }


  // ─── §7  Widget: knowledge islands (CodeLore differentiator) ─────

  function renderKnowledgeIslands(rows) {
    const container = document.getElementById('widget-knowledge-islands-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No knowledge islands. ' +
        'Either no contributors have departed past the threshold or the ' +
        'analysis was not wired through.</div>';
      return;
    }

    // Sort by ownership pct descending then by days-since-active.
    const sorted = rows.slice().sort(function (a, b) {
      const oa = (typeof a.ownership_pct === 'number') ? a.ownership_pct : 0;
      const ob = (typeof b.ownership_pct === 'number') ? b.ownership_pct : 0;
      if (oa !== ob) return ob - oa;
      const da = (typeof a.days_since_main_active === 'number') ? a.days_since_main_active : 0;
      const db = (typeof b.days_since_main_active === 'number') ? b.days_since_main_active : 0;
      return db - da;
    });

    var html = '<table><thead><tr>' +
      '<th>Path</th>' +
      '<th>Departed author</th>' +
      '<th class="num">Ownership %</th>' +
      '<th class="num">Days since active</th>' +
      '<th class="num">LOC</th>' +
      '</tr></thead><tbody>';
    for (var i = 0; i < sorted.length; i++) {
      const r = sorted[i];
      // The knowledge_islands payload uses `entity` for the file
      // path (not `path` like the other tables). Read both so a
      // future rename in either direction doesn't silently empty
      // the Path column or break the click-to-drawer lookup.
      const filePath = r.path || r.entity || '';
      html += '<tr data-path="' + escapeHtml(filePath) + '" class="ki-row">' +
        '<td class="path">' + escapeHtml(filePath) + '</td>' +
        '<td>' + escapeHtml(r.main_author || '') + '</td>' +
        '<td class="num">' + fmtNumberFlex(r.ownership_pct, 1) + '</td>' +
        '<td class="num">' + fmtInt(r.days_since_main_active) + '</td>' +
        '<td class="num">' + fmtInt(r.total_loc) + '</td>' +
        '</tr>';
    }
    html += '</tbody></table>';
    container.innerHTML = html;

    const trs = container.querySelectorAll('tr.ki-row');
    for (var j = 0; j < trs.length; j++) {
      trs[j].addEventListener('click', function (evt) {
        const path = evt.currentTarget.getAttribute('data-path');
        // Route through `_codeloreShowDetail` so the click also
        // publishes to the selection store — same pattern every
        // other path-aware widget uses; bypassing it left the
        // trends + parallel-coords highlight stale on KI clicks.
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(path);
        } else {
          showFileDetailDrawer(path, data);
        }
      });
      trs[j].style.cursor = 'pointer';
      wireRowKbActivation(trs[j]);
    }
  }


  // ─── §8  Widget: hotspot circle-pack (signature CodeScene view) ──

  function renderHotspotCirclePack(rows, colorMode) {
    const container = document.getElementById('widget-hotspot-circle-pack-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No hotspots to display. ' +
        'The repository may be too small, or thresholds filtered everything out.</div>';
      return;
    }
    colorMode = colorMode || 'bivariate';
    // Clear any prior ECharts instance so toggles re-render cleanly.
    container.innerHTML = '';

    // Build a primary-author map (path → author with max added LoC)
    // for the W7 knowledge-map mode. Computed once per render call.
    const primaryAuthorByPath = computePrimaryAuthorByPath(data.entity_ownership || []);
    const authorPalette = makeAuthorPalette(
      Array.from(new Set(Object.values(primaryAuthorByPath)))
    );

    // Build a path → clone-group-count map for the 'clones'
    // colour mode. `data.clones` is the per-file overlay computed
    // by `output/spa.rs::run_clone_summary`; one entry per path
    // with ≥ 1 clone family. Falls back to an empty object when
    // the payload omits the field (older fixtures, no clones
    // detected). `maxCloneGroups` anchors the heatmap.
    const cloneCountByPath = {};
    let maxCloneGroups = 0;
    const cloneRows = data.clones || [];
    for (var ci = 0; ci < cloneRows.length; ci++) {
      const cr = cloneRows[ci];
      cloneCountByPath[cr.path] = cr.groups;
      if (cr.groups > maxCloneGroups) maxCloneGroups = cr.groups;
    }
    const cloneScale = maxCloneGroups || 1;

    // Build a path → composite code-health band map for the 'bivariate'
    // colour mode. `data.code_health` is the composite-score overlay from
    // the code-health analysis; each entry carries the pre-computed `band`
    // (green / yellow / red). Falls back to an empty object when the payload
    // omits the field (older fixtures, analysis not run). Mirrors the
    // cloneCountByPath pattern above.
    const bandByPath = {};
    (data.code_health || []).forEach(function (r) { bandByPath[r.path] = r.band; });

    // Step 1: build a filesystem-style hierarchy from flat HotspotRow[].
    // Each row is { path, revisions, cognitive, code_health, hotspot_score }.
    // Path "a/b/c.rs" yields tree:
    //   root -> "a" -> "b" -> "c.rs" (leaf with the metrics)
    const tree = buildFsHierarchy(rows);

    // Step 2: d3.hierarchy + d3.pack() compute circle (x, y, r) coords.
    // The pack layout sizes leaves by `revisions` (churn). Internal nodes
    // are sized by the sum of their leaves.
    const root = d3.hierarchy(tree)
      .sum(function (d) { return (d.metrics ? d.metrics.revisions : 0); })
      .sort(function (a, b) { return b.value - a.value; });

    const containerWidth = container.clientWidth || 800;
    const containerHeight = container.clientHeight || 600;
    const side = Math.min(containerWidth, containerHeight);
    d3.pack().size([side, side]).padding(2)(root);
    // d3.pack lays out into a square [0, side] × [0, side]; on a panel
    // wider (or taller) than the chosen `side`, the pack would sit in
    // the top-left of the canvas. Translate every node's (x, y) so
    // the square is centred — this offset needs to land BEFORE any
    // downstream consumer (renderItem closure, arc anchors,
    // `lastHotspotNodePositions`) reads the coords, otherwise the
    // arcs would draw at the un-offset positions while the circles
    // moved.
    const xOffset = Math.max(0, (containerWidth - side) / 2);
    const yOffset = Math.max(0, (containerHeight - side) / 2);
    if (xOffset > 0 || yOffset > 0) {
      root.each(function (n) {
        n.x += xOffset;
        n.y += yOffset;
      });
    }

    // Step 3: feed the laid-out nodes into ECharts as a custom series.
    // The custom series renders one shape per node; we draw circles
    // sized + positioned exactly per d3's layout. Color encodes
    // cognitive complexity (leaves only) on a yellow→red ramp.
    const chart = mountEcharts(container);
    // Wheel-zoom + drag-pan on the canvas. ECharts `type: 'custom'`
    // doesn't support `roam` natively, so we layer CSS-transform
    // pan/zoom on top — double-click resets. Same affordance the
    // Architecture force-graph gets from `series.roam: true`.
    attachCanvasZoom(container);
    // Register the reset handler so the top-right reset button
    // (installed at boot once all widgets have rendered) calls
    // back into the canvas-zoom reset.
    window._codeloreResetZoomHandlers['widget-hotspot-circle-pack'] = function () {
      if (container && typeof container._codeloreZoomReset === 'function') {
        container._codeloreZoomReset();
      }
    };
    const nodes = root.descendants();
    const maxCognitive = nodes.reduce(function (acc, n) {
      const cog = n.data.metrics ? n.data.metrics.cognitive : 0;
      return Math.max(acc, cog);
    }, 0) || 1;

    // P75 of the run's hotspot_score distribution. The ring overlay
    // marks any leaf at or above this as "in the top-quartile of
    // hotspot risk for this analysis." Uses the project's standard
    // percentile-rank approach (matches the MI bands); absolute
    // scores still appear in tooltips for cross-repo comparability.
    const hotspotScores = nodes
      .filter(function (n) { return n.data.metrics && n.data.metrics.hotspot_score != null; })
      .map(function (n) { return n.data.metrics.hotspot_score; })
      .sort(function (a, b) { return a - b; });
    const hotspotP75 = hotspotScores.length
      ? hotspotScores[Math.floor(hotspotScores.length * 0.75)]
      : Infinity;

    // Stash the laid-out node positions in module scope so
    // updateCouplingArcs() can do partial setOption updates on click
    // without re-running buildFsHierarchy + d3.pack.
    lastHotspotChart = chart;
    lastHotspotNodePositions = new Map();
    for (var ni = 0; ni < nodes.length; ni++) {
      const n = nodes[ni];
      if (n.data && n.data.fullPath) {
        lastHotspotNodePositions.set(n.data.fullPath, { x: n.x, y: n.y, r: n.r });
      }
    }

    // Declare the data arrays as `let` here so the inline `.map(...)`
    // expressions below (inside setOption) can assign back into them.
    // The renderItem callbacks close over these names and read each
    // item's render payload via `[params.dataIndex]._raw` / `._arc`.
    //
    // ECharts 6 dropped the older path for passing structured per-item
    // data into custom-series renderItem: string-keyed `api.value()`
    // returns NaN, and numeric `api.value(N)` coerces object values
    // through Number(...) so they also come back as NaN. The
    // closure-from-data-array pattern is the documented, stable
    // ECharts 6 way to pass per-item structured data through to the
    // renderItem callback. Sibling fields (name, fullPath, metrics,
    // depth, leafCount) stay on the data item so the tooltip formatter
    // — which DOES receive the full data item via `params.data` in
    // ECharts 6 — keeps working untouched.
    let circlePackData = [];
    // `arcData` lives at module scope (declared near the top of the
    // IIFE) so updateCouplingArcs() can mutate the same reference the
    // arc renderItem closes over. Reset its length here so the per-
    // render reset doesn't reassign and break the closure.
    arcData.length = 0;

    chart.setOption({
      // The whole canvas is the d3-laid-out coordinate space. We pass
      // raw pixel offsets so we don't need a grid/axis.
      tooltip: {
        trigger: 'item',
        formatter: function (params) {
          const d = params.data || {};
          if (d.depth === 0) return '<b>root</b>';
          if (!d.metrics) {
            return '<b>' + escapeHtml(d.name) + '</b>' +
              '<br/>directory · ' + d.leafCount + ' files';
          }
          const m = d.metrics;
          // When hovering the selected file, list its coupling partners inline
          // (basename + co-change %), so the coupled set is readable in one
          // place — not just inferable from the arcs on the map.
          let couplingLine = '';
          if (selectedCouplingFile && d.fullPath === selectedCouplingFile && arcData.length) {
            const partners = arcData
              .map(function (it) {
                const a = it._arc || {};
                const nm = (a.peer || '').split('/').pop();
                return escapeHtml(nm) + ' (' + Math.round(a.degree || 0) + '%)';
              })
              .join(', ');
            couplingLine =
              '<br/><span style="opacity:.75">coupled with: ' + partners + '</span>';
          }
          return '<b>' + escapeHtml(d.fullPath) + '</b>' +
            '<br/>revisions: ' + m.revisions +
            '<br/>cognitive: ' + m.cognitive.toFixed(0) +
            '<br/>code health: ' + m.code_health.toFixed(1) +
            '<br/>hotspot score: ' + m.hotspot_score.toFixed(2) +
            couplingLine;
        },
      },
      series: [{
        type: 'custom',
        coordinateSystem: 'none',
        // ECharts 6 dropped string-keyed `api.value()` lookups for
        // custom-series renderItem callbacks; only numeric dimension
        // indices into the data item's `value` array resolve. We carry
        // the per-leaf render payload at `value[2]` and read it via
        // `api.value(2)`. Sibling properties on the data item (name,
        // fullPath, metrics, depth, leafCount) remain readable via
        // `params.data` in the tooltip formatter, which is a separate
        // ECharts callback context where `params.data` is preserved.
        renderItem: function (params, api) {
          // Read the render payload via closure over `circlePackData`.
          // ECharts 6 coerces non-numeric values from api.value(N) to
          // NaN, so we cannot pack the payload into `value[N]`. The
          // closure-from-data-array pattern is the documented escape.
          const item = circlePackData[params.dataIndex];
          const datum = item ? item._raw : null;
          if (!datum) return null;
          // Directories (non-leaf nodes) carry `metrics: null` on the
          // data item. They render as the giant transparent containers
          // around the actual files, so they must NOT capture pointer
          // events — otherwise ECharts' first-match tooltip hit-test
          // always picks the outermost root node and the tooltip shows
          // "root" for every hover. `silent: true` lets pointer events
          // pass through to the leaf circles painted on top.
          const isDirectory = !item.metrics;
          // When a file is selected, outline it and its coupling partners in
          // info-blue so the coupled set is legible on the map (the selected
          // file a touch heavier than its partners). Otherwise the leaf keeps
          // its normal stroke.
          const coupled = datum.couplingSelected || datum.couplingPeer;
          const innerCircle = {
            type: 'circle',
            shape: {
              cx: datum.x,
              cy: datum.y,
              r: datum.r,
            },
            style: api.style({
              fill: datum.color,
              stroke: coupled ? token('--color-info') : datum.stroke,
              lineWidth: datum.couplingSelected ? 3 : (datum.couplingPeer ? 2 : 1),
              opacity: datum.opacity,
            }),
            silent: isDirectory,
          };
          // Top-quartile leaves get a yellow ring overlay. Drawn
          // first (lower in z-order) so the inner circle paints on
          // top — preserves the existing color encoding. Ring stroke
          // uses the cached `token()` so theme toggles see the new
          // --color-warning via registerThemeRerender's cache flush.
          if (datum.isHotspot) {
            return {
              type: 'group',
              silent: isDirectory,
              children: [
                {
                  type: 'circle',
                  shape: { cx: datum.x, cy: datum.y, r: datum.r + 2.5 },
                  style: {
                    fill: 'transparent',
                    stroke: token('--color-warning'),
                    lineWidth: 2,
                    opacity: 0.85,
                  },
                  silent: isDirectory,
                },
                innerCircle,
              ],
            };
          }
          return innerCircle;
        },
        zlevel: 1,
        data: (circlePackData = nodes
          // Render larger-first so smaller circles paint on top.
          .slice()
          .sort(function (a, b) { return b.r - a.r; })
          .map(function (n) {
            const isLeaf = !n.children || !n.children.length;
            const m = n.data.metrics;
            const cog = m ? m.cognitive : 0;
            const ratio = cog / maxCognitive;
            let leafColor;
            if (colorMode === 'author') {
              const author = primaryAuthorByPath[n.data.fullPath];
              leafColor = author ? authorPalette[author] : 'rgba(140, 140, 140, 0.55)';
            } else if (colorMode === 'ai') {
              // Per-file AI-attribution ratio: share of commits
              // touching this file that carry an ai-assisted /
              // ai-authored signal. Continuous heatmap from pale
              // (no AI) to red (all AI). Files with no MI/AI data
              // (binary, unsupported language) render as neutral
              // grey instead of misleading "0% AI".
              const aiPct = m && typeof m.ai_pct === 'number' ? m.ai_pct : null;
              if (aiPct === null) {
                leafColor = 'rgba(140, 140, 140, 0.55)';
              } else {
                leafColor = heatmapColor(Math.max(0, Math.min(1, aiPct / 100)));
              }
            } else if (colorMode === 'clones') {
              // Structural-duplication overlay. `cloneCountByPath`
              // came from `data.clones` (see `output/spa.rs::run_clone_summary`).
              // Files outside any clone family render neutral grey so
              // they sit visually behind the heat colours on actual
              // clone hotspots. The heatmap colour scales by the
              // max group count across the whole dashboard so the
              // distribution is per-repo relative, not absolute.
              const groups = cloneCountByPath[n.data.fullPath] || 0;
              if (groups === 0) {
                leafColor = 'rgba(140, 140, 140, 0.55)';
              } else {
                leafColor = heatmapColor(Math.min(1, groups / cloneScale));
              }
            } else if (colorMode === 'health') {
              // Code Health Map mode — 3-band green / yellow / red
              // via DaisyUI semantic tokens
              // (`--color-success` / `--color-warning` / `--color-error`)
              // so the bands auto-adapt to light and dark themes. Null
              // code_health (binary, unsupported language) renders as
              // the dim foreground rather than misleading green.
              leafColor = codeHealthColor(m ? m.code_health : null);
            } else if (colorMode === 'friction') {
              // Technical Debt Friction mode — continuous heat ramp
              // on hotspot_score. The formula
              // `percentile_rank(revisions) × percentile_rank(cognitive)
              // × (100 − code_health) / 4` already intersects activity
              // with unhealthy code (Tornhill 2018 score, range [0,10]),
              // so this is pure SQL → ramp surfacing. OKLCH interpolation
              // via heatRamp keeps the midpoint perceptually correct.
              if (!m || m.hotspot_score == null) {
                leafColor = token('--color-base-content');
              } else {
                leafColor = heatRamp(Math.max(0, Math.min(1, m.hotspot_score / 10)));
              }
            } else if (colorMode === 'knowledge-loss') {
              // Knowledge Loss Map + Off-boarding Sim — collapsed
              // into one mode. Blue = current team owns the
              // file; red = primary author is in the offboarding
              // scenario's `departed` set; dim = no author data. The
              // user-driven `departed` list comes from the dropdown in
              // template.html — toggling it fires the Alpine.effect
              // bridge which re-runs this render via the rerenderer
              // registry, with the token cache flushed first
              // (registerThemeRerender wraps it).
              const author = primaryAuthorByPath[n.data.fullPath];
              if (!author) {
                leafColor = token('--color-base-content');
              } else {
                const scenarioStore = (window.Alpine && window.Alpine.store)
                  ? window.Alpine.store('scenario')
                  : null;
                const isDeparted = scenarioStore
                  && scenarioStore.departed.indexOf(author) >= 0;
                leafColor = isDeparted
                  ? token('--color-error')
                  : token('--color-info');
              }
            } else if (colorMode === 'bivariate') {
              // Health × activity in one glyph: band (green/yellow/red) ×
              // hotspot activity (low/med/high). The danger quadrant
              // (red × high) is the darkest/most saturated cell — visible
              // without swapping lenses. Missing band → neutral grey.
              leafColor = bivariateColor(
                bandByPath[n.data.fullPath],
                m ? m.hotspot_score : null
              );
            } else {
              leafColor = heatmapColor(ratio);
            }
            const color = isLeaf
              ? leafColor
              : 'rgba(255, 255, 255, 0.02)';
            const stroke = isLeaf
              ? 'rgba(0, 0, 0, 0.3)'
              : 'rgba(255, 255, 255, 0.15)';
            // Ring overlay: tag leaves whose hotspot_score sits
            // in the top quartile of the run. renderItem reads
            // `_raw.isHotspot` and wraps the leaf in a yellow ring.
            const isHotspot = isLeaf && m && m.hotspot_score != null
              && m.hotspot_score >= hotspotP75;
            // `value[0]`, `value[1]` carry the d3-laid-out (x, y) for
            // ECharts' coordinate system. `_raw` is the render payload
            // renderItem reads via closure over `circlePackData`
            // (see explanation above the `let circlePackData = []`).
            // Sibling fields (name, fullPath, metrics, …) drive the
            // tooltip formatter via `params.data`.
            return {
              value: [n.x, n.y],
              _raw: {
                x: n.x, y: n.y, r: n.r,
                color: color, stroke: stroke,
                opacity: isLeaf ? 0.85 : 1,
                isHotspot: isHotspot,
              },
              name: n.data.name || 'root',
              fullPath: n.data.fullPath || '',
              metrics: m || null,
              depth: n.depth,
              leafCount: n.leaves ? n.leaves().length : 0,
            };
          })),
      }, {
        // Second custom series for the coupling arc overlay.
        // Drives off the shared coordinateSystem ('none' = raw pixel
        // coords from d3.pack), so arcs anchor exactly on the circle
        // centres. zlevel: 2 paints above the circle pack. `silent:
        // true` keeps clicks falling through to the leaves below.
        // Initial data computed from the current `selectedCouplingFile`
        // (null on first render → empty array → invisible series).
        type: 'custom',
        coordinateSystem: 'none',
        zlevel: 2,
        silent: true,
        // Read the arc payload via closure over `arcData` — same
        // ECharts 6 pattern as the inner circle-pack series above.
        renderItem: function (params, api) {
          const item = arcData[params.dataIndex];
          const arc = item ? item._arc : null;
          if (!arc) return null;
          return {
            type: 'path',
            shape: { d: arcPath(arc.x1, arc.y1, arc.x2, arc.y2, 0.25) },
            style: {
              stroke: token('--color-warning'),
              fill: 'none',
              opacity: arc.opacity,
              lineWidth: arc.lineWidth,
            },
            silent: true,
          };
        },
        // `value[0]`, `value[1]` anchor the arc on its first endpoint;
        // `_arc` carries the full payload that renderItem reads via
        // closure over `arcData` (module scope, never reassigned —
        // mutated in place so the closure stays live across calls
        // from updateCouplingArcs).
        data: (function () {
          const arcs = buildCouplingArcs(
            selectedCouplingFile,
            lastHotspotNodePositions,
            data.coupling || []
          );
          for (var ai = 0; ai < arcs.length; ai++) {
            const a = arcs[ai];
            arcData.push({ value: [a.x1, a.y1], _arc: a });
          }
          return arcData;
        })(),
      }],
    });

    // Stash the built render payload in module scope so updateHotspotBrush()
    // can re-tint opacities via a partial setOption, and re-apply an active
    // brush after a full re-render (theme toggle etc.).
    lastCirclePackData = circlePackData;
    if (brushedPaths) updateHotspotBrush();

    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.fullPath && d.metrics) {
        // Clicking a leaf surfaces its coupling partners AND opens the
        // drawer. Route through _codeloreShowDetail so the click also
        // broadcasts the selection — the 'hotspot-map' listener then sets
        // selectedCouplingFile + redraws the arcs, so we must NOT do that
        // here too (double redraw). The direct arc update stays only on the
        // no-broadcast fallback path.
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(d.fullPath);
        } else {
          selectedCouplingFile = d.fullPath;
          updateCouplingArcs();
          showFileDetailDrawer(d.fullPath, data);
        }
      }
    });

    // Clicking the canvas background (no shape under the pointer) clears the
    // selection. `e.target` is falsy for background clicks in zrender's event
    // model. The map now PUBLISHES on leaf click, so a background click must
    // clear the shared focus across every widget — not just the local arcs —
    // to stay symmetric. Broadcasting a clear fans out to the 'hotspot-map'
    // listener, which nulls selectedCouplingFile + redraws. Fallback (Alpine
    // absent): clear the arcs directly.
    chart.getZr().on('click', function (e) {
      if (!e.target) {
        const sel =
          window.Alpine && window.Alpine.store && window.Alpine.store('selection');
        if (sel) {
          sel.clear();
        } else {
          selectedCouplingFile = null;
          updateCouplingArcs();
        }
      }
    });

    // Cross-widget selection: when a file is selected in ANY widget, light up
    // its coupling arcs on the map — the same overlay a direct leaf-click
    // shows. Reuses the existing selectedCouplingFile + updateCouplingArcs
    // machinery, so the map participates in the shared focus without a
    // second highlight mechanism. A null selection clears the arcs.
    window._codeloreRegisterSelectionListener('hotspot-map', function (selectedPath) {
      selectedCouplingFile = selectedPath || null;
      updateCouplingArcs();
    });

    // Bivariate quadrant brush: emphasise the set / dim the rest by
    // recomputing per-leaf opacity. Registered here (mirrors the selection
    // listener) so it closes over the fresh render; re-fires via the brush
    // store's Alpine.effect fan-out.
    window._codeloreRegisterBrushListener('hotspot-map', function (cell, paths) {
      brushedPaths = (paths && paths.length) ? new Set(paths) : null;
      updateHotspotBrush();
    });

    renderBivariateLegend();
  }

  // 3×3 bivariate legend: a small grid keyed to BIVARIATE_PALETTE, axes
  // labeled health (green→red, top→bottom) × activity (low→high, left→right).
  // Populates the legend mount whenever the circle-pack renders; a no-op if the
  // mount is absent. Only visible while the bivariate mode is active — in the
  // other colour modes the legend would describe an encoding not on screen, so
  // it hides itself. The palette is a fixed CVD-tuned hex set (not DaisyUI theme
  // tokens) on purpose: the health×activity blend must stay deterministic and
  // lightness-monotonic regardless of theme.
  function renderBivariateLegend() {
    const mount = document.getElementById('bivariate-legend');
    if (!mount) return;
    mount.style.display = (currentHotspotColorMode === 'bivariate') ? '' : 'none';
    const cells = BIVARIATE_PALETTE.map(function (c, i) {
      const hb = Math.floor(i / 3);
      const ab = i % 3;
      return '<div data-biv-cell data-hb="' + hb + '" data-ab="' + ab + '" '
        + 'style="width:14px;height:14px;background:' + c + ';cursor:pointer;outline-offset:1px" '
        + 'title="health ' + (['healthy', 'warning', 'unhealthy'][hb])
        + ' × activity ' + (['low', 'med', 'high'][ab])
        + ' — click to brush this quadrant"></div>';
    }).join('');
    mount.innerHTML =
      '<div class="text-xs opacity-70 mb-1">Health × Activity</div>' +
      '<div style="display:grid;grid-template-columns:repeat(3,14px);gap:2px">' + cells + '</div>' +
      '<div class="text-xs opacity-50 mt-1">↓ less healthy&nbsp;&nbsp;→ more active</div>';

    // Legend cell → quadrant set-brush. Band from data.code_health (same
    // source as the circle-pack's bandByPath); activity from data.hotspots.
    // Clicking the active cell again clears.
    const bandByPath = {};
    (data.code_health || []).forEach(function (r) { bandByPath[r.path] = r.band; });
    const cellEls = mount.querySelectorAll('[data-biv-cell]');
    for (var i = 0; i < cellEls.length; i++) {
      cellEls[i].addEventListener('click', function (evt) {
        const store = window.Alpine && window.Alpine.store && window.Alpine.store('brush');
        if (!store) return;
        const hb = Number(evt.currentTarget.getAttribute('data-hb'));
        const ab = Number(evt.currentTarget.getAttribute('data-ab'));
        if (store.isActive(hb, ab)) { store.clear(); return; }
        const paths = (data.hotspots || []).filter(function (h) {
          return healthBucket(bandByPath[h.path]) === hb
            && activityBucket(h.hotspot_score) === ab;
        }).map(function (h) { return h.path; });
        store.set([hb, ab], paths);
      });
      wireRowKbActivation(cellEls[i]); // role=button + tabindex + Enter/Space → click
    }

    // Legend is itself a brush subscriber: outline the active quadrant cell.
    if (window._codeloreRegisterBrushListener) {
      window._codeloreRegisterBrushListener('bivariate-legend', function (cell) {
        const m = document.getElementById('bivariate-legend');
        if (!m) return;
        const cs = m.querySelectorAll('[data-biv-cell]');
        for (var k = 0; k < cs.length; k++) {
          const on = !!cell
            && Number(cs[k].getAttribute('data-hb')) === cell[0]
            && Number(cs[k].getAttribute('data-ab')) === cell[1];
          cs[k].style.outline = on ? '2px solid var(--color-base-content)' : '';
        }
      });
    }
  }


  // ─── §9  Widget: hotspot table (sortable drill-down of §8) ────────

  function renderHotspotTable(rows) {
    const container = document.getElementById('widget-hotspot-table-body');
    const filterEl = document.getElementById('hotspot-table-filter');
    const summaryEl = document.getElementById('hotspot-table-summary');
    const actionsEl = document.getElementById('hotspot-table-actions');
    if (!container || !filterEl || !summaryEl || !actionsEl) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No hotspot rows.</div>';
      summaryEl.textContent = '';
      return;
    }

    const COLUMNS = [
      { key: 'path',          label: 'Path',         cls: 'path', kind: 'string', defaultDir: 1 },
      { key: 'revisions',     label: 'Revisions',    cls: 'num',  kind: 'number', defaultDir: -1, defKey: 'revisions' },
      { key: 'cognitive',     label: 'Cognitive',    cls: 'num',  kind: 'number', defaultDir: -1, defKey: 'cognitive' },
      { key: 'code_health',   label: 'Code Health',  cls: 'num',  kind: 'number', defaultDir: 1,  defKey: 'code_health' },
      { key: 'hotspot_score', label: 'Hotspot Score', cls: 'num', kind: 'number', defaultDir: -1, defKey: 'hotspot_score' },
      { key: 'mi',            label: 'MI',           cls: 'num',  kind: 'number', defaultDir: -1, defKey: 'mi' },
      { key: 'ai_pct',        label: 'AI %',         cls: 'num',  kind: 'number', defaultDir: -1, defKey: 'ai_pct' },
    ];
    const PAGE_SIZE = 500;

    // State.
    let sortKey = 'hotspot_score';
    let sortDir = -1;       // 1 = ascending, -1 = descending
    let filterText = '';
    let renderedRows = 0;   // how many of the filtered set we've appended
    let filteredView = [];  // current sorted+filtered slice

    function compare(a, b) {
      const va = a[sortKey];
      const vb = b[sortKey];
      const col = COLUMNS.find(function (c) { return c.key === sortKey; });
      if (col && col.kind === 'string') {
        return sortDir * String(va).localeCompare(String(vb));
      }
      // numeric — treat undefined as -Infinity so it sinks under desc sort
      const na = (typeof va === 'number') ? va : -Infinity;
      const nb = (typeof vb === 'number') ? vb : -Infinity;
      return sortDir * (na - nb);
    }

    function applyFilter(query) {
      const q = query.trim().toLowerCase();
      filteredView = q
        ? rows.filter(function (r) { return r.path.toLowerCase().indexOf(q) !== -1; })
        : rows.slice();
      filteredView.sort(compare);
    }

    function fmtNumber(v, opts) {
      if (typeof v !== 'number' || !isFinite(v)) return '';
      const decimals = (opts && opts.decimals != null) ? opts.decimals : 2;
      return v.toFixed(decimals);
    }

    function renderHeader() {
      // DaisyUI `table table-zebra` provides striped rows + consistent
      // typography on top of the inline `.table-container table { ... }`
      // rules. The two co-exist: inline rules win on background-color
      // (var(--bg-elev-2)) for stylistic continuity; DaisyUI's font
      // tokens layer on top.
      let html = '<table class="table table-zebra"><thead><tr>';
      for (var i = 0; i < COLUMNS.length; i++) {
        const c = COLUMNS[i];
        const active = (c.key === sortKey);
        const indicator = active
          ? (sortDir > 0 ? '▲' : '▼')
          : '';
        const tip = c.defKey ? buildTooltipHtml(c.defKey) : '';
        html += '<th class="' + (active ? 'active' : '') + '"' +
          ' data-key="' + escapeHtml(c.key) + '">' +
          escapeHtml(c.label) + tip +
          ' <span class="sort-indicator">' + indicator + '</span>' +
          '</th>';
      }
      html += '</tr></thead><tbody id="hotspot-tbody"></tbody></table>';
      container.innerHTML = html;

      // Wire header click → sort.
      const ths = container.querySelectorAll('th');
      for (var j = 0; j < ths.length; j++) {
        ths[j].addEventListener('click', function (evt) {
          // A click (or keyboard activation) on the metric-help "?" button,
          // which lives inside the <th>, must not also sort the column.
          if (evt.target.closest && evt.target.closest('.tooltip-trigger')) {
            return;
          }
          const key = evt.currentTarget.getAttribute('data-key');
          if (sortKey === key) {
            sortDir *= -1;
          } else {
            sortKey = key;
            const col = COLUMNS.find(function (c) { return c.key === key; });
            sortDir = col ? col.defaultDir : -1;
          }
          rerender();
        });
      }
    }

    async function renderNextPage(count) {
      const tbody = container.querySelector('#hotspot-tbody');
      if (!tbody) return;
      // Filter matched nothing: show an inline message instead of a blank
      // body (which reads as "the table broke"). The whole-dataset-empty
      // case is handled by the earlier `No hotspot rows.` return, so an
      // empty view here always means an active filter with no matches.
      if (filteredView.length === 0) {
        tbody.innerHTML = '<tr><td class="empty" colspan="99">No paths match “' +
          escapeHtml(filterText) + '”.</td></tr>';
        renderedRows = 0;
        refreshActions();
        return;
      }
      // Chunk the rebuild — `Show all` historically called this with
      // `Infinity` and blocked the main thread for hundreds of ms on
      // large repos (one HTML string built, one insertAdjacentHTML
      // call, one querySelectorAll over the full table for click
      // wiring). Walk in CHUNK_SIZE batches and `await yieldToMain()`
      // between each so user input (drawer open, tab switch,
      // scrolling) stays responsive during the expansion. F134 root-
      // cause fix.
      //
      // Small expansions render synchronously — the per-yield cost
      // (~0.5-2 ms message-channel round-trip + an extra paint cycle)
      // exceeds the gain when only a few chunks would run. The chunked
      // path is for the genuine "Show all on a 5000-row table" case.
      const CHUNK_SIZE = 50;
      const SYNC_THRESHOLD = 200;
      const totalEnd = Math.min(renderedRows + count, filteredView.length);
      const remaining = totalEnd - renderedRows;
      if (remaining <= SYNC_THRESHOLD) {
        await renderPageChunk(tbody, totalEnd);
        refreshActions();
        return;
      }
      while (renderedRows < totalEnd) {
        const next = Math.min(renderedRows + CHUNK_SIZE, totalEnd);
        await renderPageChunk(tbody, next);
        if (renderedRows < totalEnd) {
          // Only yield between chunks, not after the final one — the
          // caller's continuation (refreshActions) can run inline.
          await yieldToMain();
        }
      }
      refreshActions();
    }

    function renderPageChunk(tbody, next) {
      var html = '';
      for (var i = renderedRows; i < next; i++) {
        const r = filteredView[i];
        // MI cell: number + DaisyUI band badge (success / warning /
        // error) when mi_rank is finite. Empty when language is
        // unsupported by codelore-rca. The colour-coded pill carries
        // the top/mid/bottom-quartile triad accessibly for screen
        // readers and themably through DaisyUI's `--color-success` /
        // `--color-warning` / `--color-error` tokens.
        let miCell = '';
        if (typeof r.mi === 'number' && isFinite(r.mi)) {
          let bandBadge = '';
          if (typeof r.mi_rank === 'number' && isFinite(r.mi_rank)) {
            // Complete class-name literals (not string-concatenation)
            // so the Tailwind v4 pruner can see each variant during
            // `@source` scan of widgets.js — `'badge-' + kind` would
            // hide the suffix from the static scan and the variants
            // would drop out of the compiled CSS bundle.
            if (r.mi_rank >= 0.75) {
              bandBadge = ' <span class="badge badge-success badge-sm" title="MI band: High">High</span>';
            } else if (r.mi_rank >= 0.25) {
              bandBadge = ' <span class="badge badge-warning badge-sm" title="MI band: Mid">Mid</span>';
            } else {
              bandBadge = ' <span class="badge badge-error badge-sm" title="MI band: Low">Low</span>';
            }
          }
          miCell = r.mi.toFixed(1) + bandBadge;
        }
        // AI cell: percentage rendered as X% (rounded — table is dense,
        // decimal point would crowd). Wrapped in a DaisyUI outline
        // badge so the AI-attribution signal reads consistently with
        // the MI band badge above.
        const aiCell = (typeof r.ai_pct === 'number' && isFinite(r.ai_pct))
          ? '<span class="badge badge-outline badge-sm">' + Math.round(r.ai_pct) + '%</span>'
          : '';
        // `data-primary-author` lets the off-boarding effect (set up
        // below) toggle a `.hotspot-row-departed` class on rows whose
        // primary author is in `$store.scenario.departed` — the same
        // reactive signal the keyboard-accessible file list uses.
        // Lookup pulls from `_codelorePrimaryAuthorByPath`, populated
        // once at boot.
        const rowAuthor = (window._codelorePrimaryAuthorByPath || {})[r.path] || '';
        html += '<tr data-path="' + escapeHtml(r.path) + '" data-primary-author="' + escapeHtml(rowAuthor) + '" class="hotspot-row" style="cursor:pointer">' +
          '<td class="path">' + escapeHtml(r.path) + '</td>' +
          '<td class="num">' + (r.revisions != null ? r.revisions : '') + '</td>' +
          '<td class="num">' + fmtNumber(r.cognitive, { decimals: 0 }) + '</td>' +
          '<td class="num">' + fmtNumber(r.code_health, { decimals: 1 }) + '</td>' +
          '<td class="num">' + fmtNumber(r.hotspot_score, { decimals: 2 }) + '</td>' +
          '<td class="num">' + miCell + '</td>' +
          '<td class="num">' + aiCell + '</td>' +
          '</tr>';
      }
      tbody.insertAdjacentHTML('beforeend', html);
      renderedRows = next;
      // Wire row click → detail drawer for the rows we just added.
      const newRows = tbody.querySelectorAll('tr.hotspot-row:not([data-wired])');
      for (var k = 0; k < newRows.length; k++) {
        newRows[k].setAttribute('data-wired', '1');
        newRows[k].addEventListener('click', function (evt) {
          const path = evt.currentTarget.getAttribute('data-path');
          if (window._codeloreShowDetail) window._codeloreShowDetail(path);
        });
        wireRowKbActivation(newRows[k]);
      }
      return Promise.resolve();
    }

    function refreshActions() {
      summaryEl.textContent = filteredView.length === rows.length
        ? (renderedRows + ' of ' + rows.length + ' rows shown')
        : (renderedRows + ' of ' + filteredView.length + ' filtered rows shown (' +
           rows.length + ' total)');
      const more = filteredView.length - renderedRows;
      actionsEl.innerHTML = '';
      if (more <= 0) return;
      const next = Math.min(PAGE_SIZE, more);
      const showNext = document.createElement('button');
      showNext.type = 'button';
      // DaisyUI `btn btn-outline btn-sm` matches the dashboard's
      // button vocabulary (theme toggle, drawer close). Inline
      // `.table-actions button { ... }` rules in the `<style>` block
      // are no-op'd by this — the DaisyUI utility classes win
      // specificity now that we declare them explicitly.
      showNext.className = 'btn btn-outline btn-sm';
      showNext.textContent = 'Show next ' + next;
      // Element-scoped transition on `container` — the rest of the
      // dashboard stays interactive while the table animates.
      showNext.addEventListener('click', function () {
        startViewTransition(function () { renderNextPage(PAGE_SIZE); }, container);
      });
      actionsEl.appendChild(showNext);
      if (more > PAGE_SIZE) {
        const showAll = document.createElement('button');
        showAll.type = 'button';
        showAll.className = 'btn btn-outline btn-sm';
        showAll.textContent = 'Show all (' + more + ' more)';
        showAll.addEventListener('click', function () {
          startViewTransition(function () { renderNextPage(Infinity); }, container);
        });
        actionsEl.appendChild(showAll);
      }
    }

    function rerender() {
      renderHeader();
      applyFilter(filterText);
      renderedRows = 0;
      renderNextPage(PAGE_SIZE);
    }

    // Debounce the filter input — applying the filter requires a full
    // table rebuild, which is a few ms on 30k rows. 80 ms feels live.
    var debounceTimer = null;
    filterEl.addEventListener('input', function (evt) {
      filterText = evt.target.value;
      // Mirror the local filterText into the Alpine `filter` store so
      // any other widget that subscribes via `Alpine.effect(...)` sees
      // the live value. Wrapped in `window.Alpine` guard so the page
      // still works if Alpine fails to load (e.g. user disabled JS
      // through a content-security-policy header).
      if (window.Alpine) {
        window.Alpine.store('filter').set(filterText);
      }
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(rerender, 80);
    });

    // Seed the input from the persisted Alpine store on first render
    // so a page reload (e.g. `--embed` step-summary roundtrip) brings
    // the user's last filter back. The store value comes from
    // localStorage via the Alpine persist plugin.
    if (window.Alpine) {
      const persisted = window.Alpine.store('filter').text;
      if (persisted && !filterEl.value) {
        filterEl.value = persisted;
        filterText = persisted;
      }
    }

    // Initial render.
    rerender();

    // Cross-widget selection: highlight the row for the selected path (if
    // it's on the current page); a null selection clears all row highlights.
    // Rows are rebuilt on sort/filter/paginate, so query the live DOM each
    // time rather than caching nodes.
    window._codeloreRegisterSelectionListener('hotspot-table', function (selectedPath) {
      const tbody = document.getElementById('hotspot-tbody');
      if (!tbody) return;
      const rows = tbody.querySelectorAll('tr');
      for (var i = 0; i < rows.length; i++) {
        const rowPath = rows[i].getAttribute('data-path');
        const isSel = !!selectedPath && rowPath === selectedPath;
        rows[i].classList.toggle('!bg-base-300', isSel);
        // Mark the selected row for assistive tech, not just visually.
        // aria-current is removed (not set to 'false') on non-selected rows
        // so only one row ever carries the state.
        if (isSel) {
          rows[i].setAttribute('aria-current', 'true');
        } else {
          rows[i].removeAttribute('aria-current');
        }
      }
    });

    // Cross-widget quadrant brush: emphasise every row whose path is in the
    // brushed set (distinct from the single-selection `!bg-base-300` — brush
    // = context, selection = focus; a row can carry both). Rebuilt on
    // sort/filter/paginate like the selection highlight, so it re-applies on
    // the next brush change (same transient-drop behaviour as selection).
    window._codeloreRegisterBrushListener('hotspot-table', function (cell, paths) {
      const tbody = document.getElementById('hotspot-tbody');
      if (!tbody) return;
      const set = new Set(paths || []);
      const rows = tbody.querySelectorAll('tr');
      for (var i = 0; i < rows.length; i++) {
        const p = rows[i].getAttribute('data-path');
        rows[i].classList.toggle('hotspot-row-brushed', !!p && set.has(p));
      }
    });
  }


  // ─── §10 Widget: change-coupling sankey ──────────────────────────

  function renderCouplingSankey(rows) {
    const container = document.getElementById('widget-coupling-sankey-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No coupling rows. Either the ' +
        'repo has too few co-changes to be Fisher-significant or the ' +
        'analysis was not wired through.</div>';
      return;
    }

    // Depth source:
    //   'files' (default) → no aggregation: top-30 file pairs
    //   integer 2-6      → collapse entities to N path segments,
    //                       re-aggregate pairs, then top-30
    // The user setting lives in `Alpine.store('layout').sankeyDepth`
    // and is persisted across reloads.
    const TOP_N = 30;
    const sankeyLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userSankeyDepth = sankeyLayout ? sankeyLayout.sankeyDepth : 'files';

    function modulePathSeg(p, depth) {
      const parts = (p || '').split('/');
      if (parts.length <= depth) {
        const lastSlash = (p || '').lastIndexOf('/');
        return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
      }
      return parts.slice(0, depth).join('/');
    }

    var workingRows;
    if (typeof userSankeyDepth === 'number') {
      // Aggregate file-pair coupling to module-pair coupling at the
      // chosen depth. Self-pairs (s === t after collapse) and
      // duplicate (s, t) edges are merged by summing the shared-revision
      // count (`shared`) and taking the max coupling strength (`degree`,
      // the co-change percentage). These are CouplingRow's actual fields.
      const aggregated = {};
      for (var i = 0; i < rows.length; i++) {
        const r = rows[i];
        const a = modulePathSeg(r.entity_a, userSankeyDepth);
        const b = modulePathSeg(r.entity_b, userSankeyDepth);
        if (!a || !b || a === b) continue;
        const key = a < b ? a + '\x00' + b : b + '\x00' + a;
        if (!aggregated[key]) {
          aggregated[key] = {
            entity_a: a < b ? a : b,
            entity_b: a < b ? b : a,
            shared: 0,
            degree: 0,
          };
        }
        aggregated[key].shared += (r.shared || 0);
        const strength = (typeof r.degree === 'number') ? r.degree : 0;
        if (strength > aggregated[key].degree) {
          aggregated[key].degree = strength;
        }
      }
      workingRows = Object.keys(aggregated).map(function (k) { return aggregated[k]; });
    } else {
      workingRows = rows;
    }

    const topRows = workingRows.slice()
      .sort(function (a, b) {
        const ca = (typeof a.degree === 'number') ? a.degree : 0;
        const cb = (typeof b.degree === 'number') ? b.degree : 0;
        return cb - ca;
      })
      .slice(0, TOP_N);

    // Build the node + link arrays. ECharts sankey needs unique node
    // names and links {source, target, value}.
    const nodeNames = new Set();
    const links = topRows.map(function (r) {
      nodeNames.add(r.entity_a);
      nodeNames.add(r.entity_b);
      return {
        source: r.entity_a,
        target: r.entity_b,
        value: r.shared || 0,
      };
    });
    const nodes = Array.from(nodeNames).map(function (name) {
      return { name: name };
    });

    setChartAriaLabel(container,
      'Change-coupling Sankey flow across ' + nodes.length + ' entities, ' +
      links.length + ' co-change links');

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        trigger: 'item',
        formatter: function (params) {
          if (params.dataType === 'edge') {
            return '<b>' + escapeHtml(params.data.source) + ' ↔ ' +
              escapeHtml(params.data.target) + '</b>' +
              '<br/>shared revs: ' + params.data.value;
          }
          return '<b>' + escapeHtml(params.data.name) + '</b>';
        },
      },
      series: [{
        type: 'sankey',
        layout: 'none',
        nodeAlign: 'left',
        emphasis: { focus: 'adjacency' },
        data: nodes,
        links: links,
        lineStyle: { color: 'gradient', curveness: 0.5 },
        label: { color: token('--label-on-dark'), fontSize: 11 },
      }],
    });

    chart.on('click', function (params) {
      if (params.dataType === 'node' && params.data && params.data.name) {
        // In 'files' mode the node name IS a full repo-relative path, so
        // broadcast the selection. In module-depth mode the name is a
        // truncated module prefix that no file-level subscriber matches —
        // open the drawer directly without polluting the selection bus.
        if (userSankeyDepth === 'files' && window._codeloreShowDetail) {
          window._codeloreShowDetail(params.data.name);
        } else {
          showFileDetailDrawer(params.data.name, data);
        }
      }
    });

    // Cross-widget selection: emphasise the selected file's node (and its
    // links) in the coupling sankey; a null selection downplays everything.
    window._codeloreRegisterSelectionListener('coupling', function (selectedPath) {
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      // Node names live in the current depth's name-space: full paths in
      // 'files' mode, truncated module prefixes in module-depth mode. Map
      // the bus's full path into that space or the module view no-ops.
      const nodeName = (typeof userSankeyDepth === 'number')
        ? modulePathSeg(selectedPath, userSankeyDepth)
        : selectedPath;
      chart.dispatchAction({ type: 'highlight', seriesIndex: 0, name: nodeName });
    });

  }


  // ─── §11 Widget: trends multi-line ───────────────────────────────

  function renderTrends(rows) {
    const container = document.getElementById('widget-trends-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No trend data — repo too small or analyses not wired.</div>';
      return;
    }

    // User-tunable Top-N. Backend sends up to 50 paths; the frontend
    // ranks by total revisions across all months and slices to
    // `Alpine.store('layout').trendsTopN` (default 10, 'all' = no
    // cap). The selector persists across reloads.
    const trendsLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const trendsTopN = trendsLayout ? trendsLayout.trendsTopN : 10;

    // Build {month -> {path -> score}} and a sorted month list.
    const months = Array.from(new Set(rows.map(function (r) { return r.month; }))).sort();
    const byMonth = {};
    const pathTotals = {};
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      if (!byMonth[r.month]) byMonth[r.month] = {};
      byMonth[r.month][r.path] = r.hotspot_score;
      pathTotals[r.path] = (pathTotals[r.path] || 0) + (r.hotspot_score || 0);
    }
    const allPaths = Object.keys(pathTotals)
      .sort(function (a, b) { return pathTotals[b] - pathTotals[a]; });
    const paths = (trendsTopN === 'all')
      ? allPaths
      : allPaths.slice(0, Number(trendsTopN));
    // One series per path. `emphasis.focus: 'series'` + explicit
    // `blur` dim non-hovered lines so the user can isolate one
    // trajectory in a busy chart — without `blur`, ECharts 6 leaves
    // the rest at full opacity and the hovered line gets visually
    // lost.
    const series = paths.map(function (p) {
      return {
        name: p,
        type: 'line',
        smooth: true,
        symbol: 'circle',
        symbolSize: 5,
        emphasis: { focus: 'series', lineStyle: { width: 3 } },
        blur: { lineStyle: { opacity: 0.15 } },
        data: months.map(function (m) {
          return (byMonth[m] && byMonth[m][p]) || 0;
        }),
      };
    });

    // Abbreviate long paths in the legend so the scroll pager has
    // room for multiple labels per page. Keeps the top segment for
    // architectural context and the last two for file identification:
    // `app/services/clients/application/service.py` → `app/…/application/service.py`.
    // Full path is preserved in the tooltip via the formatter.
    function shortPath(p) {
      const parts = (p || '').split('/');
      if (parts.length <= 3) return p;
      return parts[0] + '/…/' + parts.slice(-2).join('/');
    }
    // Disambiguate collisions: two paths that share head + tail segments
    // (e.g. `app/a/svc/main.py` and `app/b/svc/main.py`) abbreviate to the
    // same label. ECharts merges same-named series + legend entries, so one
    // legend toggle would flip several files and the tooltip's full-path
    // line would show the wrong (last-colliding) path. On collision, fall
    // back to the full path, which is always unique.
    const shortByLong = {};
    const longByShort = {};
    paths.forEach(function (p) {
      let label = shortPath(p);
      if (longByShort[label] !== undefined && longByShort[label] !== p) {
        label = p;
      }
      shortByLong[p] = label;
      longByShort[label] = p;
    });
    const legendData = paths.map(function (p) { return shortByLong[p]; });

    setChartAriaLabel(container,
      'Hotspot-score trend for ' + paths.length + ' files over ' +
      months.length + ' months');

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        trigger: 'axis',
        formatter: function (params) {
          if (!params || !params.length) return '';
          var html = '<b>' + escapeHtml(params[0].axisValueLabel || params[0].name) + '</b>';
          for (var i = 0; i < params.length; i++) {
            const p = params[i];
            const full = longByShort[p.seriesName] || p.seriesName;
            html += '<br/>' + p.marker + escapeHtml(full) + ': <b>' + p.value + '</b>';
          }
          return html;
        },
      },
      // Vertical right-side legend. A horizontal top legend
      // collides with the y-axis name and overlaps itself once
      // there are more than ~6 entries with long paths — even
      // with the path-abbreviation. Vertical scrolling on the
      // right scales cleanly from 5 → 50 series without
      // overlapping the chart area or the axis labels.
      //
      // `selector` adds ECharts' built-in "All" / "Inv" buttons
      // at the bottom of the legend — one click clears the entire
      // selection, another click inverts. Saves the user from
      // having to click every file off individually just to
      // isolate one trajectory.
      legend: {
        type: 'scroll',
        orient: 'vertical',
        right: 8,
        top: 8,
        bottom: 30,
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        pageTextStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        data: legendData,
        itemGap: 6,
        pageButtonGap: 4,
        width: 220,
        // `title` is the button label — ECharts ships `inverse` as
        // "Inv" by default, which reads cryptic. "Swap" tells the
        // user exactly what happens: the on/off state of every
        // file flips. From the default all-on view, one click of
        // Swap turns everything off, then they click the single
        // file they want to keep.
        selector: [
          { type: 'all',     title: 'All' },
          { type: 'inverse', title: 'Swap' },
        ],
        selectorPosition: 'end',
        selectorButtonGap: 4,
        selectorLabel: {
          color: getCssVar('--fg-dim'),
          fontSize: 10,
          padding: [2, 6],
          borderColor: getCssVar('--border'),
          borderWidth: 1,
          borderRadius: 4,
        },
      },
      grid: { top: 16, left: 70, right: 248, bottom: 40 },
      xAxis: {
        type: 'category',
        data: months,
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        axisLine: { lineStyle: { color: getCssVar('--border') } },
      },
      yAxis: {
        type: 'value',
        // `nameLocation: 'middle'` + `nameRotate: 90` puts the
        // axis name along the axis instead of above it, which
        // would collide with the vertical legend on the right —
        // the original top-positioned "revisions / month" sat
        // right where the legend's first row now wants to be.
        name: 'revisions / month',
        nameLocation: 'middle',
        nameRotate: 90,
        nameGap: 40,
        nameTextStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        splitLine: { lineStyle: { color: getCssVar('--bg-elev-2') } },
      },
      series: series.map(function (s) {
        return Object.assign({}, s, { name: shortByLong[s.name] || shortPath(s.name) });
      }),
    });
    // Cross-widget selection sync: each series corresponds to one
    // path (the path array's indexing matches the series array),
    // so selecting a file dispatches `highlight` on that series
    // index. Empty selection downplays everything back to neutral.
    window._codeloreRegisterSelectionListener('trends', function (selectedPath) {
      // Downplay first: ECharts highlight is additive, so without this a
      // direct A→B selection (the drawer is non-modal, so switching files
      // without closing it is reachable) leaves A's series bold under B.
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      const idx = paths.indexOf(selectedPath);
      if (idx >= 0) {
        chart.dispatchAction({ type: 'highlight', seriesIndex: idx });
      }
    });
  }


  // ─── §11b Widget: Kamei Delivery-Risk Sparkline ──────────────────

  function renderKameiRiskSparkline(allRows) {
    const container = document.getElementById('widget-kamei-risk-body');
    if (!container) return;
    if (!allRows.length) {
      container.innerHTML = '<div class="empty">No Kamei JIT-SDP data — repo too small or kamei::enrich was not wired.</div>';
      return;
    }

    // User-tunable window. Backend now sends up to 100 most recent
    // non-merge commits; the SPA slices reactively per
    // `Alpine.store('layout').kameiWindow` (default 30, 'all' = no
    // cap). Rows are chronological; slicing from the tail keeps
    // the "most recent N" semantic.
    const kameiLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const kameiWindow = kameiLayout ? kameiLayout.kameiWindow : 30;
    const rows = (kameiWindow === 'all')
      ? allRows
      : allRows.slice(-Number(kameiWindow));

    // Normalisation anchors against the visible window. Per-feature
    // max so a 100-line commit doesn't dwarf a 5-LoC fix when the
    // composite is computed. log1p on size/spread features
    // (la/ld/nf/ndev/nuc/exp) keeps the right tail readable —
    // raw values span 3+ orders of magnitude on real repos.
    function logCap(v) { return Math.log1p(Math.max(0, v)); }
    var maxSize = 1, maxSpread = 1, maxConcurrency = 1, maxExp = 1, maxEntropy = 1;
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      maxSize       = Math.max(maxSize, logCap((r.la || 0) + (r.ld || 0)));
      maxSpread     = Math.max(maxSpread, logCap(r.nf || 0));
      maxConcurrency = Math.max(maxConcurrency, logCap(r.ndev || 0));
      maxExp        = Math.max(maxExp, logCap(r.exp || 0));
      maxEntropy    = Math.max(maxEntropy, r.entropy || 0);
    }

    // Per-commit composite: weighted sum of normalised dimensions.
    // Weights chosen to reflect Kamei 2013 §4 findings (size +
    // history dominate); not calibrated per-repo (a logistic-
    // regression fit is a future enhancement). Inexperience term:
    // low exp = higher risk, hence (1 - exp/max).
    function scoreOf(r) {
      const size = logCap((r.la || 0) + (r.ld || 0)) / maxSize;
      const spread = logCap(r.nf || 0) / maxSpread;
      const concurrency = logCap(r.ndev || 0) / maxConcurrency;
      const inexperience = 1 - (logCap(r.exp || 0) / maxExp);
      const entropy = (r.entropy || 0) / (maxEntropy || 1);
      // 0.30 size + 0.20 spread + 0.20 concurrency + 0.20 inexp + 0.10 entropy
      const composite = 0.30 * size + 0.20 * spread + 0.20 * concurrency
                      + 0.20 * inexperience + 0.10 * entropy;
      return {
        composite: Math.max(0, Math.min(1, composite)),
        size: size, spread: spread, concurrency: concurrency,
        inexperience: inexperience, entropy: entropy,
      };
    }

    // Identify which dimension dominates each commit's risk so the
    // tooltip can headline it. Captures the "why is this commit
    // risky?" answer instead of just the score.
    function dominantDimension(s) {
      const dims = [
        { name: 'size',        v: 0.30 * s.size },
        { name: 'spread',      v: 0.20 * s.spread },
        { name: 'concurrency', v: 0.20 * s.concurrency },
        { name: 'inexperience',v: 0.20 * s.inexperience },
        { name: 'entropy',     v: 0.10 * s.entropy },
      ];
      dims.sort(function (a, b) { return b.v - a.v; });
      return dims[0].name;
    }

    const seriesData = rows.map(function (r) {
      const s = scoreOf(r);
      const dom = dominantDimension(s);
      // Fix-commits get the error tone; otherwise heat-ramp by score.
      // Both reads use the cached token() helper because this widget
      // is registered via the standard rerenderers (not
      // registerThemeRerender) — but token() caching is harmless here
      // since the rerender loop fires both functions; the next theme
      // toggle re-cache happens during the next click handler.
      const color = r.fix
        ? token('--color-error')
        : heatRamp(s.composite);
      return {
        value: s.composite,
        itemStyle: { color: color },
        // Stash the row + dimensions for tooltip access.
        _rev: r.rev, _date: r.date, _row: r, _score: s, _dom: dom,
      };
    });

    var fixCount = 0;
    for (var fi = 0; fi < rows.length; fi++) {
      if (rows[fi].fix) fixCount++;
    }
    setChartAriaLabel(container,
      'Kamei delivery-risk per commit over ' + rows.length + ' recent commits, ' +
      fixCount + ' flagged as bug-fixes');

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        trigger: 'item',
        // Attach the tooltip DOM to the OUTER widget section
        // (#widget-kamei-risk) instead of the inner chart body
        // (#widget-kamei-risk-body). Positioning is then relative
        // to the whole panel — top-right lands in the panel's
        // header area, well clear of every bar in the chart
        // beneath. Without `appendTo`, ECharts attaches the
        // tooltip to the chart body so `{ top: 4, right: 4 }`
        // landed somewhere over the bars instead of above them.
        appendTo: function (chartDom) {
          return chartDom.closest('section.widget') || document.body;
        },
        position: function () {
          return { top: 4, right: 4 };
        },
        formatter: function (params) {
          const d = params.data || {};
          const r = d._row || {};
          const s = d._score || {};
          const dom = d._dom || 'size';
          const fmtPct = function (v) { return Math.round(v * 100) + '%'; };
          // Lead with the dominant dimension — the user's "why is
          // this commit risky?" question gets answered first, raw
          // Kamei vector underneath for the data-savvy reader.
          return '<b>' + escapeHtml((r.rev || '').slice(0, 8)) + '</b>'
            + '<br/><small>' + escapeHtml(r.date || '') + '</small>'
            + '<br/>composite: <b>' + fmtPct(s.composite) + '</b>'
            + ' · dominant: <b>' + dom + '</b>'
            + (r.fix ? '<br/><span class="badge badge-error badge-sm">bug-fix</span>' : '')
            + '<br/><br/><small>'
            + 'la=' + (r.la || 0) + ' · ld=' + (r.ld || 0)
            + ' · nf=' + (r.nf || 0) + ' · ndev=' + (r.ndev || 0)
            + ' · exp=' + (r.exp || 0) + ' · entropy=' + (r.entropy || 0).toFixed(2)
            + '</small>'
            + '<br/><small style="opacity:.6;">'
            + 'size=' + fmtPct(s.size) + ' · spread=' + fmtPct(s.spread)
            + ' · concurrency=' + fmtPct(s.concurrency)
            + ' · inexp=' + fmtPct(s.inexperience)
            + ' · entropy=' + fmtPct(s.entropy)
            + '</small>';
        },
      },
      grid: { top: 14, left: 50, right: 20, bottom: 30 },
      xAxis: {
        type: 'category',
        data: rows.map(function (r) { return r.date; }),
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 10, rotate: 45 },
        axisLine: { lineStyle: { color: getCssVar('--border') } },
      },
      yAxis: {
        type: 'value',
        min: 0, max: 1,
        name: 'risk',
        nameTextStyle: { color: getCssVar('--fg-dim'), fontSize: 10 },
        axisLabel: {
          color: getCssVar('--fg-dim'), fontSize: 10,
          formatter: function (v) { return Math.round(v * 100) + '%'; },
        },
        splitLine: { lineStyle: { color: getCssVar('--bg-elev-2') } },
      },
      series: [{
        type: 'bar',
        data: seriesData,
        barCategoryGap: '20%',
        // `emphasis: { disabled: true }` — ECharts 6 had a regression
        // on this exact combination (`focus: 'self'` + per-data
        // `itemStyle.color` on a `type: 'bar'` series): hovering the
        // bar dropped its rendered color, painting it with the chart
        // background → "the bar disappears." Disabling emphasis
        // entirely is the safe contract: no hover transform on the
        // bar, the tooltip carries the affordance. Same idiom on
        // parallel-coords below.
        emphasis: { disabled: true },
      }],
    });
  }


  // ─── §11c Widget: Hotspot treemap ────────────────────────────────

  function renderHotspotTreemap(rows) {
    const container = document.getElementById('widget-hotspot-treemap-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No hotspot data for treemap.</div>';
      return;
    }
    // Tree-shape: top-level dir → file leaves. Cap at top-200 hotspots
    // by hotspot_score so the treemap renders cleanly on big repos.
    const TREEMAP_CAP = 200;
    const top = rows.slice()
      .sort(function (a, b) {
        const sa = (typeof a.hotspot_score === 'number') ? a.hotspot_score : -Infinity;
        const sb = (typeof b.hotspot_score === 'number') ? b.hotspot_score : -Infinity;
        return sb - sa;
      })
      .slice(0, TREEMAP_CAP);
    const grouped = {};
    for (var i = 0; i < top.length; i++) {
      const r = top[i];
      const parts = (r.path || '').split('/');
      const dir = parts.length > 1 ? parts[0] : '<root>';
      if (!grouped[dir]) grouped[dir] = [];
      grouped[dir].push({
        name: r.path,
        value: r.revisions || 1,
        cognitive: r.cognitive || 0,
        code_health: r.code_health,
        hotspot_score: r.hotspot_score,
      });
    }
    const treeData = Object.keys(grouped).sort().map(function (dir) {
      return { name: dir, children: grouped[dir] };
    });
    setChartAriaLabel(container,
      'Hotspot treemap of ' + top.length + ' files across ' +
      treeData.length + ' top-level directories, sized by revisions');
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        formatter: function (params) {
          const d = params.data || {};
          if (!d.cognitive) return '<b>' + escapeHtml(d.name || '') + '</b><br/>directory';
          return '<b>' + escapeHtml(d.name) + '</b>' +
            '<br/>revisions: ' + (d.value || 0) +
            '<br/>cognitive: ' + d.cognitive.toFixed(0) +
            (d.code_health != null ? '<br/>health: ' + d.code_health.toFixed(1) : '') +
            (d.hotspot_score != null ? '<br/>score: ' + d.hotspot_score.toFixed(2) : '');
        },
      },
      series: [{
        type: 'treemap',
        data: treeData,
        roam: false,
        // Semantic-zoom drill-down. `leafDepth: 2` collapses the tree
        // so the top-level directories render first; clicking a
        // directory drills into its file children with an ECharts-
        // internal morph animation. The native breadcrumb (top-left
        // by default) tracks the drill path and gives one-click
        // ascent back up the hierarchy — much cleaner than the prior
        // single-flat-view that buried every file in a 200-leaf grid.
        // Spec: Apache ECharts treemap-drill-down example
        // <https://echarts.apache.org/examples/en/editor.html?c=treemap-drill-down>.
        leafDepth: 2,
        breadcrumb: {
          show: true,
          top: 6,
          left: 6,
          itemStyle: {
            color: getCssVar('--bg-elev'),
            borderColor: getCssVar('--border'),
            textStyle: { color: getCssVar('--fg') },
          },
        },
        label: { show: true, color: token('--label-on-saturated'), fontSize: 11 },
        upperLabel: { show: true, height: 18, color: getCssVar('--fg-dim'), fontSize: 11 },
        // Per-depth styling: directory level (depth 1) carries a
        // thicker border + larger gap to read as a container; file
        // level (depth 2) tightens both so leaves pack densely. With
        // `leafDepth: 2` only depths 1 and 2 are ever rendered, so a
        // depth-3 levels entry would be dead config.
        // Progressive color saturation is left to ECharts' default
        // visualMin/visualMax behavior so the existing tooltip
        // color-coding (revisions × hotspot_score) survives.
        levels: [
          { itemStyle: { borderColor: getCssVar('--border'), borderWidth: 3, gapWidth: 3 } },
          { itemStyle: { borderColor: getCssVar('--border'), borderWidth: 2, gapWidth: 2 } },
        ],
      }],
    });
    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.cognitive != null) {
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(d.name);
        } else {
          showFileDetailDrawer(d.name, data);
        }
      }
    });
  }


  // ─── §11d Widget: Parallel coordinates ───────────────────────────

  function renderParallelCoords(rows) {
    const container = document.getElementById('widget-parallel-coords-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No hotspot data for parallel coords.</div>';
      return;
    }
    // User-tunable Top-N. Sorted by hotspot_score descending so the
    // polylines remain the highest-pressure files at any setting.
    // Stored in `Alpine.store('layout').parallelTopN` (default 20,
    // 'all' = no cap), persisted across reloads.
    const parallelLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const parallelTopN = parallelLayout ? parallelLayout.parallelTopN : 20;
    const sorted = rows.slice()
      .sort(function (a, b) {
        const sa = (typeof a.hotspot_score === 'number') ? a.hotspot_score : -Infinity;
        const sb = (typeof b.hotspot_score === 'number') ? b.hotspot_score : -Infinity;
        return sb - sa;
      });
    const top = (parallelTopN === 'all') ? sorted : sorted.slice(0, Number(parallelTopN));
    setChartAriaLabel(container,
      'Parallel-coordinates plot of ' + top.length + ' files across ' +
      'revisions, cognitive complexity, code health, hotspot score and MI rank');
    const chart = mountEcharts(container);
    // Build the polyline data once so the cross-widget selection listener can
    // re-style individual lines by mutating their per-item lineStyle (emphasis
    // is disabled on this series — see below — so highlight/downplay are inert).
    const parallelData = top.map(function (r) {
      return {
        name: r.path,
        value: [
          r.revisions || 0,
          r.cognitive || 0,
          r.code_health != null ? r.code_health : 0,
          r.hotspot_score != null ? r.hotspot_score : 0,
          typeof r.mi_rank === 'number' ? r.mi_rank : 0,
        ],
      };
    });
    chart.setOption({
      parallelAxis: [
        { dim: 0, name: 'Revisions' },
        { dim: 1, name: 'Cognitive' },
        { dim: 2, name: 'Code health', inverse: true },
        { dim: 3, name: 'Hotspot score' },
        { dim: 4, name: 'MI rank', max: 1.0 },
      ],
      parallel: {
        left: 50, right: 50, top: 30, bottom: 30,
        axisExpandable: false,
        parallelAxisDefault: {
          axisLabel: { color: getCssVar('--fg-dim'), fontSize: 10 },
          nameTextStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
          axisLine: { lineStyle: { color: getCssVar('--border') } },
        },
      },
      tooltip: {
        trigger: 'item',
        // Position function pins the tooltip to the top-right of
        // the chart instead of following the cursor — at-cursor
        // placement on a parallel-coords polyline puts the popup
        // directly over the line the user is reading, so it looks
        // like the line vanished. Top-right keeps the data visible
        // and the popup readable in one glance.
        position: function (point, params, dom, rect, size) {
          return [size.viewSize[0] - size.contentSize[0] - 12, 8];
        },
        confine: true,
        formatter: function (params) {
          const v = params.value || [];
          return '<b>' + escapeHtml(params.name || '') + '</b>' +
            '<br/>revisions: ' + (v[0] || 0) +
            '<br/>cognitive: ' + (v[1] || 0).toFixed(0) +
            '<br/>health: ' + (v[2] || 0).toFixed(1) +
            '<br/>score: ' + (v[3] || 0).toFixed(2) +
            '<br/>MI rank: ' + (typeof v[4] === 'number' ? (v[4] * 100).toFixed(0) + '%' : '—');
        },
      },
      series: [{
        type: 'parallel',
        lineStyle: { width: 1, opacity: 0.6, color: token('--color-warning') },
        // `emphasis: { disabled: true }` — ECharts 6 had the same
        // hovered-item-disappears regression on this parallel
        // series as on the Kamei bar series above. Disable the
        // emphasis transform entirely; the tooltip carries all the
        // hover affordance we need, the polyline remains its normal
        // colour and width regardless of hover state.
        emphasis: { disabled: true },
        data: parallelData,
      }],
    });
    chart.on('click', function (params) {
      if (params && params.name) {
        // Route through `_codeloreShowDetail` so the click both
        // opens the drawer AND publishes the selection — direct
        // `showFileDetailDrawer` would only do the former.
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(params.name);
        } else {
          showFileDetailDrawer(params.name, data);
        }
      }
    });
    // Cross-widget selection: emphasis is disabled on this series (ECharts 6
    // hover-disappears regression), so highlight/downplay do nothing. Instead
    // re-style the lines directly — the selected file's polyline goes bold
    // info-blue while the rest fade; a null selection restores every line to
    // the default warning colour. Mutates parallelData in place + re-applies.
    const parallelPaths = top.map(function (r) { return r.path; });
    const parallelBase = { color: token('--color-warning'), width: 1, opacity: 0.6 };
    window._codeloreRegisterSelectionListener('parallel-coords', function (selectedPath) {
      const idx = selectedPath ? parallelPaths.indexOf(selectedPath) : -1;
      for (var i = 0; i < parallelData.length; i++) {
        if (idx < 0) {
          parallelData[i].lineStyle = parallelBase;
        } else if (i === idx) {
          parallelData[i].lineStyle = { color: token('--color-info'), width: 3, opacity: 1 };
        } else {
          parallelData[i].lineStyle = { color: token('--color-warning'), width: 1, opacity: 0.12 };
        }
      }
      chart.setOption({ series: [{ data: parallelData }] });
    });
  }


  // ─── §11e Widget: Cognitive complexity boxplot ───────────────────

  function renderCognitiveBoxplot(rows) {
    const container = document.getElementById('widget-cognitive-boxplot-body');
    if (!container) return;
    const values = rows
      .map(function (r) { return r.cognitive; })
      .filter(function (v) { return typeof v === 'number' && v > 0; })
      .sort(function (a, b) { return a - b; });
    if (values.length < 5) {
      container.innerHTML = '<div class="empty">Insufficient data for boxplot.</div>';
      return;
    }
    function quantile(arr, q) {
      const pos = (arr.length - 1) * q;
      const base = Math.floor(pos);
      const rest = pos - base;
      return arr[base + 1] !== undefined
        ? arr[base] + rest * (arr[base + 1] - arr[base])
        : arr[base];
    }
    const min = values[0];
    const q1 = quantile(values, 0.25);
    const med = quantile(values, 0.5);
    const q3 = quantile(values, 0.75);
    const iqr = q3 - q1;
    const upperFence = q3 + 1.5 * iqr;
    const lowerFence = Math.max(0, q1 - 1.5 * iqr);
    const max = Math.min(upperFence, values[values.length - 1]);
    const outliers = [];
    for (var i = 0; i < values.length; i++) {
      if (values[i] > upperFence || values[i] < lowerFence) {
        outliers.push([0, values[i]]);
      }
    }
    const chart = mountEcharts(container);
    // Outliers share the y-axis with the box; if any reach far above
    // the upper fence (cognitive often has a long tail), the auto-fit
    // scale collapses the IQR to a few pixels. Clip the y-axis to the
    // whisker range with a small headroom and surface the outlier
    // tally + extreme value as a corner annotation so the information
    // is preserved without distorting the box.
    const maxOutlier = outliers.length
      ? outliers.reduce(function (m, o) { return o[1] > m ? o[1] : m; }, 0)
      : 0;
    setChartAriaLabel(container,
      'Cognitive-complexity distribution across ' + values.length +
      ' functions, median ' + Math.round(med) + ', ' + outliers.length + ' outliers');
    const yAxisMax = Math.ceil(upperFence * 1.15);
    chart.setOption({
      tooltip: { trigger: 'item' },
      grid: { top: 30, left: 60, right: 24, bottom: 36 },
      xAxis: {
        type: 'category',
        data: ['cognitive'],
        boundaryGap: true,
        axisLabel: { color: getCssVar('--fg-dim') },
      },
      yAxis: {
        type: 'value',
        min: 0,
        max: yAxisMax,
        axisLabel: { color: getCssVar('--fg-dim') },
        splitLine: { lineStyle: { color: getCssVar('--bg-elev-2') } },
      },
      series: [
        {
          type: 'boxplot',
          boxWidth: [60, 140],
          data: [[min, q1, med, q3, max]],
          itemStyle: { color: token('--color-warning'), borderColor: token('--color-error') },
        },
      ],
      graphic: outliers.length
        ? [{
            type: 'text',
            right: 16,
            top: 8,
            style: {
              text: '+' + outliers.length + ' outliers · max ' + Math.round(maxOutlier),
              fill: getCssVar('--fg-dim'),
              fontSize: 11,
            },
          }]
        : [],
    });
  }


  // ─── §11f Widget: Module chord diagram ───────────────────────────

  function renderModuleChord(rows) {
    const container = document.getElementById('widget-module-chord-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No coupling data for module chord.</div>';
      return;
    }
    // Roll up each pair to module-level using the first N path
    // segments (e.g. depth=2 → `app/services/x.py` → `app/services`).
    // Different repos have different natural granularities — a Rust
    // workspace at the crate level may want depth 1-2; a Python web
    // app with deep `app/services/<feature>/...` trees needs 3+.
    // Fixed depth + infra filter often collapses to 2-3 nodes; we
    // deepen adaptively until the chord has enough structure to
    // read.
    function modulePath(p, depth) {
      const parts = (p || '').split('/');
      if (parts.length <= depth) {
        const lastSlash = (p || '').lastIndexOf('/');
        return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
      }
      return parts.slice(0, depth).join('/');
    }
    // Drop infrastructure / configuration files from the chord.
    // Change-coupling captures co-modification, so lock files,
    // env files, and top-level docs cluster with every release
    // commit they were bumped in — they pollute the module
    // diagram with edges that don't represent code architecture.
    // Keep this list conservative; users who explicitly want
    // infra coupling can read the raw coupling table.
    function isInfrastructureFile(p) {
      if (!p) return false;
      // Lock files (uv.lock, package-lock.json, Cargo.lock, poetry.lock, yarn.lock, ...)
      if (/\.lock$/i.test(p) || /-lock\.json$/i.test(p)) return true;
      // Env files (.env, .env.example, .env.test, .env.local, ...)
      if (/(^|\/)\.env(\..+)?$/i.test(p)) return true;
      // Docs (top-level .md / .rst / .txt + entire docs/** tree)
      if (/^docs?\//i.test(p)) return true;
      if (/\.(md|rst|txt|adoc)$/i.test(p)) return true;
      // Build / dependency manifests at any depth
      if (/(^|\/)(pyproject|Cargo|package|composer|Gemfile|setup)\.(toml|json|yaml|yml)$/i.test(p)) return true;
      if (/(^|\/)(requirements[^/]*|setup)\.(txt|cfg|py)$/i.test(p)) return true;
      // CI / repo metadata
      if (/^\.github\//i.test(p) || /^\.gitlab/i.test(p)) return true;
      if (/^\.(gitignore|gitattributes|dockerignore|editorconfig|prettierrc|eslintrc)/i.test(p)) return true;
      // Version markers
      if (/(^|\/)VERSION$/.test(p) || /(^|\/)CHANGELOG(\.[^/]+)?$/i.test(p)) return true;
      return false;
    }
    // Depth source:
    //   'auto' (default) → adaptive loop, deepens until ≥ MIN_NODES
    //   integer 2-6     → user-fixed depth, no adaptation
    // The user setting lives in `Alpine.store('layout').chordDepth`
    // (persisted across reloads). When the store value changes, an
    // Alpine effect in template.html fires the rerenderer list,
    // which re-runs this function with the new setting.
    const MIN_NODES_FOR_USEFUL_CHORD = 6;
    const layout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userChordDepth = layout ? layout.chordDepth : 'auto';
    function aggregateAt(depth) {
      const ee = {};
      const nn = {};
      for (var i = 0; i < rows.length; i++) {
        const r = rows[i];
        if (isInfrastructureFile(r.entity_a) || isInfrastructureFile(r.entity_b)) continue;
        const a = modulePath(r.entity_a, depth);
        const b = modulePath(r.entity_b, depth);
        if (!a || !b || a === b) continue;
        const key = a < b ? a + '\x00' + b : b + '\x00' + a;
        ee[key] = (ee[key] || 0) + (r.shared || 1);
        nn[a] = true;
        nn[b] = true;
      }
      return { edges: ee, nodeCount: Object.keys(nn).length };
    }
    var edges = {};
    if (typeof userChordDepth === 'number') {
      edges = aggregateAt(userChordDepth).edges;
    } else {
      for (var depth = 2; depth <= 6; depth++) {
        const result = aggregateAt(depth);
        edges = result.edges;
        if (result.nodeCount >= MIN_NODES_FOR_USEFUL_CHORD) break;
      }
    }
    const linkRows = Object.keys(edges).map(function (k) {
      const parts = k.split('\x00');
      return { source: parts[0], target: parts[1], value: edges[k] };
    });
    if (!linkRows.length) {
      container.innerHTML = '<div class="empty">All change-coupling stays inside a single 2-segment module after dropping infrastructure files (lock / env / docs / build manifests). See the raw <em>coupling</em> table for the full pair list.</div>';
      return;
    }
    const nodes = {};
    for (var ei = 0; ei < linkRows.length; ei++) {
      nodes[linkRows[ei].source] = true;
      nodes[linkRows[ei].target] = true;
    }
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
    setChartAriaLabel(container,
      'Module change-coupling chord diagram, ' + nodeArr.length + ' modules and ' +
      linkRows.length + ' coupled pairs');
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: { trigger: 'item' },
      series: [{
        type: 'graph',
        layout: 'circular',
        circular: { rotateLabel: true },
        data: nodeArr,
        categories: categories,
        links: linkRows,
        roam: false,
        label: { show: true, color: getCssVar('--fg-dim'), fontSize: 10, position: 'right' },
        lineStyle: {
          color: 'source',
          opacity: 0.45,
          curveness: 0.45,
        },
        emphasis: { focus: 'adjacency', lineStyle: { width: 3 } },
      }],
    });
  }


  // ─── §11g Architecture widgets: shared module roll-up ────────────
  // Both the force graph and the dependency-structure matrix roll file
  // paths up to their module at `depth` segments and aggregate the
  // resolved import edges the same way; the shared helpers live here so
  // the two views can never drift.

  // Roll a file path up to its module at `depth` path segments (parent
  // dir when the file has fewer than `depth` segments, so leaf-level
  // imports don't collapse onto themselves).
  function modulePath(p, depth) {
    const parts = (p || '').split('/');
    if (parts.length <= depth) {
      const lastSlash = (p || '').lastIndexOf('/');
      return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
    }
    return parts.slice(0, depth).join('/');
  }

  // Aggregate resolved import edges to module granularity at `depth`,
  // dropping self-edges. Returns `{ edges: {src\x00tgt: count}, nodes }`.
  function aggregateImportsAt(imports, depth) {
    const ee = {};
    const nn = {};
    for (var i = 0; i < imports.length; i++) {
      const imp = imports[i];
      if (!imp.target_path) continue;
      const s = modulePath(imp.src_path, depth);
      const t = modulePath(imp.target_path, depth);
      if (!s || !t || s === t) continue;
      const key = s + '\x00' + t;
      ee[key] = (ee[key] || 0) + 1;
      nn[s] = true;
      nn[t] = true;
    }
    return { edges: ee, nodes: nn };
  }

  // ─── §11g Widget: Architecture force-graph ───────────────────────

  function renderArchGraph(imports, violations, unstable, roles) {
    violations = violations || [];
    unstable = unstable || [];
    roles = roles || [];
    const container = document.getElementById('widget-arch-graph-body');
    if (!container) return;
    if (!imports.length) {
      container.innerHTML = '<div class="empty">No resolved import edges yet. The resolver covers Rust, Python, and JS/TS today; Java FQN&rarr;file mapping is not attempted.</div>';
      return;
    }
    // Adaptive module depth: aggregate edges using path prefixes,
    // starting at 2 segments and deepening until at least one
    // inter-module edge survives. Different repos have different
    // "natural" module boundaries (Rust crates: 1-2 segments,
    // Django apps: 2-3, monorepos: 3-5). A fixed depth empties the
    // graph for repos whose imports all sit under a single root
    // like `app/services/` — we'd then show the misleading "stays
    // intra-module" message. Caps at 6 segments to keep labels
    // readable.
    // Depth source:
    //   'auto' (default) → adaptive loop, deepens until ≥ MIN_NODES
    //   integer 2-6     → user-fixed depth, no adaptation
    // The user setting lives in `Alpine.store('layout').archGraphDepth`
    // (persisted across reloads). When the store value changes, an
    // Alpine effect in template.html fires the rerenderer list,
    // which re-runs this function with the new setting.
    const MIN_NODES_FOR_USEFUL_GRAPH = 8;
    const archLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userArchDepth = archLayout ? archLayout.archGraphDepth : 'auto';
    var edges = {};
    var nodes = {};
    // The depth the structural aggregation settled on — reused for the
    // fusion overlay so violation edges + unstable nodes roll up to the
    // SAME module granularity as the import edges.
    var chosenDepth = (typeof userArchDepth === 'number') ? userArchDepth : 6;
    if (typeof userArchDepth === 'number') {
      const result = aggregateImportsAt(imports, userArchDepth);
      edges = result.edges;
      nodes = result.nodes;
    } else {
      for (var depth = 2; depth <= 6; depth++) {
        const result = aggregateImportsAt(imports, depth);
        edges = result.edges;
        nodes = result.nodes;
        chosenDepth = depth;
        if (Object.keys(nodes).length >= MIN_NODES_FOR_USEFUL_GRAPH) break;
      }
    }
    // Fusion overlay #1 — modularity-violation edges: Fisher-significant
    // co-change pairs with NO structural import edge, aggregated to
    // module level. These are the "temporal-only" edges an import graph
    // cannot show (implicit/hidden coupling). Keyed strongest-degree.
    const violEdges = {};
    for (var vi = 0; vi < violations.length; vi++) {
      const v = violations[vi];
      const vs = modulePath(v.entity_a, chosenDepth);
      const vt = modulePath(v.entity_b, chosenDepth);
      if (!vs || !vt || vs === vt) continue;
      const vkey = vs + '\x00' + vt;
      const vd = (typeof v.degree === 'number') ? v.degree : 0;
      if (!(vkey in violEdges) || vd > violEdges[vkey]) violEdges[vkey] = vd;
      nodes[vs] = true;
      nodes[vt] = true;
    }
    // Fusion overlay #2 — unstable-interface modules (warning nodes):
    // heavily-imported files that change often and drag their dependents.
    const unstableModules = {};
    for (var ui = 0; ui < unstable.length; ui++) {
      const um = modulePath(unstable[ui].path, chosenDepth);
      if (!um) continue;
      unstableModules[um] = (unstableModules[um] || 0) + 1;
      nodes[um] = true;
    }
    // Architecture roles (Core/Shared/Control/Periphery) + cycle
    // membership, aggregated to module level. A module takes the
    // highest-precedence role among its files (core > control > shared >
    // periphery). Roles only COLOUR existing nodes — they don't add
    // isolated single-file nodes. Also accumulate the propagation cost.
    const ROLE_RANK = { core: 3, control: 2, shared: 1, periphery: 0 };
    const moduleRole = {};
    const moduleInCycle = {};
    // Shallowest topological level among a module's files — the band a
    // module sits in for the layered layout (entry points = level 0).
    const moduleLevel = {};
    var vfoSum = 0;
    var fileCount = 0;
    var filesInCycles = 0;
    for (var rri = 0; rri < roles.length; rri++) {
      const rr = roles[rri];
      fileCount += 1;
      vfoSum += (typeof rr.vfo === 'number') ? rr.vfo : 0;
      if (rr.in_cycle) filesInCycles += 1;
      const rm = modulePath(rr.path, chosenDepth);
      if (!rm) continue;
      const cur = moduleRole[rm];
      if (cur === undefined || (ROLE_RANK[rr.role] || 0) > (ROLE_RANK[cur] || 0)) {
        moduleRole[rm] = rr.role;
      }
      if (rr.in_cycle) moduleInCycle[rm] = true;
      const lv = (typeof rr.level === 'number') ? rr.level : 1e9;
      if (moduleLevel[rm] === undefined || lv < moduleLevel[rm]) moduleLevel[rm] = lv;
    }
    // Propagation cost = mean(vfo)/fileCount = sum(vfo)/fileCount² — "a
    // change to a random file reaches this fraction of the system".
    const propagationCost = fileCount > 0 ? (vfoSum / (fileCount * fileCount)) : 0;
    if (!Object.keys(nodes).length) {
      container.innerHTML = '<div class="empty">All resolved imports stay intra-module — no inter-module edges to graph.</div>';
      return;
    }
    const roleColors = {
      core: token('--color-error') || '#dc2626',
      control: token('--color-warning') || '#d97706',
      shared: token('--color-info') || '#2563eb',
      periphery: token('--color-neutral') || '#6b7280',
    };
    const violColor = roleColors.control;
    const cycleRing = getCssVar('--fg') || '#111827';
    const ROLE_ORDER = ['core', 'control', 'shared', 'periphery'];
    const nodeArr = Object.keys(nodes).map(function (n) {
      const role = moduleRole[n] || 'periphery';
      const isUnstable = !!unstableModules[n];
      const inCycle = !!moduleInCycle[n];
      const cat = ROLE_ORDER.indexOf(role);
      return {
        name: n,
        symbol: isUnstable ? 'diamond' : 'circle',
        symbolSize: isUnstable ? 42 : 30,
        category: cat < 0 ? 3 : cat,
        // A high-contrast ring marks modules that sit in a dependency
        // cycle; the fill stays the role colour.
        itemStyle: inCycle ? { borderColor: cycleRing, borderWidth: 3 } : undefined,
      };
    });
    // Layout mode: 'force' (default physics) or 'layered' (topological
    // bands). Layered de-hairballs the graph — modules stack in
    // horizontal bands by level, so forward deps flow downward and
    // back-edges (cycles) visibly run back up. Encodings (role colour,
    // cycle ring, unstable diamond) are on the nodes, so they carry over.
    const layoutMode = (archLayout && archLayout.archGraphLayout === 'layered')
      ? 'layered' : 'force';
    if (layoutMode === 'layered') {
      const W = container.clientWidth || 900;
      const padX = 64;
      const padTop = 40;
      const padBot = 40;
      const rowGap = 96;
      const minSpacingX = 96; // horizontal room per node so labels don't collide
      const usableW = W - 2 * padX;
      const perRow = Math.max(1, Math.floor(usableW / minSpacingX));
      // Bucket nodes by level in one pass; the distinct levels (shallow→
      // deep, entry points first) fall out of the bucket keys.
      const byLevel = {};
      nodeArr.forEach(function (nd) {
        const lv = (moduleLevel[nd.name] === undefined) ? 0 : moduleLevel[nd.name];
        (byLevel[lv] = byLevel[lv] || []).push(nd);
      });
      const levelsPresent = Object.keys(byLevel)
        .map(Number)
        .sort(function (a, b) { return a - b; });
      // Each level occupies one or more sub-rows: a band wider than
      // `perRow` wraps instead of squeezing its nodes (and their labels)
      // into an unreadable single line. Rows accumulate top-to-bottom so
      // level order — and the downward forward-dep flow — is preserved.
      var rowCursor = 0;
      levelsPresent.forEach(function (lv) {
        const band = byLevel[lv].sort(function (a, c) { return a.name < c.name ? -1 : 1; });
        const subRows = Math.max(1, Math.ceil(band.length / perRow));
        for (var sr = 0; sr < subRows; sr++) {
          const slice = band.slice(sr * perRow, (sr + 1) * perRow);
          const k = slice.length;
          const yRow = padTop + (rowCursor + sr) * rowGap;
          slice.forEach(function (nd, i) {
            nd.x = (k === 1) ? (W / 2) : (padX + (i / (k - 1)) * usableW);
            nd.y = yRow;
          });
        }
        rowCursor += subRows;
      });
      // Grow the container to fit all rows (force mode keeps the compact
      // default); panning via roam handles anything still off-screen.
      const totalRows = Math.max(rowCursor, 1);
      container.style.height = (padTop + padBot + (totalRows - 1) * rowGap + 40) + 'px';
    } else {
      // Restore the template's default height when leaving layered mode.
      container.style.height = '380px';
    }
    const structuralLinks = Object.keys(edges).map(function (k) {
      const parts = k.split('\x00');
      return { source: parts[0], target: parts[1], value: edges[k], _kind: 'import' };
    });
    const violationLinks = Object.keys(violEdges).map(function (k) {
      const parts = k.split('\x00');
      return {
        source: parts[0],
        target: parts[1],
        value: violEdges[k],
        _kind: 'violation',
        lineStyle: { color: violColor, type: 'dashed', opacity: 0.9, width: 2, curveness: 0.2 },
      };
    });
    const edgeArr = structuralLinks.concat(violationLinks);
    const cycleModuleCount = Object.keys(moduleInCycle).length;
    setChartAriaLabel(container,
      'Architecture graph, ' + nodeArr.length + ' modules coloured by role ' +
      '(core/control/shared/periphery), ' + structuralLinks.length + ' import edges, ' +
      violationLinks.length + ' dashed modularity-violation edges, ' +
      Object.keys(unstableModules).length + ' unstable-interface modules shown as diamonds, ' +
      cycleModuleCount + ' modules in dependency cycles shown ringed. Propagation cost ' +
      (propagationCost * 100).toFixed(1) + ' percent.');
    const chart = mountEcharts(container);
    const titleText = fileCount > 0
      ? 'Propagation cost ' + (propagationCost * 100).toFixed(1) + '%  ·  ' +
        filesInCycles + ' files in cycles'
      : '';
    chart.setOption({
      title: {
        text: titleText,
        left: 'center',
        top: 4,
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 12, fontWeight: 'normal' },
      },
      tooltip: {
        trigger: 'item',
        formatter: function (p) {
          if (p.dataType === 'edge') {
            if (p.data && p.data._kind === 'violation') {
              return 'Modularity violation — co-change, no import<br/>' +
                p.data.source + ' &harr; ' + p.data.target +
                '<br/>coupling degree ' + (Number(p.data.value) || 0).toFixed(1) + '%';
            }
            return 'Imports: ' + p.data.source + ' &rarr; ' + p.data.target +
              ' (' + p.data.value + ')';
          }
          return p.name + '<br/>role: ' + (moduleRole[p.name] || 'periphery') +
            (moduleInCycle[p.name] ? ' &middot; in cycle' : '') +
            (unstableModules[p.name] ? ' &middot; unstable interface' : '');
        },
      },
      legend: [{
        data: ROLE_ORDER,
        textStyle: { color: getCssVar('--fg-dim') },
        bottom: 0,
      }],
      series: [{
        type: 'graph',
        layout: 'force',
        categories: [
          { name: 'core', itemStyle: { color: roleColors.core } },
          { name: 'control', itemStyle: { color: roleColors.control } },
          { name: 'shared', itemStyle: { color: roleColors.shared } },
          { name: 'periphery', itemStyle: { color: roleColors.periphery } },
        ],
        data: nodeArr,
        links: edgeArr,
        roam: true,
        layout: layoutMode === 'layered' ? 'none' : 'force',
        force: { repulsion: 200, edgeLength: 80 },
        // In layered mode draw arrowheads so the downward = forward,
        // upward = back-edge (cycle) flow is legible; force mode keeps
        // the cleaner undecorated lines.
        edgeSymbol: layoutMode === 'layered' ? ['none', 'arrow'] : 'none',
        edgeSymbolSize: 7,
        // Layered mode packs nodes tightly, so label with the leaf
        // segment only (full module path still shows in the tooltip);
        // force mode has room for the full name.
        label: {
          show: true,
          color: getCssVar('--fg-dim'),
          fontSize: 11,
          formatter: layoutMode === 'layered'
            ? function (p) { const s = String(p.name).split('/'); return s[s.length - 1]; }
            : undefined,
        },
        lineStyle: { color: token('--color-info'), opacity: 0.6, width: 1.5 },
        emphasis: { focus: 'adjacency', lineStyle: { width: 3 } },
      }],
    });
    // Reset-zoom for the arch graph: re-run the renderer to wipe
    // the chart's internal `roam` state (pan offset + zoom level)
    // — ECharts reuses the chart instance via getInstanceByDom, so
    // this is a cheap setOption, not a full re-mount. Same effect
    // as the Hotspots widget's `_codeloreZoomReset` but driven by
    // ECharts' built-in roam reset instead of the CSS-transform
    // overlay.
    window._codeloreResetZoomHandlers['widget-arch-graph'] = function () {
      renderArchGraph(imports, violations, unstable, roles);
    };
  }


  // ─── §11h Widget: Dependency Structure Matrix ────────────────────

  // The scalable, layer-ordered view of the same import graph. A DSM
  // (Steward 1981; Sangal et al. 2005) is the matrix form of the
  // dependency graph: it does not hairball as the module count grows.
  // Modules are ordered by architectural layer (from architecture-
  // roles' topological `level`); a cell (row imports col) coloured blue
  // is a healthy forward dependency (above the diagonal), red is a
  // back-edge (below the diagonal) — which only happens inside a
  // dependency cycle. A clean acyclic architecture is a triangular,
  // all-blue matrix.
  // ─── §11i Widget: Architecture decay trend ──────────────────────
  // Dual-axis line over the sampled historical revisions: propagation
  // cost (left, blue) and dependency-cycle count (right, red, stepped).
  // Answers "is the architecture decaying, and when did it start?" —
  // the HEAD metrics projected back across history.
  function renderArchTrend(rows) {
    const container = document.getElementById('widget-arch-trend-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No architecture-trend data — repo too small, or the historical scan was skipped.</div>';
      return;
    }
    const dates = rows.map(function (r) { return r.date; });
    const propagation = rows.map(function (r) {
      return Number(((r.propagation_cost || 0) * 100).toFixed(2));
    });
    const cycles = rows.map(function (r) { return r.cycle_count || 0; });
    const infoColor = token('--color-info') || '#2563eb';
    const errColor = token('--color-error') || '#dc2626';
    const dim = getCssVar('--fg-dim');

    setChartAriaLabel(container,
      'Architecture decay trend over ' + rows.length +
      ' sampled revisions: propagation cost (percent) and dependency-cycle count.');

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: { trigger: 'axis' },
      legend: {
        data: ['Propagation cost %', 'Dependency cycles'],
        textStyle: { color: dim },
        bottom: 0,
      },
      grid: { left: 8, right: 8, top: 24, bottom: 48, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLabel: { color: dim, rotate: 30, fontSize: 10 },
      },
      yAxis: [
        {
          type: 'value',
          name: 'Propagation %',
          position: 'left',
          axisLabel: { color: dim },
          nameTextStyle: { color: dim },
          splitLine: { lineStyle: { color: getCssVar('--border') } },
        },
        {
          type: 'value',
          name: 'Cycles',
          position: 'right',
          minInterval: 1,
          axisLabel: { color: dim },
          nameTextStyle: { color: dim },
          splitLine: { show: false },
        },
      ],
      series: [
        {
          name: 'Propagation cost %',
          type: 'line',
          smooth: true,
          symbol: 'circle',
          symbolSize: 6,
          yAxisIndex: 0,
          data: propagation,
          lineStyle: { color: infoColor, width: 2 },
          itemStyle: { color: infoColor },
          areaStyle: { color: infoColor, opacity: 0.08 },
        },
        {
          name: 'Dependency cycles',
          type: 'line',
          step: 'end',
          symbol: 'circle',
          symbolSize: 6,
          yAxisIndex: 1,
          data: cycles,
          lineStyle: { color: errColor, width: 2 },
          itemStyle: { color: errColor },
        },
      ],
    });
  }

  // ─── §11j Widget: Repo health timeline ─────────────────────────────
  // Overlaid 3-line chart (Combined bold; Architectural + Code lighter)
  // on a 0–100 axis with faint red/yellow/green band background, plus a
  // vanilla toggle that re-renders as three stacked small-multiples.
  function healthTrendBands(errColor, warnColor, okColor) {
    // Red / yellow / green background zones (0-40 / 40-70 / 70-100). Colors are
    // resolved once by the caller and indexed by band, rather than re-read
    // inside the ECharts per-entry color callback.
    var zoneColors = [errColor, warnColor, okColor];
    return {
      silent: true,
      data: [
        [{ yAxis: 0 }, { yAxis: 40 }],
        [{ yAxis: 40 }, { yAxis: 70 }],
        [{ yAxis: 70 }, { yAxis: 100 }],
      ],
      itemStyle: {
        color: function (params) {
          return zoneColors[params.dataIndex] || zoneColors[0];
        },
        opacity: 0.06,
      },
    };
  }

  function renderHealthTrend(rows, mode) {
    const container = document.getElementById('widget-health-trend-body');
    if (!container) return;
    if (rows.length < 2) {
      container.innerHTML =
        '<div class="empty">Not enough history for a health timeline — need at least 2 commits.</div>';
      return;
    }
    const view = mode || 'overlay';
    const dates = rows.map(function (r) { return r.date; });
    const arch = rows.map(function (r) { return Number((r.arch_health || 0).toFixed(2)); });
    const code = rows.map(function (r) { return Number((r.code_health || 0).toFixed(2)); });
    const combined = rows.map(function (r) { return Number((r.combined_health || 0).toFixed(2)); });

    const okColor = token('--color-success') || '#16a34a';
    const warnColor = token('--color-warning') || '#ca8a04';
    const errColor = token('--color-error') || '#dc2626';
    const fgColor = getCssVar('--fg') || '#e6edf3';
    const dim = getCssVar('--fg-dim');
    const bands = healthTrendBands(errColor, warnColor, okColor);

    // Toggle button + chart host.
    container.innerHTML =
      '<div class="widget-toolbar"><button id="ht-toggle" class="toggle">' +
      (view === 'overlay' ? 'Split view' : 'Overlay view') +
      '</button></div><div id="ht-charts"></div>';
    const toggle = document.getElementById('ht-toggle');
    if (toggle) {
      toggle.onclick = function () {
        renderHealthTrend(rows, view === 'overlay' ? 'split' : 'overlay');
      };
    }
    const host = document.getElementById('ht-charts');

    const baseAxis = {
      tooltip: { trigger: 'axis' },
      grid: { left: 8, right: 8, top: 28, bottom: 28, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        boundaryGap: false,
        axisLabel: { color: dim, rotate: 30, fontSize: 10 },
      },
      yAxis: {
        type: 'value',
        min: 0,
        max: 100,
        axisLabel: { color: dim },
        splitLine: { lineStyle: { color: getCssVar('--border') } },
      },
    };

    if (view === 'overlay') {
      setChartAriaLabel(container,
        'Repo health timeline over ' + rows.length +
        ' sampled revisions: combined, architectural, and code health (0–100).');
      const chart = mountEcharts(host);
      chart.setOption(Object.assign({}, baseAxis, {
        legend: {
          data: ['Combined', 'Architectural', 'Code'],
          textStyle: { color: dim },
          bottom: 0,
        },
        series: [
          {
            name: 'Architectural',
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 5,
            data: arch,
            lineStyle: { color: okColor, width: 1.5, opacity: 0.7 },
            itemStyle: { color: okColor },
          },
          {
            name: 'Code',
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 5,
            data: code,
            lineStyle: { color: warnColor, width: 1.5, opacity: 0.7 },
            itemStyle: { color: warnColor },
          },
          {
            name: 'Combined',
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 6,
            data: combined,
            lineStyle: { color: fgColor, width: 3 },
            itemStyle: { color: fgColor },
            markArea: bands,
          },
        ],
      }));
      return;
    }

    // Split: three stacked small-multiples, same data, no recompute.
    const panels = [
      { label: 'Combined',       series: combined, color: fgColor },
      { label: 'Architectural',  series: arch,     color: okColor },
      { label: 'Code',           series: code,     color: warnColor },
    ];
    host.innerHTML = panels
      .map(function (p, i) { return '<div id="ht-sm-' + i + '" class="ht-sm"></div>'; })
      .join('');
    panels.forEach(function (p, i) {
      const el = document.getElementById('ht-sm-' + i);
      if (!el) return;
      const c = mountEcharts(el);
      c.setOption(Object.assign({}, baseAxis, {
        title: {
          text: p.label,
          left: 8,
          top: 4,
          textStyle: { fontSize: 12, color: getCssVar('--fg') },
        },
        grid: { left: 8, right: 8, top: 36, bottom: 24, containLabel: true },
        series: [
          {
            name: p.label,
            type: 'line',
            smooth: true,
            symbol: 'circle',
            symbolSize: 5,
            data: p.series,
            lineStyle: { color: p.color, width: 2 },
            itemStyle: { color: p.color },
            markArea: bands,
          },
        ],
      }));
    });
  }

  function renderArchMatrix(imports, roles) {
    roles = roles || [];
    const container = document.getElementById('widget-arch-matrix-body');
    if (!container) return;
    if (!imports.length) {
      container.innerHTML = '<div class="empty">No resolved import edges to matrix yet (Rust + Python + JS/TS).</div>';
      return;
    }
    // Same module roll-up + edge aggregation as the force graph.
    const archLayout = (window.Alpine && window.Alpine.store)
      ? window.Alpine.store('layout') : null;
    const userDepth = archLayout ? archLayout.archGraphDepth : 'auto';
    var edges = {};
    var nodes = {};
    var chosenDepth = (typeof userDepth === 'number') ? userDepth : 6;
    if (typeof userDepth === 'number') {
      const r = aggregateImportsAt(imports, userDepth);
      edges = r.edges;
      nodes = r.nodes;
    } else {
      for (var d = 2; d <= 6; d++) {
        const r = aggregateImportsAt(imports, d);
        edges = r.edges;
        nodes = r.nodes;
        chosenDepth = d;
        if (Object.keys(nodes).length >= 8) break;
      }
    }
    const mods = Object.keys(nodes);
    if (!mods.length) {
      container.innerHTML = '<div class="empty">All resolved imports stay intra-module — no inter-module matrix.</div>';
      return;
    }
    // Module layer = shallowest member file's topological level (from
    // architecture-roles). Modules with no level sink to the bottom.
    const moduleLevel = {};
    for (var ri = 0; ri < roles.length; ri++) {
      const m = modulePath(roles[ri].path, chosenDepth);
      if (!m || !nodes[m]) continue;
      const lv = (typeof roles[ri].level === 'number') ? roles[ri].level : 1e9;
      if (moduleLevel[m] === undefined || lv < moduleLevel[m]) moduleLevel[m] = lv;
    }
    // Order entry points first, foundations last → forward deps read
    // above the diagonal, back-edges (cycles) below it.
    const order = mods.slice().sort(function (a, b) {
      const la = (moduleLevel[a] === undefined) ? 1e9 : moduleLevel[a];
      const lb = (moduleLevel[b] === undefined) ? 1e9 : moduleLevel[b];
      return la - lb || (a < b ? -1 : (a > b ? 1 : 0));
    });
    const idxOf = {};
    order.forEach(function (m, i) { idxOf[m] = i; });
    const n = order.length;
    const labels = order.map(function (m) {
      return m.length > 24 ? '…' + m.slice(-23) : m;
    });
    const fwdColor = token('--color-info') || '#2563eb';
    const backColor = token('--color-error') || '#dc2626';
    var maxCount = 1;
    Object.keys(edges).forEach(function (k) { if (edges[k] > maxCount) maxCount = edges[k]; });
    const cells = [];
    var backEdges = 0;
    Object.keys(edges).forEach(function (k) {
      const parts = k.split('\x00');
      const r = idxOf[parts[0]]; // importer → row
      const c = idxOf[parts[1]]; // imported → col
      if (r === undefined || c === undefined) return;
      const count = edges[k];
      const isBack = r > c; // imports own-layer-or-shallower = cycle/back-edge
      if (isBack) backEdges += 1;
      cells.push({
        value: [c, r, count],
        itemStyle: {
          color: isBack ? backColor : fwdColor,
          // Forward edges fade by weight; back-edges stay loud (they're the
          // thing to notice). Floor kept high enough to read on the dark grid.
          opacity: isBack ? 0.95 : (0.5 + 0.45 * (count / maxCount)),
        },
      });
    });
    // Diagonal guide cells. No file imports itself, so every r==c cell is
    // empty — without a marker the eye has nothing to anchor the triangle to
    // (the caption's "triangular, all-blue = clean" is otherwise invisible).
    // value[2] = -1 lets the tooltip tell a guide cell from a real edge.
    for (var di = 0; di < n; di++) {
      cells.push({
        value: [di, di, -1],
        itemStyle: { color: getCssVar('--fg-dim') || '#888', opacity: 0.22 },
      });
    }
    setChartAriaLabel(container,
      'Dependency structure matrix, ' + n + ' modules ordered by architectural layer, ' +
      Object.keys(edges).length + ' dependency cells, ' + backEdges +
      ' below-diagonal back-edges (dependency cycles / layering violations) in red.');

    // Square cells so the diagonal is a true 45° line rather than the shallow
    // slope a full-width stretch produces. Reserve fixed margins for the
    // (rotated) labels, size the plot box to n×n equal cells, and centre it
    // when the panel is wider than the matrix.
    const cell = Math.max(11, Math.min(26, Math.round(620 / Math.max(n, 1))));
    const padTop = 128;   // rotated column labels
    const padLeft = 156;  // row labels
    const span = n * cell;
    const cw = container.clientWidth || 900;
    const gridLeft = Math.max(padLeft, Math.round((cw - span) / 2));
    container.style.height = (padTop + span + 10) + 'px';

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        position: 'top',
        formatter: function (p) {
          const c = p.value[0];
          const r = p.value[1];
          const v = p.value[2];
          if (r === c) return order[r] + '<br/><span style="opacity:.7">diagonal (self)</span>';
          return order[r] + ' &rarr; ' + order[c] + '<br/>' + v + ' import' + (v === 1 ? '' : 's') +
            (r > c ? '<br/><strong>back-edge — dependency cycle / layering violation</strong>' : '');
        },
      },
      grid: { left: gridLeft, top: padTop, width: span, height: span, containLabel: false },
      xAxis: {
        type: 'category', data: labels, position: 'top',
        axisTick: { show: false },
        axisLabel: { rotate: 55, fontSize: 9, color: getCssVar('--fg-dim'), margin: 8 },
        // Faint column banding ONLY — banding both axes cross-hatches into the
        // heavy checkerboard that drowns the data cells.
        splitArea: { show: true, areaStyle: { color: ['transparent', 'rgba(128,128,128,0.05)'] } },
      },
      yAxis: {
        type: 'category', data: labels, inverse: true,
        axisTick: { show: false },
        axisLabel: { fontSize: 9, color: getCssVar('--fg-dim'), margin: 8 },
        splitArea: { show: false },
      },
      series: [{
        type: 'heatmap',
        data: cells,
        label: { show: false },
        itemStyle: { borderColor: getCssVar('--bg'), borderWidth: 0.5 },
        emphasis: { itemStyle: { borderColor: getCssVar('--fg'), borderWidth: 1 } },
      }],
    });

    // Cross-widget selection: emphasise the selected file's row and column
    // in the DSM. The matrix axes are module-level prefixes (at `chosenDepth`
    // segments); the bus delivers full file paths, so we truncate via the same
    // `modulePath` helper before looking up in `idxOf`. The data array is
    // sparse, so we scan `cells` rather than assuming a dense row-major layout.
    // `value[0]` is the column (imported module), `value[1]` is the row
    // (importer module). A null selection, or a path outside the visible
    // modules, downplays everything back to neutral.
    window._codeloreRegisterSelectionListener('dsm', function (selectedPath) {
      chart.dispatchAction({ type: 'downplay' });
      if (!selectedPath) return;
      const mod = modulePath(selectedPath, chosenDepth);
      const idx = idxOf[mod];
      if (idx === undefined) return;
      const indices = [];
      for (var k = 0; k < cells.length; k++) {
        if (cells[k].value[0] === idx || cells[k].value[1] === idx) indices.push(k);
      }
      chart.dispatchAction({ type: 'highlight', seriesIndex: 0, dataIndex: indices });
    });
  }


  // ─── §12 Widget: calendar heatmap (commits per day) ──────────────

  function renderCalendarHeatmap(rows) {
    const container = document.getElementById('widget-calendar-heatmap-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No commit-activity data.</div>';
      return;
    }

    const data = rows.map(function (r) { return [r.date, r.count]; });
    // Single-pass min/max. `Math.min.apply(null, counts)` spreads every
    // element as an argument; on multi-year repos `counts` holds one
    // entry per active day (thousands), overflowing the call-stack arg
    // limit (RangeError). `rows.length` is guaranteed > 0 by the early
    // return above, so seeding from counts[0] is safe.
    const counts = rows.map(function (r) { return r.count; });
    let minVal = counts[0];
    let maxVal = counts[0];
    for (var ci = 1; ci < counts.length; ci++) {
      if (counts[ci] < minVal) minVal = counts[ci];
      if (counts[ci] > maxVal) maxVal = counts[ci];
    }

    // Determine which years to render — one calendar block per year
    // present in the data. Many heatmaps cap at one year; we want
    // multi-year history visible.
    const years = Array.from(new Set(rows.map(function (r) { return r.date.slice(0, 4); }))).sort();
    const calendars = years.map(function (y, idx) {
      return {
        range: y,
        // Top: 20 (was 30) drops the horizontal-top visualMap padding
        // — we moved the legend to the right-vertical axis. `right:
        // 130` reserves room for the vertical visualMap on the right
        // (its labels span ~110 px).
        top: 20 + idx * 110,
        cellSize: ['auto', 13],
        left: 70,
        right: 130,
        splitLine: { lineStyle: { color: getCssVar('--border') } },
        itemStyle: { color: 'transparent', borderColor: getCssVar('--border') },
        yearLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        monthLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        dayLabel: { color: getCssVar('--fg-dim'), fontSize: 10 },
      };
    });
    container.style.height = (30 + years.length * 110 + 20) + 'px';

    const series = years.map(function (y, idx) {
      return {
        type: 'heatmap',
        coordinateSystem: 'calendar',
        calendarIndex: idx,
        data: data.filter(function (d) { return d[0].startsWith(y); }),
      };
    });

    setChartAriaLabel(container,
      'Commit-activity calendar heatmap over ' + years.length + ' year(s), ' +
      data.length + ' active days');
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        formatter: function (params) {
          const date = params.value[0];
          const n = params.value[1];
          return '<b>' + escapeHtml(date) + '</b><br/>' +
            n + ' commit' + (n === 1 ? '' : 's');
        },
      },
      // Vertical right-side legend. The horizontal-top placement
      // collided with each calendar's month labels (Jan / Feb / …)
      // — they share the same y-band, so "May 1.0-5.4 5.4-9.8 Jun"
      // crashed into "May" / "Jun". Vertical on the right is the
      // GitHub contributions-graph idiom and stays clear of every
      // month label.
      visualMap: {
        // A degenerate range (every active day shares one commit count)
        // collapses the piecewise bands and renders cells near-invisible.
        // Anchor the low end at 0 so the single value still paints a
        // visible top-band shade.
        min: minVal === maxVal ? 0 : minVal,
        max: maxVal,
        type: 'piecewise',
        orient: 'vertical',
        right: 12,
        top: 'middle',
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 10 },
        itemWidth: 12,
        itemHeight: 12,
        itemGap: 6,
        inRange: {
          color: [
            token('--heatmap-1'),
            token('--heatmap-2'),
            token('--heatmap-3'),
            token('--heatmap-4'),
            token('--heatmap-5'),
          ],
        },
      },
      calendar: calendars,
      series: series,
    });
  }


  // ─── §13 Widget: X-Ray sunburst (function-level drill-down) ─────

  function renderXRaySunburst(rows) {
    const container = document.getElementById('widget-xray-sunburst-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No function-level data. ' +
        'Add Tier-1 source files to the repo or enable the `spa` feature ' +
        'on a build that has tree-sitter language support.</div>';
      return;
    }

    // Build a hierarchy: top-level path segment → file → function.
    // First pass collects leaves; second pass assigns per-leaf
    // `itemStyle.color` driven by cognitive complexity (yellow→red
    // ramp via the shared `heatmapColor` helper). Container nodes
    // (top dir, file) keep depth-based shading so the wedge boundary
    // stays visible against the heatmap.
    const root = { name: 'all', children: [] };
    let maxCognitive = 0;
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      const segs = (r.path || '').split('/').filter(Boolean);
      const top = segs[0] || '<root>';
      const file = segs.slice(1).join('/') || r.path;
      let topNode = root.children.find(function (c) { return c.name === top; });
      if (!topNode) {
        topNode = { name: top, children: [] };
        root.children.push(topNode);
      }
      let fileNode = topNode.children.find(function (c) { return c.name === file; });
      if (!fileNode) {
        fileNode = { name: file, children: [], fullPath: r.path };
        topNode.children.push(fileNode);
      }
      const cog = typeof r.cognitive === 'number' ? r.cognitive : 0;
      if (cog > maxCognitive) maxCognitive = cog;
      fileNode.children.push({
        name: r.function || '<anonymous>',
        value: cog,
        cognitive: cog,
        startLine: r.start_line,
        endLine: r.end_line,
        fullPath: r.path,
      });
    }
    // Assign per-leaf colour by `cognitive / maxCognitive`. Anchored
    // to 1 to avoid div-by-zero on degenerate fixtures where every
    // function has cognitive complexity 0.
    const cogScale = maxCognitive || 1;
    for (var ti = 0; ti < root.children.length; ti++) {
      const topNode = root.children[ti];
      for (var fi = 0; fi < topNode.children.length; fi++) {
        const fileNode = topNode.children[fi];
        for (var fni = 0; fni < fileNode.children.length; fni++) {
          const fn = fileNode.children[fni];
          fn.itemStyle = { color: heatmapColor(fn.cognitive / cogScale) };
        }
      }
    }

    setChartAriaLabel(container,
      'X-Ray complexity sunburst of ' + rows.length + ' functions across ' +
      root.children.length + ' top-level paths, coloured by cognitive complexity');
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        formatter: function (params) {
          const d = params.data || {};
          if (d.cognitive != null) {
            return '<b>' + escapeHtml(d.fullPath) + '</b><br/>' +
              'function <code>' + escapeHtml(d.name) + '</code><br/>' +
              'cognitive: ' + d.cognitive.toFixed(0) + '<br/>' +
              'lines ' + d.startLine + '–' + d.endLine;
          }
          return '<b>' + escapeHtml(d.name) + '</b>';
        },
      },
      series: [{
        type: 'sunburst',
        data: root.children,
        radius: ['0%', '90%'],
        nodeClick: 'rootToNode',
        emphasis: { focus: 'ancestor' },
        // Container shading carries the visual hierarchy: ring 1 (top
        // path segment) is dark, ring 2 (file) is mid. Ring 3 (function
        // leaves) is overridden per-node by the heatmap colour assigned
        // above — the empty entry below disables the default ramp.
        // Container-ring fills + label colors pulled from CSS vars so
        // they swap on theme toggle (the `_codeloreRerenderers` registry
        // re-runs this widget when `$store.theme.isDark` flips, which
        // re-evaluates the getCssVar calls against the new theme). The
        // leaf-ring label color stays dark across themes because it sits
        // on the saturated heatmap (yellow→red) where light text would
        // drop below WCAG AA contrast on the yellow end.
        levels: [
          {},
          {
            itemStyle: { color: getCssVar('--xray-ring-1') },
            label: { color: getCssVar('--xray-ring-label'), fontSize: 11 },
          },
          {
            itemStyle: { color: getCssVar('--xray-ring-2') },
            label: { color: getCssVar('--xray-ring-label'), fontSize: 10 },
          },
          { label: { color: getCssVar('--xray-leaf-label'), fontSize: 9 } },
        ],
      }],
    });

    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.fullPath && d.cognitive != null) {
        if (window._codeloreShowDetail) {
          window._codeloreShowDetail(d.fullPath);
        } else {
          showFileDetailDrawer(d.fullPath, data);
        }
      }
    });

  }


  // ─── §14 Controls: hotspot color-mode toggles ────────────────────
  //
  // Theme toggle itself is owned by Alpine (`$store.theme.isDark`
  // registered in template.html). The `Alpine.effect` there flips
  // `<html data-theme>` AND fires every callback in
  // `window._codeloreRerenderers`, so this file just appends to that
  // registry — no theme-toggle init function lives here.

  function initHotspotColorToggles() {
    // Gap #4 migration: button row → DaisyUI `tabs tabs-boxed`. The
    // selector matches `[role="tab"]` so the same handler works on
    // both the old class-`toggle` markup (if any cached SPA HTML is
    // still in the wild during the cut-over) and the new tab markup.
    const bar = document.getElementById('hotspot-color-toggles');
    if (!bar) return;
    const buttons = bar.querySelectorAll('button[role="tab"], button.toggle');
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].addEventListener('click', function (evt) {
        const mode = evt.currentTarget.getAttribute('data-mode');
        // Update active state — `tab-active` for DaisyUI tabs and
        // `active` for any legacy markup. Both classes are written
        // as complete literals so the Tailwind v4 `@source` scanner
        // sees them (otherwise dynamic suffixes drop out of the bundle).
        // `aria-selected` mirrors the visual state for the WAI-ARIA
        // tabs pattern — without it screen readers see every tab as
        // equally focusable but none as "selected" (F136).
        for (var j = 0; j < buttons.length; j++) {
          const isCurrent = (buttons[j] === evt.currentTarget);
          buttons[j].classList.toggle('tab-active', isCurrent);
          buttons[j].classList.toggle('active', isCurrent);
          buttons[j].setAttribute('aria-selected', isCurrent ? 'true' : 'false');
        }
        // Wrap the re-render in startViewTransition so the colour-
        // mode swap smoothly crossfades. On unsupported
        // browsers the wrapper is a synchronous no-op and the
        // re-render runs identically. Updating `currentHotspotColorMode`
        // inside the callback ensures the theme-toggle re-render
        // path sees the active mode too.
        startViewTransition(function () {
          currentHotspotColorMode = mode;
          // Leaving bivariate hides the legend — drop any active quadrant
          // brush so the map isn't left dimmed with no legend to clear it.
          if (mode !== 'bivariate') {
            const bs = window.Alpine && window.Alpine.store && window.Alpine.store('brush');
            if (bs && bs.cell) bs.clear();
          }
          renderHotspotCirclePack(data.hotspots || [], mode);
        });
      });
    }
  }


  // ─── §15 Utility helpers ──────────────────────────────────────────

  // Build the path hierarchy. Input: HotspotRow[]. Output:
  // { name: 'root', children: [{name, children?, metrics?, fullPath?}] }
  // where leaves carry the metrics object.
  function buildFsHierarchy(rows) {
    const root = { name: 'root', children: [] };
    for (var i = 0; i < rows.length; i++) {
      const row = rows[i];
      const parts = (row.path || '').split('/').filter(Boolean);
      var node = root;
      var acc = '';
      for (var j = 0; j < parts.length; j++) {
        const segment = parts[j];
        acc = acc ? (acc + '/' + segment) : segment;
        var child = node.children && node.children.find(function (c) { return c.name === segment; });
        if (!child) {
          child = { name: segment, fullPath: acc };
          if (j === parts.length - 1) {
            child.metrics = {
              revisions: row.revisions,
              cognitive: row.cognitive,
              code_health: row.code_health,
              hotspot_score: row.hotspot_score,
            };
          } else {
            child.children = [];
          }
          if (!node.children) node.children = [];
          node.children.push(child);
        }
        node = child;
      }
    }
    return root;
  }

  // Interpolated yellow → red ramp for cognitive complexity.
  // ratio ∈ [0, 1].
  function heatmapColor(ratio) {
    const r = 255;
    const g = Math.round(255 * (1 - ratio * 0.85));
    const b = Math.round(50 * (1 - ratio));
    return 'rgb(' + r + ',' + g + ',' + b + ')';
  }

  // Pick the primary author per file: the one with the most added
  // LoC. Stable across re-renders (Object.values walks in insertion
  // order; ties broken by first-occurrence).
  function computePrimaryAuthorByPath(rows) {
    const byPath = {};
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      const cur = byPath[r.entity];
      if (!cur || r.added > cur.added) {
        byPath[r.entity] = { author: r.author, added: r.added };
      }
    }
    const out = {};
    Object.keys(byPath).forEach(function (p) { out[p] = byPath[p].author; });
    return out;
  }

  // Unique author list for the offboarding picker.
  // Uses entity_ownership (one row per (path, author) tuple). Sorted
  // for stable rendering across reloads.
  function computeUniqueAuthors(entityOwnership) {
    const set = new Set();
    for (var i = 0; i < entityOwnership.length; i++) {
      const author = entityOwnership[i].author;
      if (author) set.add(author);
    }
    return Array.from(set).sort();
  }

  // Coupling arc overlay helpers.

  // Quadratic Bezier path from (x1,y1) to (x2,y2) with the control
  // point offset perpendicular to the chord. curveness ∈ [0, 1];
  // 0 = straight line, 0.25 = the CodeScene-equivalent gentle arc.
  // Returned in SVG path syntax for ECharts' `type: 'path'` shape.
  function arcPath(x1, y1, x2, y2, curveness) {
    const mx = (x1 + x2) / 2;
    const my = (y1 + y2) / 2;
    const dx = x2 - x1;
    const dy = y2 - y1;
    const cx = mx + (-dy * curveness);
    const cy = my + (dx * curveness);
    return 'M ' + x1 + ',' + y1 + ' Q ' + cx + ',' + cy + ' ' + x2 + ',' + y2;
  }

  // Build the arc descriptors for the file the user clicked. Returns
  // an array of {x1, y1, x2, y2, opacity, lineWidth} — empty array
  // means "no overlay" (renders as a silent series). Per Gap #2
  // accepted: top-5 by Fisher significance (lower p = more
  // significant = drawn first). Width encodes coupling degree so
  // strongly-coupled pairs read thicker — two extra perceptual
  // dimensions on a primitive CodeScene leaves flat.
  function buildCouplingArcs(filePath, nodePositions, couplingRows) {
    if (!filePath || !nodePositions || !couplingRows || !couplingRows.length) {
      return [];
    }
    const selfPos = nodePositions.get(filePath);
    if (!selfPos) return [];
    // Filter to rows that touch filePath; sort by p ASC; take top-5.
    const matching = [];
    for (var i = 0; i < couplingRows.length; i++) {
      const r = couplingRows[i];
      if (r.entity_a === filePath || r.entity_b === filePath) matching.push(r);
    }
    matching.sort(function (a, b) {
      const pa = (typeof a.fisher_p === 'number') ? a.fisher_p : 1;
      const pb = (typeof b.fisher_p === 'number') ? b.fisher_p : 1;
      return pa - pb;
    });
    const top = matching.slice(0, 5);
    const arcs = [];
    for (var k = 0; k < top.length; k++) {
      const row = top[k];
      const peer = (row.entity_a === filePath) ? row.entity_b : row.entity_a;
      const peerPos = nodePositions.get(peer);
      if (!peerPos) continue;
      // Opacity ← (1 - fisher_p). p=0.001 → ~1.0, p=0.05 → 0.95.
      // Floor at 0.4 so even weakly-significant arcs remain visible.
      const opacity = Math.max(0.4, 1.0 - (row.fisher_p || 0));
      // Width ← coupling degree (% co-change). Cap at 7px so a
      // 100%-coupled pair doesn't overwhelm a 20%-coupled one.
      const lineWidth = 1 + Math.min(6, (row.degree || 0) / 8);
      arcs.push({
        x1: selfPos.x, y1: selfPos.y,
        x2: peerPos.x, y2: peerPos.y,
        opacity: opacity,
        lineWidth: lineWidth,
        // Partner identity + strength, so the map can emphasise the peer
        // circle and name it (the arc itself stays silent — clicks fall
        // through to the circles — so identity rides on the leaf, not the line).
        peer: peer,
        degree: row.degree,
        shared: row.shared,
      });
    }
    return arcs;
  }

  // Partial setOption update: refresh only the arc series (series[1])
  // without touching the circle-pack series[0]. Avoids the d3.pack
  // re-layout on every click. The `{}` for series[0] tells ECharts
  // "no changes here" via index-based merge.
  function updateCouplingArcs() {
    if (!lastHotspotChart || !lastHotspotNodePositions) return;
    if (lastHotspotChart.isDisposed && lastHotspotChart.isDisposed()) return;
    const arcs = buildCouplingArcs(
      selectedCouplingFile,
      lastHotspotNodePositions,
      data.coupling || []
    );
    // Mutate the module-scoped `arcData` array in place so the arc
    // renderItem's closure-captured reference keeps pointing at live
    // data. Reassigning would orphan the closure and the click flow
    // would silently render stale arcs.
    arcData.length = 0;
    for (var ai = 0; ai < arcs.length; ai++) {
      const a = arcs[ai];
      arcData.push({ value: [a.x1, a.y1], _arc: a });
    }
    // Emphasise the selected file + its coupling partners as outlined circles
    // so it is clear WHICH files are coupled, not merely that arcs exist. The
    // partner circles keep their normal leaf tooltip (name + metrics on hover).
    const peers = new Set();
    for (var pi = 0; pi < arcs.length; pi++) peers.add(arcs[pi].peer);
    if (lastCirclePackData) {
      for (var ci = 0; ci < lastCirclePackData.length; ci++) {
        const it = lastCirclePackData[ci];
        if (!it || !it._raw || !it.metrics) continue;
        it._raw.couplingSelected =
          !!selectedCouplingFile && it.fullPath === selectedCouplingFile;
        it._raw.couplingPeer = peers.has(it.fullPath);
      }
    }
    lastHotspotChart.setOption({
      series: [
        lastCirclePackData ? { data: lastCirclePackData } : {},
        { data: arcData },
      ],
    });
  }

  // Partial setOption update: re-tint per-leaf opacity to emphasise the
  // brushed bivariate quadrant (members bright, non-members dimmed), leaving
  // the arc series[1] untouched (`{}` = no change). Mirrors updateCouplingArcs'
  // index-merge pattern. `brushedPaths === null` restores default leaf opacity.
  function updateHotspotBrush() {
    if (!lastHotspotChart || !lastCirclePackData) return;
    if (lastHotspotChart.isDisposed && lastHotspotChart.isDisposed()) return;
    for (var i = 0; i < lastCirclePackData.length; i++) {
      const item = lastCirclePackData[i];
      if (!item || !item._raw || !item.metrics) continue; // leaves only
      item._raw.opacity = !brushedPaths
        ? 0.85
        : (brushedPaths.has(item.fullPath) ? 0.95 : 0.12);
    }
    lastHotspotChart.setOption({ series: [{ data: lastCirclePackData }, {}] });
  }

  // Stable palette assignment for author colors. A discrete categorical
  // palette tuned for dark-background readability; cycles if there are
  // more authors than colors.
  function makeAuthorPalette(authors) {
    // F132: palette read from CSS custom properties so light theme
    // gets a deeper-saturation set that's readable on white cards.
    // The 15-token chain mirrors the previous hard-coded array;
    // `token()` is theme-aware and cache-invalidated on rerender.
    const palette = [
      token('--chart-palette-1'),  token('--chart-palette-2'),  token('--chart-palette-3'),
      token('--chart-palette-4'),  token('--chart-palette-5'),  token('--chart-palette-6'),
      token('--chart-palette-7'),  token('--chart-palette-8'),  token('--chart-palette-9'),
      token('--chart-palette-10'), token('--chart-palette-11'), token('--chart-palette-12'),
      token('--chart-palette-13'), token('--chart-palette-14'), token('--chart-palette-15'),
    ];
    const sorted = authors.slice().sort();
    const out = {};
    for (var i = 0; i < sorted.length; i++) {
      out[sorted[i]] = palette[i % palette.length];
    }
    return out;
  }
})();
