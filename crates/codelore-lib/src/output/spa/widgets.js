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
  // Theme toggle is now an Alpine store registered in template.html
  // (`$store.theme.isDark`). The store's `Alpine.effect` reactively
  // sets `<html data-theme>` AND fires registered re-renderers, so
  // this script doesn't manage the toggle directly anymore.
  // Color-mode toggles for the hotspot circle-pack (cognitive / author / ai).
  initHotspotColorToggles();


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
      formula: 'code_health = 100 × (1 − 0.40 × normalize(cognitive)). Median is the per-file midpoint across the analysed set.',
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
      formula: 'Pairs (a, b) where the two files change in the same commit, gated by min_shared_revs and Fisher exact p < fisher_significance.',
      citation: { label: 'Gall et al. 1998 + Tornhill 2015', anchor: '#coupling-' },
    },
    coupling_density: {
      formula: 'edges / (V·(V−1)/2) where V is the candidate node set (files with revs ≥ min_revs) and edges are Fisher-significant coupling pairs.',
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
      formula: '100 × (1 − 0.40 × normalize(cognitive)). Empirical range [60, 100]; lower = more cognitively complex.',
      citation: { label: 'code-health composite', anchor: '#code-health-' },
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

  renderKpiTiles(data);                                            // → §6

  renderKnowledgeIslands(data.knowledge_islands || []);            // → §7

  let currentHotspotColorMode = 'cognitive';
  renderHotspotCirclePack(data.hotspots || [], currentHotspotColorMode);  // → §8
  // Use registerThemeRerender (not the raw push) because §8 reads
  // theme tokens via the cached `token()` helper (friction heat ramp,
  // health 3-band, top-quartile ring overlay). Flushing the cache
  // before the chart redraws keeps colours in lockstep with the live
  // theme.
  registerThemeRerender(function () {
    renderHotspotCirclePack(data.hotspots || [], currentHotspotColorMode);
  });

  renderHotspotTable(data.hotspots || []);                         // → §9

  renderCouplingSankey(data.coupling || []);                       // → §10
  window._codeloreRerenderers.push(function () {
    renderCouplingSankey(data.coupling || []);
  });

  renderTrends(data.trends || []);                                 // → §11
  window._codeloreRerenderers.push(function () { renderTrends(data.trends || []); });

  renderKameiRiskSparkline(data.kamei_risk || []);                 // → §11b
  window._codeloreRerenderers.push(function () { renderKameiRiskSparkline(data.kamei_risk || []); });

  // Treemap and parallel-coordinates views. Both consume the
  // existing hotspots payload (no new SQL); they're alternative
  // views surfaced as their own sections.
  renderHotspotTreemap(data.hotspots || []);                       // → §11c
  window._codeloreRerenderers.push(function () { renderHotspotTreemap(data.hotspots || []); });
  renderParallelCoords(data.hotspots || []);                       // → §11d
  window._codeloreRerenderers.push(function () { renderParallelCoords(data.hotspots || []); });

  // Cognitive-complexity boxplot, module-to-module chord, and
  // architecture force-graph.
  renderCognitiveBoxplot(data.hotspots || []);                     // → §11e
  window._codeloreRerenderers.push(function () { renderCognitiveBoxplot(data.hotspots || []); });
  renderModuleChord(data.coupling || []);                          // → §11f
  window._codeloreRerenderers.push(function () { renderModuleChord(data.coupling || []); });
  renderArchGraph(data.imports || []);                             // → §11g
  window._codeloreRerenderers.push(function () { renderArchGraph(data.imports || []); });

  renderCalendarHeatmap(data.daily_commits || []);                 // → §12
  window._codeloreRerenderers.push(function () { renderCalendarHeatmap(data.daily_commits || []); });

  renderXRaySunburst(data.xray || []);                             // → §13
  window._codeloreRerenderers.push(function () { renderXRaySunburst(data.xray || []); });

  // Expose the drawer-show callback so the hotspot-table row-click
  // handler can fire it. Must execute after `data` is loaded (above);
  // order vs the renderXxx() calls is immaterial because this is
  // invoked at user-click time.
  window._codeloreShowDetail = function (path) { showFileDetailDrawer(path, data); };

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
  function buildTooltipHtml(defKey) {
    const def = METRIC_DEFS[defKey];
    if (!def) return '';
    const citationHref = RESEARCH_FOUNDATIONS_URL + (def.citation.anchor || '');
    return '<span class="tooltip-host">' +
      '<button type="button" class="tooltip-trigger" aria-label="What does this metric mean?" tabindex="0">?</button>' +
      '<span class="tooltip-popup" role="tooltip">' +
        '<strong>Formula</strong>' +
        '<div class="tooltip-formula">' + escapeHtml(def.formula) + '</div>' +
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
  function startViewTransition(updateFn) {
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
    document.startViewTransition(updateFn);
  }


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

  function showFileDetailDrawer(path, d) {
    const drawer = document.getElementById('file-detail-drawer');
    const title = document.getElementById('drawer-title');
    const body = document.getElementById('drawer-body');
    if (!drawer || !title || !body) return;
    title.textContent = path;

    var html = '';

    // Section: hotspot row
    const hot = (d.hotspots || []).find(function (r) { return r.path === path; });
    if (hot) {
      html += '<h4>Hotspot</h4><dl>' +
        '<dt>Revisions</dt><dd>' + fmtInt(hot.revisions) + '</dd>' +
        '<dt>Cognitive</dt><dd>' + fmtNumberFlex(hot.cognitive, 0) + '</dd>' +
        '<dt>Code health</dt><dd>' + fmtNumberFlex(hot.code_health, 1) + '</dd>' +
        '<dt>Hotspot score</dt><dd>' + fmtNumberFlex(hot.hotspot_score, 2) + '</dd>' +
        '</dl>';
    }

    // Section: knowledge island
    const ki = (d.knowledge_islands || []).find(function (r) { return r.path === path; });
    if (ki) {
      html += '<h4>Knowledge island</h4><dl>' +
        '<dt>Primary author</dt><dd>' + escapeHtml(ki.main_author || '') + '</dd>' +
        '<dt>Ownership</dt><dd>' + fmtNumberFlex(ki.ownership_pct, 1) + ' %</dd>' +
        '<dt>Days since active</dt><dd>' + fmtInt(ki.days_since_main_active) + '</dd>' +
        '<dt>Total LoC</dt><dd>' + fmtInt(ki.total_loc) + '</dd>' +
        '</dl>';
    }

    // Section: coupling partners
    const partners = (d.coupling || []).filter(function (r) {
      return r.entity_a === path || r.entity_b === path;
    });
    if (partners.length) {
      html += '<h4>Coupling partners</h4><ul>';
      for (var i = 0; i < Math.min(partners.length, 20); i++) {
        const p = partners[i];
        const other = (p.entity_a === path) ? p.entity_b : p.entity_a;
        html += '<li><code>' + escapeHtml(other) + '</code>' +
          ' — ' + fmtInt(p.shared_revs) + ' shared revs' +
          (p.combined_score != null ? (' (score ' + fmtNumberFlex(p.combined_score, 2) + ')') : '') +
          '</li>';
      }
      if (partners.length > 20) {
        html += '<li>… ' + (partners.length - 20) + ' more</li>';
      }
      html += '</ul>';
    }

    // Section: code health
    const ch = (d.code_health || []).find(function (r) { return r.path === path; });
    if (ch) {
      html += '<h4>Code health</h4><dl>' +
        '<dt>Score</dt><dd>' + fmtNumberFlex(ch.score, 1) + '</dd>' +
        '<dt>Cognitive</dt><dd>' + fmtNumberFlex(ch.cognitive, 0) + '</dd>' +
        '</dl>';
    }

    if (!html) {
      html = '<div class="empty">No additional details for this path. ' +
        'The path may have been filtered out by minimum-revision thresholds, ' +
        'or its row type is not yet wired into the dashboard.</div>';
    }

    // Radar chart at the top of the drawer. Six axes profile the
    // file across CodeLore's primary metrics: cognitive / churn /
    // age / coupling / MI / AI%. Each axis is normalised to [0, 1]
    // against the run's max so a small repo doesn't show every file
    // at 0 — the radar is comparative within the current dataset.
    html = '<div id="drawer-radar" style="height: 220px; margin-bottom: 14px;"></div>' + html;

    body.innerHTML = html;

    // Render the radar after body.innerHTML so the container exists.
    renderDrawerRadar(path, d);

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
      container.innerHTML = '<div class="opacity-60 text-xs">No metrics for this file.</div>';
      return;
    }
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
      html += '<tr data-path="' + escapeHtml(r.path) + '" class="ki-row">' +
        '<td class="path">' + escapeHtml(r.path) + '</td>' +
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
        showFileDetailDrawer(path, data);
      });
      trs[j].style.cursor = 'pointer';
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
    colorMode = colorMode || 'cognitive';
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

    // Step 3: feed the laid-out nodes into ECharts as a custom series.
    // The custom series renders one shape per node; we draw circles
    // sized + positioned exactly per d3's layout. Color encodes
    // cognitive complexity (leaves only) on a yellow→red ramp.
    const chart = mountEcharts(container);
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
          return '<b>' + escapeHtml(d.fullPath) + '</b>' +
            '<br/>revisions: ' + m.revisions +
            '<br/>cognitive: ' + m.cognitive.toFixed(0) +
            '<br/>code health: ' + m.code_health.toFixed(1) +
            '<br/>hotspot score: ' + m.hotspot_score.toFixed(2);
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
          const innerCircle = {
            type: 'circle',
            shape: {
              cx: datum.x,
              cy: datum.y,
              r: datum.r,
            },
            style: api.style({
              fill: datum.color,
              stroke: datum.stroke,
              lineWidth: 1,
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

    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.fullPath && d.metrics) {
        // Clicking a leaf surfaces its coupling partners AND
        // opens the drawer. Both behaviours co-exist intentionally
        // — the arcs scaffold the drawer's narrative ("this file is
        // tightly coupled to these others") rather than competing
        // with it.
        selectedCouplingFile = d.fullPath;
        updateCouplingArcs();
        showFileDetailDrawer(d.fullPath, data);
      }
    });

    // Clicking the canvas background (no shape under the
    // pointer) clears the arc overlay. `e.target` is falsy for
    // background clicks in zrender's event model.
    chart.getZr().on('click', function (e) {
      if (!e.target) {
        selectedCouplingFile = null;
        updateCouplingArcs();
      }
    });

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

    function renderNextPage(count) {
      const tbody = container.querySelector('#hotspot-tbody');
      if (!tbody) return;
      const next = Math.min(renderedRows + count, filteredView.length);
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
        html += '<tr data-path="' + escapeHtml(r.path) + '" class="hotspot-row" style="cursor:pointer">' +
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
      }
      refreshActions();
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
      showNext.addEventListener('click', function () { renderNextPage(PAGE_SIZE); });
      actionsEl.appendChild(showNext);
      if (more > PAGE_SIZE) {
        const showAll = document.createElement('button');
        showAll.type = 'button';
        showAll.className = 'btn btn-outline btn-sm';
        showAll.textContent = 'Show all (' + more + ' more)';
        showAll.addEventListener('click', function () { renderNextPage(Infinity); });
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

    // Top-N to keep the sankey legible. Sort by combined_score desc.
    const TOP_N = 30;
    const topRows = rows.slice()
      .sort(function (a, b) {
        const ca = (typeof a.combined_score === 'number') ? a.combined_score : 0;
        const cb = (typeof b.combined_score === 'number') ? b.combined_score : 0;
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
        value: r.shared_revs || 1,
      };
    });
    const nodes = Array.from(nodeNames).map(function (name) {
      return { name: name };
    });

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
        label: { color: '#e6e6e6', fontSize: 11 },
      }],
    });

    chart.on('click', function (params) {
      if (params.dataType === 'node' && params.data && params.data.name) {
        showFileDetailDrawer(params.data.name, data);
      }
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

    // Build {month -> {path -> score}} and a sorted month list.
    const months = Array.from(new Set(rows.map(function (r) { return r.month; }))).sort();
    const pathSet = new Set(rows.map(function (r) { return r.path; }));
    const paths = Array.from(pathSet);
    const byMonth = {};
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      if (!byMonth[r.month]) byMonth[r.month] = {};
      byMonth[r.month][r.path] = r.hotspot_score;
    }
    // One series per path
    const series = paths.map(function (p) {
      return {
        name: p,
        type: 'line',
        smooth: true,
        symbol: 'circle',
        symbolSize: 5,
        emphasis: { focus: 'series' },
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
    const legendData = paths.map(function (p) { return shortPath(p); });
    const longByShort = {};
    paths.forEach(function (p) { longByShort[shortPath(p)] = p; });

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
      legend: {
        top: 0,
        type: 'scroll',
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        data: legendData,
      },
      grid: { top: 40, left: 50, right: 20, bottom: 40 },
      xAxis: {
        type: 'category',
        data: months,
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        axisLine: { lineStyle: { color: getCssVar('--border') } },
      },
      yAxis: {
        type: 'value',
        name: 'revisions / month',
        nameTextStyle: { color: getCssVar('--fg-dim'), fontSize: 11 },
        axisLabel: { color: getCssVar('--fg-dim'), fontSize: 11 },
        splitLine: { lineStyle: { color: getCssVar('--bg-elev-2') } },
      },
      series: series.map(function (s) {
        return Object.assign({}, s, { name: shortPath(s.name) });
      }),
    });
  }


  // ─── §11b Widget: Kamei Delivery-Risk Sparkline ──────────────────

  function renderKameiRiskSparkline(rows) {
    const container = document.getElementById('widget-kamei-risk-body');
    if (!container) return;
    if (!rows.length) {
      container.innerHTML = '<div class="empty">No Kamei JIT-SDP data — repo too small or kamei::enrich was not wired.</div>';
      return;
    }

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

    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: {
        trigger: 'item',
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
        // Same emphasis/blur protocol as the parallel-coords widget:
        // hovered bar stays full-opacity, others dim. Without explicit
        // config ECharts 6 applies its default fade that washes the
        // hovered bar out alongside the rest.
        emphasis: { focus: 'self', itemStyle: { opacity: 1 } },
        blur: { itemStyle: { opacity: 0.25 } },
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
        breadcrumb: { show: false },
        label: { show: true, color: '#fff', fontSize: 11 },
        upperLabel: { show: true, height: 18, color: getCssVar('--fg-dim'), fontSize: 11 },
        levels: [
          { itemStyle: { borderColor: getCssVar('--border'), borderWidth: 2, gapWidth: 2 } },
          { itemStyle: { borderColor: getCssVar('--border'), borderWidth: 1, gapWidth: 1 } },
        ],
      }],
    });
    chart.on('click', function (params) {
      const d = params && params.data;
      if (d && d.cognitive != null) {
        showFileDetailDrawer(d.name, data);
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
    // Top-20 by hotspot_score so the polyline soup stays readable.
    const TOP_N = 20;
    const top = rows.slice()
      .sort(function (a, b) {
        const sa = (typeof a.hotspot_score === 'number') ? a.hotspot_score : -Infinity;
        const sb = (typeof b.hotspot_score === 'number') ? b.hotspot_score : -Infinity;
        return sb - sa;
      })
      .slice(0, TOP_N);
    const chart = mountEcharts(container);
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
        // `focus: 'self'` keeps the hovered line opaque while the
        // explicit `blur` config dims the rest. Without these, ECharts
        // 6's default emphasis behaviour leaves the hovered polyline
        // visually identical to its neighbours (or worse, the hovered
        // line gets re-painted underneath the dim batch and appears to
        // vanish entirely).
        emphasis: { focus: 'self', lineStyle: { width: 3, opacity: 1.0 } },
        blur: { lineStyle: { opacity: 0.15 } },
        data: top.map(function (r) {
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
        }),
      }],
    });
    chart.on('click', function (params) {
      if (params && params.name) {
        showFileDetailDrawer(params.name, data);
      }
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
    // Roll up each pair to module-level using the first two path
    // segments (e.g. `app/services/x.py` → `app/services`). One
    // segment is too coarse for repos that put everything under a
    // single root like `app/` or `src/` — every edge collapses to
    // `app→app` and the chord empties out.
    function modulePath(p) {
      const parts = (p || '').split('/');
      if (parts.length <= 1) return p || '';
      return parts.slice(0, 2).join('/');
    }
    const edges = {};
    for (var i = 0; i < rows.length; i++) {
      const r = rows[i];
      const a = modulePath(r.entity_a);
      const b = modulePath(r.entity_b);
      if (!a || !b || a === b) continue;
      const key = a < b ? a + '\x00' + b : b + '\x00' + a;
      edges[key] = (edges[key] || 0) + (r.shared || 1);
    }
    const linkRows = Object.keys(edges).map(function (k) {
      const parts = k.split('\x00');
      return { source: parts[0], target: parts[1], value: edges[k] };
    });
    if (!linkRows.length) {
      container.innerHTML = '<div class="empty">All coupling is intra-module — no cross-module edges to chord.</div>';
      return;
    }
    const nodes = {};
    for (var ei = 0; ei < linkRows.length; ei++) {
      nodes[linkRows[ei].source] = true;
      nodes[linkRows[ei].target] = true;
    }
    const nodeArr = Object.keys(nodes).sort().map(function (n) { return { name: n }; });
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: { trigger: 'item' },
      series: [{
        type: 'graph',
        layout: 'circular',
        circular: { rotateLabel: true },
        data: nodeArr,
        links: linkRows,
        roam: false,
        label: { show: true, color: getCssVar('--fg-dim'), fontSize: 10, position: 'right' },
        lineStyle: {
          color: 'source',
          opacity: 0.55,
          curveness: 0.3,
        },
        emphasis: { focus: 'adjacency', lineStyle: { width: 3 } },
      }],
    });
  }


  // ─── §11g Widget: Architecture force-graph ───────────────────────

  function renderArchGraph(imports) {
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
    function modulePath(p, depth) {
      const parts = (p || '').split('/');
      if (parts.length <= depth) {
        // Use parent dir if file has fewer than `depth` segments;
        // avoids collapsing leaf-level imports onto themselves.
        const lastSlash = (p || '').lastIndexOf('/');
        return lastSlash < 0 ? (p || '') : p.slice(0, lastSlash);
      }
      return parts.slice(0, depth).join('/');
    }
    var edges = {};
    var nodes = {};
    for (var depth = 2; depth <= 6; depth++) {
      edges = {};
      nodes = {};
      for (var i = 0; i < imports.length; i++) {
        const imp = imports[i];
        if (!imp.target_path) continue;
        const s = modulePath(imp.src_path, depth);
        const t = modulePath(imp.target_path, depth);
        if (!s || !t || s === t) continue;
        const key = s + '\x00' + t;
        edges[key] = (edges[key] || 0) + 1;
        nodes[s] = true;
        nodes[t] = true;
      }
      if (Object.keys(edges).length > 0) break;
    }
    const nodeArr = Object.keys(nodes).map(function (n) { return { name: n, symbolSize: 30 }; });
    const edgeArr = Object.keys(edges).map(function (k) {
      const parts = k.split('\x00');
      return { source: parts[0], target: parts[1], value: edges[k] };
    });
    if (!nodeArr.length) {
      container.innerHTML = '<div class="empty">All resolved imports stay intra-module — no inter-module edges to graph.</div>';
      return;
    }
    const chart = mountEcharts(container);
    chart.setOption({
      tooltip: { trigger: 'item' },
      series: [{
        type: 'graph',
        layout: 'force',
        data: nodeArr,
        links: edgeArr,
        roam: true,
        force: { repulsion: 200, edgeLength: 80 },
        label: { show: true, color: getCssVar('--fg-dim'), fontSize: 11 },
        lineStyle: { color: token('--color-info'), opacity: 0.6, width: 1.5 },
        emphasis: { focus: 'adjacency', lineStyle: { width: 3 } },
      }],
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
    const counts = rows.map(function (r) { return r.count; });
    const minVal = Math.min.apply(null, counts);
    const maxVal = Math.max.apply(null, counts);

    // Determine which years to render — one calendar block per year
    // present in the data. Many heatmaps cap at one year; we want
    // multi-year history visible.
    const years = Array.from(new Set(rows.map(function (r) { return r.date.slice(0, 4); }))).sort();
    const calendars = years.map(function (y, idx) {
      return {
        range: y,
        top: 30 + idx * 110,
        cellSize: ['auto', 13],
        left: 70,
        right: 20,
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
      visualMap: {
        min: minVal,
        max: maxVal,
        type: 'piecewise',
        orient: 'horizontal',
        left: 'center',
        top: 0,
        textStyle: { color: getCssVar('--fg-dim'), fontSize: 10 },
        inRange: { color: ['#1a4a2c', '#2ea44f', '#7dd87a', '#f59e0b', '#e0584e'] },
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
        showFileDetailDrawer(d.fullPath, data);
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
        for (var j = 0; j < buttons.length; j++) {
          const isCurrent = (buttons[j] === evt.currentTarget);
          buttons[j].classList.toggle('tab-active', isCurrent);
          buttons[j].classList.toggle('active', isCurrent);
        }
        // Wrap the re-render in startViewTransition so the colour-
        // mode swap smoothly crossfades. On unsupported
        // browsers the wrapper is a synchronous no-op and the
        // re-render runs identically. Updating `currentHotspotColorMode`
        // inside the callback ensures the theme-toggle re-render
        // path sees the active mode too.
        startViewTransition(function () {
          currentHotspotColorMode = mode;
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
    lastHotspotChart.setOption({
      series: [
        {},
        { data: arcData },
      ],
    });
  }

  // Stable palette assignment for author colors. A discrete categorical
  // palette tuned for dark-background readability; cycles if there are
  // more authors than colors.
  function makeAuthorPalette(authors) {
    const palette = [
      '#5fa472', '#2ea44f', '#7dd87a', '#f59e0b', '#e0584e',
      '#c47ddb', '#8ab4ff', '#5bcdd5', '#d4953b', '#a8a8a8',
      '#b53935', '#3d7d4f', '#c97600', '#6a6aef', '#ce62a6',
    ];
    const sorted = authors.slice().sort();
    const out = {};
    for (var i = 0; i < sorted.length; i++) {
      out[sorted[i]] = palette[i % palette.length];
    }
    return out;
  }
})();
