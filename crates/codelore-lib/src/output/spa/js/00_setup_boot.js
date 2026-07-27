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
  //           bandLeafColor · heatRamp
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
    // `[id$="-chart-host"]` also reaches a chart mounted on a nested host
    // inside a widget body (the DSM matrix's `#wam-chart-host`), which the
    // `-body` selectors alone miss. `getInstanceByDom` returns null for the
    // wrapping `-body` in that case, so there is no double-resize.
    const bodies = root.querySelectorAll(
      '.widget-body, [id$="-body"], [id$="-chart-host"]',
    );
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

  // ─── §3b  Sticky section nav: scrollspy + jump links + back-to-top ──
  // `#dash-nav`'s chips (template.html, sibling of `<header>`) and the
  // four factor tiles (`renderFactorHeader`, 10_helpers_drawer.js) both
  // jump to a `.dash-group` section through this one function.
  function dashPrefersReducedMotion() {
    return typeof window.matchMedia === 'function'
      && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  }

  // NEVER reads or writes `location.hash` — the SPA owns the hash as its
  // state serializer (`readUrlIntoStores`/`writeStoresToUrl`, further
  // down in template.html) and anchor-style navigation would corrupt
  // it. The sticky nav's height offset is handled by `scroll-margin-top`
  // on `.dash-group` (hand-written CSS in the inline `<style>` block),
  // not JS math.
  function scrollToDashSection(targetId) {
    const el = document.getElementById(targetId);
    if (!el) return;
    el.scrollIntoView({
      behavior: dashPrefersReducedMotion() ? 'auto' : 'smooth',
      block: 'start',
    });
  }

  // Wires the nav chips (click + scrollspy highlight) and the
  // back-to-top button. Called once at boot end — the markup it binds
  // to (`#dash-nav`, `.dash-group`, `#dash-top-btn`) is static template
  // HTML, present regardless of dashboard data.
  function initDashNav() {
    const nav = document.getElementById('dash-nav');
    if (!nav) return;
    const chips = nav.querySelectorAll('.dash-nav-chip');

    function setActiveChip(targetId) {
      for (let i = 0; i < chips.length; i++) {
        const isMatch = chips[i].getAttribute('data-target') === targetId;
        chips[i].classList.toggle('dash-active', isMatch);
      }
    }

    // Click: highlight immediately — deterministic feedback that
    // doesn't wait on the scroll animation to settle — then scroll.
    // The IntersectionObserver below keeps the highlight in sync during
    // ordinary free-scrolling.
    for (let i = 0; i < chips.length; i++) {
      const target = chips[i].getAttribute('data-target');
      chips[i].addEventListener('click', function () {
        setActiveChip(target);
        scrollToDashSection(target);
      });
    }

    // Scrollspy: one observer over all six sections. The `-40% / -55%`
    // margins shrink the intersection root to a thin horizontal band
    // roughly at reading height, so a section is only "active" once
    // it's the one the user is actually looking at — not merely
    // partially visible at the very top or bottom of the viewport.
    const groups = [];
    for (let i = 0; i < chips.length; i++) {
      const el = document.getElementById(chips[i].getAttribute('data-target'));
      if (el) groups.push(el);
    }
    if (groups.length && typeof IntersectionObserver === 'function') {
      const observer = new IntersectionObserver(function (entries) {
        for (let i = 0; i < entries.length; i++) {
          if (entries[i].isIntersecting) setActiveChip(entries[i].target.id);
        }
      }, { rootMargin: '-40% 0px -55% 0px' });
      for (let i = 0; i < groups.length; i++) observer.observe(groups[i]);
    }

    // Back-to-top: appears once the user has scrolled past 600px;
    // click scrolls to the document top (no `.dash-group` involved, so
    // no `scroll-margin-top` offset applies here).
    const topBtn = document.getElementById('dash-top-btn');
    if (topBtn) {
      topBtn.addEventListener('click', function () {
        window.scrollTo({ top: 0, behavior: dashPrefersReducedMotion() ? 'auto' : 'smooth' });
      });
      window.addEventListener('scroll', function () {
        topBtn.classList.toggle('dash-visible', window.scrollY > 600);
      }, { passive: true });
    }
  }

  // Wires each section heading's `.dash-collapse` chevron (template.html)
  // to toggle `.dash-collapsed` on its `.dash-group` ancestor, hiding the
  // section's `.dash-group-grid` via hand-written CSS. Collapse state is
  // NEVER persisted — every section renders expanded at load, so ECharts
  // instances never mount inside a hidden container. Expanding re-runs the
  // existing `resizeAllEchartsIn` sweep over just that section, recovering
  // correct layout for any chart that was resized (e.g. by a fullscreen
  // toggle, window resize, or theme rerender) while its section was
  // collapsed.
  function initDashCollapse() {
    const buttons = document.querySelectorAll('.dash-collapse');
    for (let i = 0; i < buttons.length; i++) {
      const btn = buttons[i];
      btn.addEventListener('click', function () {
        const group = btn.closest('.dash-group');
        if (!group) return;
        const collapsed = group.classList.toggle('dash-collapsed');
        btn.setAttribute('aria-expanded', collapsed ? 'false' : 'true');
        if (!collapsed) resizeAllEchartsIn(group);
      });
    }
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

  // Wire a `role="tree"` for the WAI-ARIA Tree View keyboard pattern
  // (the hotspot file list — template.html's parallel DOM tree next to
  // the circle-pack canvas). Single-level, so only the "move focus"
  // behaviors apply — no Left/Right expand/collapse. Two deliberate
  // differences from `wireTablistArrows` above:
  //   - Arrow keys only MOVE FOCUS; they don't activate. Per the
  //     WAI-ARIA APG treeview pattern, activation is Enter/Space,
  //     which template.html already wires inline against
  //     `_codeloreShowDetail` — this handler doesn't duplicate that.
  //   - Navigation does NOT wrap at the ends (Down Arrow on the last
  //     node — and Up Arrow on the first — does nothing further),
  //     unlike the tablist's wraparound.
  // Roving tabindex still applies (exactly one treeitem is a tab
  // stop), but the INITIAL state is set by the tree's Alpine
  // `:tabindex="fileIdx === 0 ? '0' : '-1'"` binding rather than here
  // — the list is `x-for`-generated against `$store.dashboard.hotspots`,
  // which is populated after this boot script's synchronous body runs,
  // so there may be zero treeitems in the DOM at wire time. That's
  // fine: `items()` re-queries the DOM live on every keydown, and the
  // listener is attached to the tree container itself so it keeps
  // working once Alpine renders the rows.
  function wireTreeArrows(treeEl) {
    function items() {
      return Array.prototype.slice.call(
        treeEl.querySelectorAll('[role="treeitem"]')
      );
    }
    function syncRovingTabindex(activeIdx) {
      const list = items();
      for (var i = 0; i < list.length; i++) {
        list[i].setAttribute('tabindex', i === activeIdx ? '0' : '-1');
      }
    }
    function focusItem(idx) {
      const list = items();
      const target = list[idx];
      if (!target) return;
      target.focus();
      syncRovingTabindex(idx);
    }
    treeEl.addEventListener('keydown', function (evt) {
      const list = items();
      const current = list.indexOf(document.activeElement);
      if (current < 0) return;
      var next = null;
      if (evt.key === 'ArrowDown') {
        next = Math.min(current + 1, list.length - 1);
      } else if (evt.key === 'ArrowUp') {
        next = Math.max(current - 1, 0);
      } else if (evt.key === 'Home') {
        next = 0;
      } else if (evt.key === 'End') {
        next = list.length - 1;
      }
      if (next === null || next === current) return;
      evt.preventDefault();
      focusItem(next);
    });
  }
  function wireAllTrees() {
    const trees = document.querySelectorAll('[role="tree"]');
    for (var i = 0; i < trees.length; i++) {
      wireTreeArrows(trees[i]);
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
  // Arrow-key navigation + roving tabindex for every `role="tree"`
  // (the hotspot file list's keyboard-accessible DOM tree).
  wireAllTrees();


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
      formula: 'percentile_rank(revisions) × percentile_rank(cognitive) × (100 − cognitive_health) / 4. Range [0, 10] (CodeScene convention).',
      citation: { label: 'Tornhill 2018 — Software Design X-Rays', anchor: '#hotspots-' },
    },
    revisions: {
      formula: 'Count of distinct commits touching the file in the analysed history, after time-bucket and lineage rewrites.',
      citation: { label: 'Revisions analysis', anchor: '#revisions-' },
    },
    cognitive_health: {
      formula: 'Hotspot-table Cognitive Health column: the hotspots analysis\'s own inline signal, 100 × (1 − 0.40 × normalize(cognitive)), empirical range [60, 100]; lower = more cognitively complex. (Distinct from the code-health composite score.)',
      citation: { label: 'hotspots inline cognitive-health', anchor: '#hotspots-' },
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

  // ─── §3c  Guided tour state ──────────────────────────────────────
  //
  // A 4-step martini-glass walk over the hero circle-pack. Each step
  // sets a color mode and publishes a brush so other widgets highlight
  // the same paths. Tour state is ephemeral (not persisted across
  // reloads); `tourStep = -1` means the tour is inactive (free-form).
  //
  // Color-mode mapping (browser tab data-mode values):
  //   health    → 'health'    (code-health band view)
  //   activity  → 'cognitive' (complexity/churn; closest to "hotspot activity")
  //   effort    → 'friction'  (friction heat-ramp; effort-weighted churn)
  //   targets   → 'health'    (re-use health lens; top-10 refactoring targets brushed)
  //
  // The 'effort' and 'targets' modes are not distinct circle-pack color
  // modes today — they re-use the closest existing mode and distinguish
  // themselves via the brush set and the step note.
  var tourStep = -1; // -1 = inactive

  var TOUR_STEPS = [
    {
      title: 'Code health',
      lens: 'health',
      note: 'Circle color shows the code-health band (green/yellow/red). ' +
            'Large red circles combine high churn with poor health — prime refactoring candidates.',
    },
    {
      title: 'Hotspots',
      lens: 'cognitive',
      note: 'Color shifts to cognitive complexity. Files that are both large (circle size = revisions) ' +
            'and cognitively complex (dark red) accumulate the most defect risk.',
    },
    {
      title: 'Effort in red',
      lens: 'friction',
      note: 'Friction heat-ramp: the warmest circles absorb the most churn relative to their health. ' +
            'These are the files where effort is being wasted on unhealthy code.',
    },
    {
      title: 'Refactoring targets',
      lens: 'health',
      // Brushes the top-10 refactoring targets (data.refactoring_targets,
      // ranked by return-on-investment: risk ÷ inspection effort). Falls back
      // to the top-10 hotspots by score when that field is absent.
      note: 'Top-10 refactoring targets are brushed across all widgets. ' +
            'These are ranked by return-on-investment — structural risk × churn, ' +
            'divided by inspection effort — so small, dense, unhealthy files rise to the top.',
      brushRefactoringTargets: true,
    },
  ];

  // Apply one tour step: switch color mode, publish brush, show/hide banner.
  function applyTourStep(idx) {
    var step = TOUR_STEPS[idx];
    if (!step) return;

    // 1. Switch the circle-pack color mode. Re-uses the same path as
    //    the manual color-toggle tabs (§14): update module state + re-render.
    //    Also update the tab UI so it stays in sync with the tour.
    var bar = document.getElementById('hotspot-color-toggles');
    if (bar) {
      var buttons = bar.querySelectorAll('button[role="tab"], button.toggle');
      for (var i = 0; i < buttons.length; i++) {
        var isCurrent = (buttons[i].getAttribute('data-mode') === step.lens);
        buttons[i].classList.toggle('tab-active', isCurrent);
        buttons[i].classList.toggle('active', isCurrent);
        buttons[i].setAttribute('aria-selected', isCurrent ? 'true' : 'false');
      }
    }
    currentHotspotColorMode = step.lens;
    renderHotspotCirclePack(data.hotspots || [], step.lens);

    // 2. Publish brush — the real refactoring targets for the "targets"
    //    step, empty otherwise. `data.refactoring_targets` is pre-sorted by
    //    priority (return-on-investment: risk ÷ effort) DESC in the builder,
    //    so its first 10 are the highest-ROI candidates — a genuinely
    //    different ordering from raw hotspot score. When the field is absent
    //    (older payloads, or the code-health composite was unavailable), fall
    //    back to the top-10 hotspots by score so the step still highlights
    //    something rather than breaking the tour.
    if (window.Alpine && window.Alpine.store) {
      var bs = window.Alpine.store('brush');
      if (bs) {
        if (step.brushRefactoringTargets) {
          var top10 = (data.refactoring_targets || [])
            .slice(0, 10)
            .map(function (r) { return r.path; });
          if (!top10.length) {
            top10 = (data.hotspots || [])
              .slice()
              .sort(function (a, b) {
                return ((b.hotspot_score || 0) - (a.hotspot_score || 0));
              })
              .slice(0, 10)
              .map(function (r) { return r.path; });
          }
          // Publish as a synthetic brush cell so all brush listeners fire.
          bs.set(['targets', 'top10'], top10);
        } else {
          bs.clear();
        }
      }
    }

    // 3. Update the tour stepper UI.
    renderGuidedTour();
  }

  // Exit the tour: clear brush, restore bivariate mode, hide the banner.
  function exitTour() {
    tourStep = -1;
    if (window.Alpine && window.Alpine.store) {
      var bs = window.Alpine.store('brush');
      if (bs) bs.clear();
    }
    currentHotspotColorMode = 'bivariate';
    renderHotspotCirclePack(data.hotspots || [], 'bivariate');
    // Restore bivariate tab as active.
    var bar = document.getElementById('hotspot-color-toggles');
    if (bar) {
      var buttons = bar.querySelectorAll('button[role="tab"], button.toggle');
      for (var i = 0; i < buttons.length; i++) {
        var isCurrent = (buttons[i].getAttribute('data-mode') === 'bivariate');
        buttons[i].classList.toggle('tab-active', isCurrent);
        buttons[i].classList.toggle('active', isCurrent);
        buttons[i].setAttribute('aria-selected', isCurrent ? 'true' : 'false');
      }
    }
    renderGuidedTour();
  }

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
  //
  // Order matches the dashboard's section order (template.html):
  // Overview, Hotspots & Risk, Code Health, Architecture, Knowledge,
  // Delivery — each section's widgets in the order they appear on the
  // page, top to bottom. `factor-header` stays first since it renders
  // synchronously before the cooperative boot loop below yields between
  // the rest. Paint order is otherwise independent of DOM order (every
  // renderer targets its widget by element id), so this is purely a
  // "first thing the user sees is the first thing that paints" ordering.
  const WIDGETS = [
    { name: 'factor-header',      rerender: 'theme', render: () => renderFactorHeader(data.factors || [], data.options || {}) },
    { name: 'kpi-tiles',          rerender: false, render: () => renderKpiTiles(data) },
    { name: 'guided-tour',        rerender: false, render: () => renderGuidedTour() },
    { name: 'hotspot-circle-pack', rerender: 'theme', render: () => renderHotspotCirclePack(data.hotspots || [], currentHotspotColorMode) },
    { name: 'hotspot-table',      rerender: false, render: () => renderHotspotTable(data.hotspots || []) },
    { name: 'hotspot-treemap',    rerender: 'theme', render: () => renderHotspotTreemap(data.hotspots || []) },
    { name: 'xray-sunburst',      render: () => renderXRaySunburst(data.xray || []) },
    { name: 'health-trend',       rerender: 'theme', render: () => renderHealthTrend(data.health_trend || []) },
    { name: 'trends',             render: () => renderTrends(data.trends || []) },
    { name: 'share-bars',         rerender: false, render: () => renderShareBars(data.effort_exposure || [], data.options || {}) },
    { name: 'improvements-feed',  rerender: false,   render: () => renderImprovementsFeed(data.health_transitions || []) },
    { name: 'cognitive-boxplot',  rerender: 'theme', render: () => renderCognitiveBoxplot(data.hotspots || []) },
    { name: 'parallel-coords',    rerender: 'theme', render: () => renderParallelCoords(data.hotspots || []) },
    { name: 'arch-graph',         rerender: 'theme', render: () => renderArchGraph(data.imports || [], data.modularity_violations || [], data.unstable_interface || [], data.architecture_roles || []) },
    { name: 'arch-matrix',        rerender: 'theme', render: () => renderArchMatrix(data.imports || [], data.architecture_roles || [], data.coupling || []) },
    { name: 'arch-trend',         rerender: 'theme', render: () => renderArchTrend(data.architecture_trend || []) },
    { name: 'module-chord',       render: () => renderModuleChord(data.coupling || []) },
    { name: 'coupling-sankey',    rerender: 'theme', render: () => renderCouplingSankey(data.coupling || []) },
    { name: 'knowledge-surfaces', rerender: false, render: () => renderKnowledgeSurfaces(data.code_familiarity || [], data.team_composition || [], data.coordination_needs || []) },
    { name: 'knowledge-islands',  rerender: false, render: () => renderKnowledgeIslands(data.knowledge_islands || []) },
    { name: 'delivery-card',      rerender: false,   render: () => renderDeliveryCard(data) },
    { name: 'kamei-risk-sparkline', rerender: 'theme', render: () => renderKameiRiskSparkline(data.kamei_risk || []) },
    { name: 'calendar-heatmap',   rerender: 'theme', render: () => renderCalendarHeatmap(data.daily_commits || []) },
  ];

  // F97: boot widgets cooperatively. The synchronous `forEach` blocked
  // first paint until all 14 widgets had run their initial render
  // (ECharts mount + d3.pack layout + initial DOM injection each
  // costs tens of ms on large repos). Now: render the first widget
  // synchronously so the user sees SOMETHING immediately, then yield
  // between each subsequent widget so the browser can paint progress.
  // The theme/regular rerender registration is unchanged (those
  // rerenderers still fire as a single batch on theme toggle, yielding
  // between them via _codeloreYieldToMain).
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
      // yield is a wasted task). The first widget (factor-header) is
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
            cognitive_health: r.cognitive_health,
            hotspot_score: r.hotspot_score,
            primary_author: listPrimaryAuthorByPath[r.path] || null,
          };
        });
      dashboardStore.hotspots = sorted;
    }
  }

  // Ownership-cap note. When the entity-ownership embed was capped to the
  // top-N hotspot files (data.entity_ownership_cap present), the knowledge-map
  // lens, off-boarding picker, and drawer contributor lists cover only those
  // files — say so rather than silently omitting the rest. No Alpine
  // dependency: a plain DOM write, so it works on the fallback boot path too.
  (function () {
    const note = document.getElementById('ownership-cap-note');
    if (!note) return;
    const cap = data.entity_ownership_cap;
    if (typeof cap === 'number' && cap > 0) {
      note.textContent =
        'Contributor & ownership data is limited to the ' + cap +
        ' most-active files to keep this report self-contained; ' +
        'lower-ranked files show no author colour or contributor list.';
      note.hidden = false;
    }
  })();

  // Static markup (`#dash-nav` chips, `.dash-group` sections,
  // `#dash-top-btn`) is unconditional template HTML, so the sticky nav
  // and the collapse chevrons can wire up regardless of whether any
  // widget data loaded above.
  initDashNav();
  initDashCollapse();

